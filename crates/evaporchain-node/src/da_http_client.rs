//! HTTP-backed implementation of `evaporchain_da::CellSource`.
//!
//! Closes the second half of punch-list item #2 — the
//! `LightClientSampler<S: CellSource>` abstraction in `evaporchain-da`
//! takes a transport-free trait; this module is the production transport.
//!
//! Talks to any node serving the `/api/da/cell/:block/:row/:col` endpoint
//! (exposed at `evaporchain-node/src/api.rs:get_da_cell_sample`). A pool
//! of peer URLs is provided at construction; the client round-robins
//! across them so a faulty peer's responses get diluted across attempts.
//!
//! Faulty-peer marking is delegated back to a host-supplied closure so
//! the DA crate can stay free of any peer-reputation infrastructure
//! (which doesn't exist as a standalone module yet — see punch-list 2c).
//!
//! # Wire format
//!
//! The endpoint returns JSON with hex-encoded fields:
//! ```text
//!   {
//!     "block": u64, "row": usize, "col": usize,
//!     "cell_data": hex(bytes),
//!     "cell_hash": hex(32),
//!     "row_root": hex(32), "col_root": hex(32),
//!     "data_root": hex(32),
//!     "extended_dim": usize,
//!     "row_proof_siblings": [hex(32), …],
//!     "col_proof_siblings": [hex(32), …]
//!   }
//! ```
//!
//! `cell_data` is the only field that's variable-length; everything else
//! is 32-byte hash output. Decoded into `evaporchain_da::CellProof`.

use std::cell::RefCell;
use std::sync::Mutex;
use std::time::Duration;

use evaporchain_da::commitments::CellProof;
use evaporchain_da::light_client::{CellSource, CellSourceError, PeerFaultReason};

/// Configuration for `HttpCellSource`.
#[derive(Debug, Clone)]
pub struct HttpCellSourceConfig {
    /// Base URLs of peer nodes to fetch cells from. Each must be reachable
    /// at `{base}/api/da/cell/{block}/{row}/{col}`. Round-robin order
    /// applies — list peers in trust-priority order if you have any.
    pub peer_base_urls: Vec<String>,
    /// HTTP request timeout per cell fetch. Default 5s — DA samples are
    /// latency-sensitive, slow peers are useless.
    pub timeout: Duration,
}

impl Default for HttpCellSourceConfig {
    fn default() -> Self {
        Self {
            peer_base_urls: Vec::new(),
            timeout: Duration::from_secs(5),
        }
    }
}

/// HTTP-backed cell source that pulls from a configured set of peer nodes.
///
/// Manual `Debug` impl below — `on_fault` holds `Box<dyn Fn>` which
/// has no `Debug` derive; tests need `Debug` to call
/// `Result::unwrap_err`.
pub struct HttpCellSource {
    client: reqwest::blocking::Client,
    peers: Vec<String>,
    cursor: Mutex<usize>,
    /// Optional callback to forward fault reports to the host's peer
    /// reputation system. Wrapped in RefCell so the trait method (which
    /// takes `&self`) can mutate state if the host wants it to.
    on_fault: Mutex<Option<Box<dyn Fn(&str, PeerFaultReason) + Send + Sync>>>,
    /// Per-process counter of faults reported, mainly for tests + metrics.
    fault_count: RefCell<u64>,
}

impl std::fmt::Debug for HttpCellSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cur = self.cursor.lock().map(|g| *g).unwrap_or(0);
        f.debug_struct("HttpCellSource")
            .field("peers", &self.peers)
            .field("cursor", &cur)
            .field("on_fault_set", &self.on_fault.lock().map(|g| g.is_some()).unwrap_or(false))
            .field("fault_count", &*self.fault_count.borrow())
            .finish()
    }
}

impl HttpCellSource {
    pub fn new(config: HttpCellSourceConfig) -> Result<Self, String> {
        if config.peer_base_urls.is_empty() {
            return Err("HttpCellSource requires at least one peer URL".into());
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| format!("HTTP client init: {e}"))?;
        Ok(Self {
            client,
            peers: config.peer_base_urls,
            cursor: Mutex::new(0),
            on_fault: Mutex::new(None),
            fault_count: RefCell::new(0),
        })
    }

    /// Set a callback to receive `(peer_id, reason)` whenever the
    /// sampler reports a faulty proof. Hosts can wire this to whatever
    /// peer-reputation infrastructure they have (or just log it).
    pub fn set_fault_handler<F>(&self, handler: F)
    where
        F: Fn(&str, PeerFaultReason) + Send + Sync + 'static,
    {
        let mut guard = self.on_fault.lock().expect("on_fault mutex");
        *guard = Some(Box::new(handler));
    }

    /// Number of times `report_faulty` has fired. Useful in tests.
    pub fn fault_count(&self) -> u64 {
        *self.fault_count.borrow()
    }

    fn next_peer(&self) -> String {
        let mut cur = self.cursor.lock().expect("peer cursor mutex");
        let peer = self.peers[*cur % self.peers.len()].clone();
        *cur = cur.wrapping_add(1);
        peer
    }
}

/// JSON response shape for `/api/da/cell/:block/:row/:col`.
#[derive(serde::Deserialize)]
struct CellResponse {
    row: usize,
    col: usize,
    cell_data: String,
    cell_hash: String,
    row_root: String,
    col_root: String,
    data_root: String,
    row_proof_siblings: Vec<String>,
    col_proof_siblings: Vec<String>,
}

