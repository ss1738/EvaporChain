//! Persistent IP ban list for libp2p Sybil resistance.
//!
//! Bans are keyed by source [`IpAddr`] (PeerIds are cheap to mint;
//! source IPs are not). Each entry has an expiry timestamp in
//! milliseconds since the unix epoch, and a free-form reason string
//! used for audit / metrics labelling.
//!
//! On-disk format (`<data_dir>/network/bans.json`):
//!
//! ```json
//! {
//!   "version": 1,
//!   "bans": [
//!     {"ip": "192.0.2.1", "until_ms": 1716000000000, "reason": "score_threshold_breach"}
//!   ]
//! }
//! ```
//!
//! Loaded on startup, written on shutdown. Expired entries are pruned
//! both on every `is_banned` call and via the on-disk save path so the
//! file does not accumulate forever.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// One row in the on-disk ban list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanEntry {
    /// Source IP. Both v4 and v6 supported; serialised as the standard
    /// human-readable form ("192.0.2.1" / "2001:db8::1").
    pub ip: IpAddr,
    /// Unix-epoch milliseconds when the ban expires.
    pub until_ms: u64,
    /// Free-form reason ("score_threshold_breach" / "manual" / etc.).
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BanFile {
    version: u32,
    bans: Vec<BanEntry>,
}

/// In-memory ban-list manager. Cheap to clone (it's a `HashMap`) but
/// the typical pattern is to share a single instance behind an
/// `Arc<RwLock<_>>`.
#[derive(Debug, Default, Clone)]
pub struct BanList {
    bans: HashMap<IpAddr, BanEntry>,
}

/// Current wall-clock time in milliseconds. Wraps the `SystemTime` API
/// so callers don't have to handle the unwrap.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl BanList {
    pub fn new() -> Self {
        Self {
            bans: HashMap::new(),
        }
    }

    /// Add or refresh a ban. Replaces any existing entry for `ip` —
    /// the longer expiry wins.
    pub fn add_ban(&mut self, ip: IpAddr, until_ms: u64, reason: impl Into<String>) {
        let entry = self.bans.entry(ip).or_insert(BanEntry {
            ip,
            until_ms,
            reason: reason.into(),
        });
        if until_ms > entry.until_ms {
            entry.until_ms = until_ms;
        }
    }

    /// Remove a ban regardless of expiry. Used by manual unban.
    pub fn remove_ban(&mut self, ip: &IpAddr) -> bool {
        self.bans.remove(ip).is_some()
    }

    /// Returns `true` iff `ip` has an active (un-expired) ban.
    /// Side effect: lazily prunes the entry if the ban has elapsed.
    pub fn is_banned(&mut self, ip: &IpAddr) -> bool {
        let now = now_ms();
        match self.bans.get(ip) {
            Some(entry) if entry.until_ms > now => true,
            Some(_) => {
                self.bans.remove(ip);
                false
            }
            None => false,
        }
    }

    /// Drop every ban whose expiry has elapsed. Cheap to call on a
    /// timer.
    pub fn cleanup_expired(&mut self) -> usize {
        let now = now_ms();
        let before = self.bans.len();
        self.bans.retain(|_, e| e.until_ms > now);
        before - self.bans.len()
    }

    /// All currently-active bans. Useful for the `/api/network/banned`
    /// endpoint and the `evap_active_bans` gauge.
    pub fn active_bans(&self) -> Vec<BanEntry> {
        let now = now_ms();
        self.bans
            .values()
            .filter(|e| e.until_ms > now)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.bans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bans.is_empty()
    }

    /// Default on-disk path under a node's `data_dir`.
    pub fn default_path(data_dir: &Path) -> PathBuf {
        data_dir.join("network").join("bans.json")
    }

    /// Read a ban file from disk. Missing file → empty ban list.
    /// Malformed file → warn + empty (we never refuse to start because
    /// the ban list is corrupt; that would be a self-DoS).
    pub fn load(path: &Path) -> Self {
        let mut bl = BanList::new();
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!("ban list not found at {}; starting empty", path.display());
                return bl;
            }
            Err(e) => {
                warn!("ban list read error at {}: {e}; starting empty", path.display());
                return bl;
            }
        };
        match serde_json::from_slice::<BanFile>(&bytes) {
            Ok(file) => {
                for entry in file.bans {
                    bl.bans.insert(entry.ip, entry);
                }
                bl.cleanup_expired();
                debug!("loaded {} ban(s) from {}", bl.bans.len(), path.display());
            }
            Err(e) => {
                warn!("ban list parse error at {}: {e}; starting empty", path.display());
            }
        }
        bl
    }

    /// Persist to disk. Creates parent directories if needed.
    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        self.cleanup_expired();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = BanFile {
            version: 1,
            bans: self.bans.values().cloned().collect(),
        };
        let json = serde_json::to_vec_pretty(&file)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn add_then_check_banned() {
        let mut bl = BanList::new();
        let target = ip(192, 0, 2, 1);
        bl.add_ban(target, now_ms() + 60_000, "test");
        assert!(bl.is_banned(&target));
        assert!(!bl.is_banned(&ip(192, 0, 2, 2)));
    }

    #[test]
    fn expired_ban_is_pruned() {
        let mut bl = BanList::new();
        let target = ip(192, 0, 2, 3);
        // until_ms in the past
        bl.add_ban(target, now_ms().saturating_sub(1), "expired");
        assert!(!bl.is_banned(&target));
        assert_eq!(bl.len(), 0);
    }

    #[test]
    fn longer_expiry_wins() {
        let mut bl = BanList::new();
        let target = ip(192, 0, 2, 4);
        bl.add_ban(target, now_ms() + 10_000, "first");
        bl.add_ban(target, now_ms() + 60_000, "second");
        let active = bl.active_bans();
        assert_eq!(active.len(), 1);
        assert!(active[0].until_ms >= now_ms() + 30_000);
    }

    #[test]
    fn ban_persists_across_restart() {
        let dir = std::env::temp_dir().join(format!("evap_banlist_{}", now_ms()));
        let path = BanList::default_path(&dir);

        // Round 1: write a ban and save.
        let mut bl = BanList::new();
        let target = ip(198, 51, 100, 7);
        let until = now_ms() + 600_000;
        bl.add_ban(target, until, "test_persist");
        bl.save(&path).expect("save");

        // Round 2: simulate a fresh process — load from disk.
        let mut reloaded = BanList::load(&path);
        assert!(reloaded.is_banned(&target), "ban must survive restart");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_expired_drops_old_entries() {
        let mut bl = BanList::new();
        bl.add_ban(ip(192, 0, 2, 5), now_ms().saturating_sub(1), "old");
        bl.add_ban(ip(192, 0, 2, 6), now_ms() + 60_000, "fresh");
        bl.cleanup_expired();
        assert_eq!(bl.len(), 1);
        assert!(bl.is_banned(&ip(192, 0, 2, 6)));
    }

    #[test]
    fn manual_remove_ban() {
        let mut bl = BanList::new();
        let target = ip(192, 0, 2, 8);
        bl.add_ban(target, now_ms() + 60_000, "test");
        assert!(bl.is_banned(&target));
        assert!(bl.remove_ban(&target));
        assert!(!bl.is_banned(&target));
    }
}
