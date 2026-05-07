//! Higher-level chain-sync helpers built on
//! [`crate::transport::RpcTransport`].
//!
//! The base SDK ([`crate::LightClient::ingest_block`] +
//! [`crate::LightClient::verify_state`]) operates on
//! caller-supplied data — consumers fetch headers/proofs/Nova
//! attestations themselves and pass them in. This module adds
//! sync-loop conveniences that drive the verification forward
//! across many blocks given just a transport.
//!
//! ## Verification semantics
//!
//! Each helper internally calls
//! [`crate::LightClient::ingest_block`] (or the Nova variant)
//! per fetched header, so the same BFT BLS aggregate-sig + Nova-
//! IVC checks apply uniformly. On any verification failure the
//! sync helper returns immediately with the SDK's error; the
//! trusted tip is left at the last successfully-verified block.
//!
//! ## Atomicity
//!
//! Sync helpers are NOT atomic across multiple blocks — partial
//! progress is preserved if a block in the middle of the range
//! fails. Consumers can retry from the new trusted tip. If
//! atomic-or-nothing semantics are required, use
//! [`LightClient::ingest_block`] directly with a save/restore
//! shim around the trusted tip.

use crate::client::LightClient;
use crate::error::LightClientError;
use crate::transport::{transport_err_to_sdk, RpcTransport};

impl LightClient {
    /// Walk the chain forward from the trusted tip to the given
    /// `target` height by fetching each intermediate header and
    /// ingesting it via the BFT-only path. Returns:
    ///   - `Ok(num_blocks_ingested)` on full success.
    ///   - `Err(...)` on the first verification or transport
    ///     failure; the trusted tip will be at the last
    ///     successfully-verified height.
    ///
    /// Walks at most `target - current_height()` blocks. If the
    /// target is at-or-below the trusted tip this is a no-op
    /// (returns `Ok(0)`).
    pub fn sync_to_height<T: RpcTransport>(
        &mut self,
        transport: &T,
        target: u64,
        current_time: u64,
    ) -> Result<u64, LightClientError> {
        let start = self.current_height();
        if target <= start {
            return Ok(0);
        }
        let mut ingested = 0u64;
        for h in (start + 1)..=target {
            let header = transport
                .fetch_header_at(h)
                .map_err(transport_err_to_sdk)?;
            self.ingest_block(header, current_time)?;
            ingested += 1;
        }
        Ok(ingested)
    }

    /// Walk forward to whatever height the node reports as its
    /// latest. Internally:
    ///   1. Fetches the node's latest header (height N).
    ///   2. If N > trusted, walks `(trusted+1)..=N` via
    ///      [`Self::sync_to_height`].
    ///
    /// Returns the height after the sync (i.e., the new trusted
    /// tip's height).
    pub fn sync_to_latest<T: RpcTransport>(
        &mut self,
        transport: &T,
        current_time: u64,
    ) -> Result<u64, LightClientError> {
        let latest = transport
            .fetch_latest_header()
            .map_err(transport_err_to_sdk)?;
        let target = latest.height;
        // Pre-fetch the latest header is informational; the actual
        // walk uses fetch_header_at for each intermediate.
        let _ = self.sync_to_height(transport, target, current_time)?;
        Ok(self.current_height())
    }

    /// One-shot state query: fetch a Verkle proof for `key` from
    /// the transport, then verify it against the trusted state
    /// root. Returns the proof's value (or `None` for non-
    /// membership) on successful verification.
    pub fn fetch_and_verify_state<T: RpcTransport>(
        &self,
        transport: &T,
        key: &[u8; 32],
        expected_value: Option<[u8; 32]>,
    ) -> Result<Option<[u8; 32]>, LightClientError> {
        let proof = transport
            .fetch_state_proof(key)
            .map_err(transport_err_to_sdk)?;
        self.verify_state(&proof, expected_value)?;
        Ok(proof.value)
    }
}