fn decode_hash(field: &str, hex_str: &str) -> Result<[u8; 32], CellSourceError> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| CellSourceError::Malformed(format!("{field}: bad hex ({e})")))?;
    if bytes.len() != 32 {
        return Err(CellSourceError::Malformed(format!(
            "{field}: expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn decode_siblings(field: &str, list: &[String]) -> Result<Vec<[u8; 32]>, CellSourceError> {
    list.iter()
        .enumerate()
        .map(|(i, s)| decode_hash(&format!("{field}[{i}]"), s))
        .collect()
}

impl CellSource for HttpCellSource {
    fn fetch_cell(
        &self,
        height: u64,
        row: usize,
        col: usize,
    ) -> Result<(String, CellProof), CellSourceError> {
        let peer = self.next_peer();
        let url = format!(
            "{}/api/da/cell/{}/{}/{}",
            peer.trim_end_matches('/'),
            height,
            row,
            col,
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| CellSourceError::Transport(format!("{peer}: {e}")))?;

        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(CellSourceError::NotFound);
        }
        if !status.is_success() {
            return Err(CellSourceError::Transport(format!(
                "{peer}: HTTP {status}"
            )));
        }
        let body: CellResponse = resp
            .json()
            .map_err(|e| CellSourceError::Malformed(format!("{peer}: bad JSON ({e})")))?;

        // The endpoint echoes coords back; refuse on mismatch — a peer
        // that returns a different cell than was queried is misbehaving.
        if body.row != row || body.col != col {
            return Err(CellSourceError::Malformed(format!(
                "{peer}: returned ({},{}), queried ({row},{col})",
                body.row, body.col
            )));
        }

        let cell_data = hex::decode(&body.cell_data)
            .map_err(|e| CellSourceError::Malformed(format!("{peer}: cell_data hex: {e}")))?;
        let cell_hash = decode_hash("cell_hash", &body.cell_hash)?;
        let row_root = decode_hash("row_root", &body.row_root)?;
        let col_root = decode_hash("col_root", &body.col_root)?;
        let data_root = decode_hash("data_root", &body.data_root)?;
        let row_siblings = decode_siblings("row_proof_siblings", &body.row_proof_siblings)?;
        let col_siblings = decode_siblings("col_proof_siblings", &body.col_proof_siblings)?;

        let proof = CellProof {
            cell_data,
            row,
            col,
            cell_hash,
            row_root,
            col_root,
            row_siblings,
            col_siblings,
            data_root,
        };
        Ok((peer, proof))
    }

    fn report_faulty(&self, peer_id: &str, reason: PeerFaultReason) {
        *self.fault_count.borrow_mut() += 1;
        if let Ok(guard) = self.on_fault.lock() {
            if let Some(ref cb) = *guard {
                cb(peer_id, reason);
            }
        }
        tracing::warn!(
            peer = peer_id,
            reason = ?reason,
            "DA sampler: peer returned faulty cell proof"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_peer_list() {
        let cfg = HttpCellSourceConfig::default();
        let err = HttpCellSource::new(cfg).unwrap_err();
        assert!(err.contains("at least one peer"));
    }

    #[test]
    fn round_robin_advances_cursor() {
        let cfg = HttpCellSourceConfig {
            peer_base_urls: vec!["http://a".into(), "http://b".into(), "http://c".into()],
            ..Default::default()
        };
        let src = HttpCellSource::new(cfg).unwrap();
        let p0 = src.next_peer();
        let p1 = src.next_peer();
        let p2 = src.next_peer();
        let p3 = src.next_peer();
        assert_eq!(p0, "http://a");
        assert_eq!(p1, "http://b");
        assert_eq!(p2, "http://c");
        assert_eq!(p3, "http://a"); // wraps
    }

    #[test]
    fn report_faulty_increments_counter() {
        let cfg = HttpCellSourceConfig {
            peer_base_urls: vec!["http://a".into()],
            ..Default::default()
        };
        let src = HttpCellSource::new(cfg).unwrap();
        assert_eq!(src.fault_count(), 0);
        src.report_faulty("http://a", PeerFaultReason::InvalidProof);
        src.report_faulty("http://a", PeerFaultReason::HashMismatch);
        assert_eq!(src.fault_count(), 2);
    }

    #[test]
    fn report_faulty_invokes_handler() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let cfg = HttpCellSourceConfig {
            peer_base_urls: vec!["http://a".into()],
            ..Default::default()
        };
        let src = HttpCellSource::new(cfg).unwrap();

        let observed = Arc::new(AtomicU32::new(0));
        let cloned = observed.clone();
        src.set_fault_handler(move |_peer, _reason| {
            cloned.fetch_add(1, Ordering::SeqCst);
        });

        src.report_faulty("http://a", PeerFaultReason::Unreachable);
        src.report_faulty("http://a", PeerFaultReason::OutOfRange);
        assert_eq!(observed.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn decode_hash_rejects_wrong_length() {
        let err = decode_hash("test", "ab").unwrap_err();
        match err {
            CellSourceError::Malformed(msg) => assert!(msg.contains("32 bytes")),
            _ => panic!("expected Malformed"),
        }
    }

    #[test]
    fn decode_hash_rejects_bad_hex() {
        let err = decode_hash("test", "zzzz").unwrap_err();
        match err {
            CellSourceError::Malformed(msg) => assert!(msg.contains("bad hex")),
            _ => panic!("expected Malformed"),
        }
    }
}
