//! §Network — BanList Sybil resistance e2e
//!
//! Scenario: "EvaporChain node Sybil ejection arc" — a node operator
//! OMAR detects three Sybil IPs via score-threshold violations and bans
//! them for 10 minutes. IVAN (one of the Sybils) reconnects from a
//! different IP; OMAR bans that one too. One ban expires naturally;
//! OMAR manually unbans a third after investigation shows it was a false
//! positive. The fourth remains active across a simulated restart (save
//! + load round-trip).
//!
//! The suite proves: fresh bans register; expired bans auto-prune on
//! `is_banned`; re-adding a shorter expiry does not shorten the ban;
//! re-adding a longer expiry extends it; manual unban removes;
//! `cleanup_expired` returns the count dropped; IPv6 addresses work;
//! save + load round-trip preserves active bans; malformed on-disk file
//! does not crash the node.

use evaporchain_network::banlist::{BanList, now_ms};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

fn ipv4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

fn ipv6(suffix: u128) -> IpAddr {
    IpAddr::V6(Ipv6Addr::from(0x2001_0db8_0000_0000_0000_0000_0000_0000u128 | suffix))
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn fresh_ban_registers() {
    let mut bl = BanList::new();
    let ip = ipv4(198, 51, 100, 1);
    bl.add_ban(ip, now_ms() + 600_000, "score_threshold_breach");
    assert!(bl.is_banned(&ip), "freshly banned IP must be detected");
}

#[test]
fn expired_ban_auto_prunes_on_is_banned() {
    let mut bl = BanList::new();
    let ip = ipv4(198, 51, 100, 2);
    bl.add_ban(ip, now_ms().saturating_sub(1), "stale");
    assert!(!bl.is_banned(&ip), "expired ban must return false");
    // Pruned from the map.
    assert_eq!(bl.active_bans().len(), 0);
}

#[test]
fn shorter_expiry_does_not_shorten_ban() {
    let mut bl = BanList::new();
    let ip = ipv4(198, 51, 100, 3);
    let long = now_ms() + 600_000;
    bl.add_ban(ip, long, "initial");
    bl.add_ban(ip, now_ms() + 1_000, "shorter");
    let active = bl.active_bans();
    assert_eq!(active.len(), 1);
    assert!(
        active[0].until_ms >= long,
        "shorter expiry must not shrink the ban: got {} < {long}", active[0].until_ms
    );
}

#[test]
fn longer_expiry_extends_ban() {
    let mut bl = BanList::new();
    let ip = ipv4(198, 51, 100, 4);
    bl.add_ban(ip, now_ms() + 60_000,  "first");
    bl.add_ban(ip, now_ms() + 600_000, "extended");
    let active = bl.active_bans();
    assert!(active[0].until_ms >= now_ms() + 300_000,
        "longer expiry must extend the ban");
}

#[test]
fn manual_unban_removes_entry() {
    let mut bl = BanList::new();
    let ip = ipv4(198, 51, 100, 5);
    bl.add_ban(ip, now_ms() + 600_000, "test");
    assert!(bl.is_banned(&ip));
    assert!(bl.remove_ban(&ip), "first remove must succeed");
    assert!(!bl.is_banned(&ip), "IP must be unbanned");
    assert!(!bl.remove_ban(&ip), "second remove must be no-op");
}

#[test]
fn cleanup_expired_returns_count() {
    let mut bl = BanList::new();
    bl.add_ban(ipv4(192, 0, 2, 10), now_ms().saturating_sub(1), "old1");
    bl.add_ban(ipv4(192, 0, 2, 11), now_ms().saturating_sub(1), "old2");
    bl.add_ban(ipv4(192, 0, 2, 12), now_ms() + 600_000, "fresh");
    let dropped = bl.cleanup_expired();
    assert_eq!(dropped, 2, "two expired entries must be dropped");
    assert_eq!(bl.len(), 1);
    // Idempotent.
    assert_eq!(bl.cleanup_expired(), 0);
}

#[test]
fn active_bans_filters_expired_without_mutation() {
    let mut bl = BanList::new();
    bl.add_ban(ipv4(192, 0, 2, 20), now_ms() + 600_000, "fresh");
    bl.add_ban(ipv4(192, 0, 2, 21), now_ms().saturating_sub(1), "expired");
    // active_bans filters by expiry but does NOT mutate the map.
    let active = bl.active_bans();
    assert_eq!(active.len(), 1);
    assert_eq!(bl.len(), 2, "underlying map unchanged by active_bans");
}

#[test]
fn ipv6_ban_works() {
    let mut bl = BanList::new();
    let ip = ipv6(0x0001);
    bl.add_ban(ip, now_ms() + 600_000, "sybil_v6");
    assert!(bl.is_banned(&ip));
    // IPv4 and IPv6 namespaces are independent.
    assert!(!bl.is_banned(&ipv4(1, 0, 0, 1)));
}

#[test]
fn save_load_round_trip_preserves_active_bans() {
    let dir = std::env::temp_dir().join(format!("evap_net_e2e_{}", now_ms()));
    let path = BanList::default_path(&dir);

    let target = ipv4(203, 0, 113, 7);
    let until = now_ms() + 600_000;

    let mut bl = BanList::new();
    bl.add_ban(target, until, "persist_test");
    bl.save(&path).expect("save must succeed");

    let mut reloaded = BanList::load(&path);
    assert!(reloaded.is_banned(&target), "ban must survive save+load");
    assert_eq!(reloaded.active_bans().len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_missing_file_returns_empty() {
    let bl = BanList::load(std::path::Path::new("/tmp/evap_e2e_missing_bans.json"));
    assert!(bl.is_empty(), "missing file must yield empty ban list");
}

#[test]
fn load_malformed_file_returns_empty() {
    let path = std::env::temp_dir()
        .join(format!("evap_net_malformed_{}.json", now_ms()));
    std::fs::write(&path, b"not valid json!!").unwrap();
    let bl = BanList::load(&path);
    assert!(bl.is_empty(), "malformed file must never crash the node");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn empty_banlist_is_empty() {
    let bl = BanList::new();
    assert!(bl.is_empty());
    assert_eq!(bl.len(), 0);
    assert!(bl.active_bans().is_empty());
}

#[test]
fn omar_sybil_ejection_full_arc() {
    // Full arc: OMAR bans 3 IPs, one expires naturally, one is manually
    // cleared, one persists across a simulated restart.
    let dir = std::env::temp_dir().join(format!("evap_net_arc_{}", now_ms()));
    let path = BanList::default_path(&dir);

    let sybil_a = ipv4(198, 51, 100, 100); // expires naturally (stale)
    let sybil_b = ipv4(198, 51, 100, 101); // false positive, manual unban
    let sybil_c = ipv4(198, 51, 100, 102); // persists

    let mut bl = BanList::new();
    bl.add_ban(sybil_a, now_ms().saturating_sub(1), "score_breach"); // already expired
    bl.add_ban(sybil_b, now_ms() + 600_000, "score_breach");
    bl.add_ban(sybil_c, now_ms() + 600_000, "score_breach");

    // Sybil A: expired — auto-pruned on lookup.
    assert!(!bl.is_banned(&sybil_a), "expired sybil must already be gone");

    // Sybil B: false positive — manual unban.
    assert!(bl.remove_ban(&sybil_b));
    assert!(!bl.is_banned(&sybil_b));

    // Sybil C: persists.
    assert!(bl.is_banned(&sybil_c));

    // Persist and reload.
    bl.save(&path).expect("save");
    let mut reloaded = BanList::load(&path);

    assert!(!reloaded.is_banned(&sybil_a));
    assert!(!reloaded.is_banned(&sybil_b));
    assert!(reloaded.is_banned(&sybil_c), "sybil_c must survive restart");

    let _ = std::fs::remove_dir_all(&dir);
}