// ───────────────────────── tests ───────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::test_fixtures::*;
    use crate::transport::test_transport::MockTransport;
    use evaporchain_crypto::energy_verkle::EnergyVerkleTrie;

    #[test]
    fn sync_to_height_no_op_when_at_or_below_target() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(5, [0u8; 32], [0xaa; 32], vs, &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis, 100, None);

        let mock = MockTransport::new();
        // Target 3 < trusted 5 → no-op.
        let ingested = lc.sync_to_height(&mock, 3, 110).unwrap();
        assert_eq!(ingested, 0);
        assert_eq!(lc.current_height(), 5);

        // Target 5 == trusted 5 → no-op.
        let ingested = lc.sync_to_height(&mock, 5, 110).unwrap();
        assert_eq!(ingested, 0);
        assert_eq!(lc.current_height(), 5);
    }

    #[test]
    fn sync_to_height_walks_sequential_chain() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis.clone(), 100, None);

        // Build a 4-block chain: heights 2, 3, 4, 5. Each block's
        // parent_hash matches its predecessor's block_hash.
        let mock = MockTransport::new();
        let mut prev_hash = genesis.block_hash;
        for h in 2..=5 {
            let block_hash = [h as u8 + 0xa0; 32];
            let header = make_signed_header(
                h,
                prev_hash,
                block_hash,
                vs.clone(),
                &kps,
                &[0, 1, 2],
            );
            mock.insert_header(header);
            prev_hash = block_hash;
        }

        let ingested = lc.sync_to_height(&mock, 5, 110).unwrap();
        assert_eq!(ingested, 4);
        assert_eq!(lc.current_height(), 5);
        assert_eq!(lc.current_state_root(), [5u8; 32]);
    }

    #[test]
    fn sync_to_latest_walks_to_newest_header() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis.clone(), 100, None);

        let mock = MockTransport::new();
        let mut prev_hash = genesis.block_hash;
        for h in 2..=8 {
            let block_hash = [h as u8 + 0xa0; 32];
            let header = make_signed_header(
                h,
                prev_hash,
                block_hash,
                vs.clone(),
                &kps,
                &[0, 1, 2],
            );
            mock.insert_header(header);
            prev_hash = block_hash;
        }

        let new_height = lc.sync_to_latest(&mock, 110).unwrap();
        assert_eq!(new_height, 8);
        assert_eq!(lc.current_height(), 8);
    }

    #[test]
    fn sync_partial_failure_preserves_trusted_tip_at_last_good() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis.clone(), 100, None);

        let mock = MockTransport::new();
        let mut prev_hash = genesis.block_hash;
        // Block 2: good.
        let h2_hash = [0xb2; 32];
        mock.insert_header(make_signed_header(
            2, prev_hash, h2_hash, vs.clone(), &kps, &[0, 1, 2],
        ));
        prev_hash = h2_hash;
        // Block 3: bad — only 1 of 4 signers (below quorum).
        let h3_hash = [0xb3; 32];
        mock.insert_header(make_signed_header(
            3,
            prev_hash,
            h3_hash,
            vs.clone(),
            &kps,
            &[0],
        ));
        // Block 4 not inserted — sync would stop at 3 anyway.

        let err = lc
            .sync_to_height(&mock, 4, 110)
            .expect_err("sync must fail at block 3");
        assert!(matches!(err, LightClientError::Bft(_)));
        // Block 2 was successfully verified before block 3 failed.
        assert_eq!(lc.current_height(), 2);
    }

    #[test]
    fn sync_returns_transport_error_on_missing_header() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs, &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis, 100, None);

        let mock = MockTransport::new(); // empty — no headers.
        let err = lc
            .sync_to_height(&mock, 5, 110)
            .expect_err("missing header must surface as protocol error");
        assert!(matches!(err, LightClientError::Protocol(_)));
    }

    #[test]
    fn fetch_and_verify_state_round_trip() {
        // Build a Verkle trie and derive the state root that
        // matches a synthetic genesis header.
        let mut trie = EnergyVerkleTrie::new();
        let key = [1u8; 32];
        let value = [42u8; 32];
        trie.insert(key, value, 0, 0, 0);
        let proof = trie.prove(&key);

        // Construct a header whose state_root matches the trie's
        // root.
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let cert = make_commit_certificate(1, 0, [0xaa; 32], &kps, &[0, 1, 2]);
        let mut header = evaporchain_consensus::light_client::LightBlockHeader {
            height: 1,
            epoch: 0,
            block_hash: [0xaa; 32],
            parent_hash: [0u8; 32],
            state_root: trie.root(),
            timestamp: 10,
            validator_set: vs,
            commit_certificate: cert,
        };
        // Make sure the cert's block_hash matches the header's.
        header.commit_certificate.block_hash = header.block_hash;

        let lc = LightClient::new(header, 100, None);

        let mock = MockTransport::new();
        mock.insert_state_proof(key, proof);

        let returned = lc
            .fetch_and_verify_state(&mock, &key, Some(value))
            .expect("state query round-trip must verify");
        assert_eq!(returned, Some(value));
    }

    #[test]
    fn fetch_and_verify_state_missing_proof_surfaces_transport_error() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs, &kps, &[0, 1, 2]);
        let lc = LightClient::new(genesis, 100, None);

        let mock = MockTransport::new();
        let err = lc
            .fetch_and_verify_state(&mock, &[7u8; 32], Some([0u8; 32]))
            .expect_err("missing proof must surface as transport error");
        assert!(matches!(err, LightClientError::Protocol(_)));
    }
}
