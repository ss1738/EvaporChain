//! T0.7 — Mempool + signature DoS hardening regression suite.
//!
//! Per `MAINNET_READINESS.md` T0.7, this file tracks the in-process
//! DoS-resistance contracts. Each test exercises one DoS vector
//! against the existing admission gates and locks the current
//! defensive behaviour so regressions surface in CI.
//!
//! Vectors (per the spec):
//!   1. Tx flooding (1k, 10k, 100k tx/s)
//!   2. Signature-verification storm (high-volume malformed sigs)
//!   3. Mempool overflow (per-account cap, full mempool)
//!   4. Encrypted mempool reveal flood (TODO — separate test file)
//!   5. Fork-spam (TODO — needs DAG-level harness)
//!   6. Gas exhaustion (covered in execution crate's block_stm tests)
//!   7. Memory blow-up via large blobs (covered by global byte-cap test)
//!
//! What this file ships today:
//!   - Vector 1: tx-flood admission cap (max_size hits before pool grows unbounded)
//!   - Vector 2: malformed-sig storm (each tx rejected, pool stays empty)
//!   - Vector 3: per-account cap (single sender can't monopolise the pool)
//!
//! Future vectors (4, 5) are intentionally omitted from this file
//! pending the encrypted-mempool DoS harness + DAG fork-spam
//! integration test. Vectors 6, 7 are already covered elsewhere.
//!
//! The acceptance criterion in T0.7 ("harness committed to tests/dos/,
//! runs ≥1hr at each load level") is for a CLUSTER-driven harness;
//! this file is the in-process companion that locks the per-validator
//! admission contracts the cluster harness would otherwise re-discover.

use evaporchain_consensus::mempool::Mempool;
use evaporchain_crypto::signatures::{HybridKeypair, Signer};
use evaporchain_types::{Transaction, TransferTx};

fn make_transfer(sender: u8, nonce: u64) -> Transaction {
    let mut from = [0u8; 32];
    from[0] = sender;
    Transaction::Transfer(TransferTx {
        from,
        to: [2u8; 32],
        amount: 100,
        nonce,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    })
}

fn make_transfer_with_garbage_sig(sender: u8, nonce: u64) -> Transaction {
    let mut from = [0u8; 32];
    from[0] = sender;
    // Random-bytes signature + public_key. Length-shape matches what
    // HybridVerifier::verify expects (so we exercise the cryptographic
    // verify path, not a pre-check), but the bytes themselves are
    // garbage and will not verify under any keypair.
    let kp = HybridKeypair::generate();
    let pk = kp.public_key_bytes();
    let garbage_sig = vec![0xAAu8; 1000]; // typical hybrid-sig size; bogus content
    Transaction::Transfer(TransferTx {
        from,
        to: [2u8; 32],
        amount: 100,
        nonce,
        signature: Some(garbage_sig),
        public_key: Some(pk),
        mev_refund_eligible: None,
    })
}

// ─── Vector 1 — Tx flooding (max_size cap) ───────────────────────────

/// 20K-tx flood against a default-sized mempool (10K cap). The cap MUST
/// fire — pool stays at 10K, the remaining 10K are counted as rejected.
/// Locks the per-validator first line of defence: an attacker cannot
/// grow the mempool unbounded by submitting unique-hash txs.
/// Construct a transfer with sender encoded across the first 2 bytes
/// of the address so we can generate >256 distinct senders.
fn make_transfer_2b_sender(sender_id: u32, nonce: u64) -> Transaction {
    let mut from = [0u8; 32];
    from[..4].copy_from_slice(&sender_id.to_le_bytes());
    Transaction::Transfer(TransferTx {
        from,
        to: [2u8; 32],
        amount: 100,
        nonce,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    })
}

#[test]
fn dos_v1_tx_flood_caps_at_max_size() {
    // MAX_MEMPOOL_SIZE = 10_000 (mempool.rs:73). Locked here as a
    // documented constant; if the upstream value changes this test
    // re-tunes via the assertion shape rather than the literal.
    const DOCUMENTED_MAX_MEMPOOL_SIZE: usize = 10_000;
    const DOCUMENTED_MAX_TXS_PER_ACCOUNT: usize = 64;
    let mut pool = Mempool::new();
    let flood_count = DOCUMENTED_MAX_MEMPOOL_SIZE * 2;

    // Spread across enough senders that NEITHER the per-account cap
    // NOR the duplicate cache fires before the global max_size cap:
    // 50 nonces per sender (< 64 per-account cap) × 400 senders =
    // 20_000 unique (sender, nonce) combinations.
    const NONCES_PER_SENDER: u64 = 50;
    assert!(NONCES_PER_SENDER < DOCUMENTED_MAX_TXS_PER_ACCOUNT as u64);

    let mut accepted = 0usize;
    for i in 0..flood_count {
        let sender_id = (i as u64 / NONCES_PER_SENDER) as u32 + 1;
        let nonce = (i as u64) % NONCES_PER_SENDER;
        if pool.submit(make_transfer_2b_sender(sender_id, nonce)) {
            accepted += 1;
        }
    }
    assert_eq!(
        accepted, DOCUMENTED_MAX_MEMPOOL_SIZE,
        "max_size cap MUST stop accepts at exactly cap; got accepted={}",
        accepted
    );
    assert_eq!(
        pool.len(),
        DOCUMENTED_MAX_MEMPOOL_SIZE,
        "pool size MUST stay at cap: cap={}, len={}",
        DOCUMENTED_MAX_MEMPOOL_SIZE,
        pool.len()
    );
    // With unique (sender, nonce) tuples, every overflow tx goes
    // through max_size rejection (not duplicate, not per-account cap).
    let expected_rejections = (flood_count - DOCUMENTED_MAX_MEMPOOL_SIZE) as u64;
    assert_eq!(
        pool.rejected_count(),
        expected_rejections,
        "rejected_count must be exactly flood - cap; flood={}, cap={}, got={}",
        flood_count,
        DOCUMENTED_MAX_MEMPOOL_SIZE,
        pool.rejected_count()
    );
    assert_eq!(
        pool.duplicate_count(),
        0,
        "no duplicates should be generated by unique (sender, nonce) flood"
    );
}

// ─── Vector 2 — Signature-verification storm ─────────────────────────

/// Flood the mempool with garbage-signature transactions (each tx has
/// signature + public_key set to length-correct but cryptographically
/// invalid bytes). With `verify_signatures = true`, every tx must hit
/// HybridVerifier::verify and be rejected. The pool MUST stay empty.
///
/// Today's defence: verify_signatures gate runs synchronously in
/// validate_submission; each malformed tx is rejected immediately
/// after the verify call returns false.
///
/// Failure mode this test catches: any future change that allows a
/// malformed-sig tx to slip past validate_submission and into the pool
/// (e.g. a verify-then-skip optimisation that mistakenly accepts on
/// verifier error).
#[test]
fn dos_v2_signature_storm_pool_stays_empty_under_garbage_sigs() {
    let mut pool = Mempool::new();
    pool.enable_sig_verification();
    let flood_count = 200usize; // CPU-bounded; 200 is enough to lock the contract

    let mut accepted = 0usize;
    for i in 0..flood_count {
        let sender = ((i / 10) % 250) as u8 + 1;
        let nonce = (i % 10) as u64;
        if pool.submit(make_transfer_with_garbage_sig(sender, nonce)) {
            accepted += 1;
        }
    }
    assert_eq!(
        accepted, 0,
        "ZERO garbage-sig txs may enter the pool under verify_signatures=true; \
         got accepted={}",
        accepted
    );
    assert_eq!(
        pool.len(),
        0,
        "pool must remain empty after signature-storm; got len={}",
        pool.len()
    );
    assert_eq!(
        pool.rejected_count(),
        flood_count as u64,
        "every garbage-sig tx must hit rejected_count; got rejected={}",
        pool.rejected_count()
    );
}

// ─── Vector 3 — Per-account cap (single-sender exhaustion) ──────────

/// Single sender floods with 200 unique-nonce txs. The per-account cap
/// MUST kick in well before the global max_size cap, so a single
/// adversary can't monopolise the mempool's slot budget.
///
/// Locks the doctrine: mempool admission is fairness-preserving across
/// senders. This is what stops a single Sybil identity from filling
/// 100% of the 10K-tx capacity.
#[test]
fn dos_v3_single_sender_capped_below_global_max() {
    // Per-account cap (MAX_TXS_PER_ACCOUNT = 64, mempool.rs:88)
    // < global max_size (10_000). A single sender must hit the
    // per-account gate well before exhausting the global slot budget.
    const DOCUMENTED_MAX_TXS_PER_ACCOUNT: usize = 64;
    const DOCUMENTED_MAX_MEMPOOL_SIZE: usize = 10_000;

    let mut pool = Mempool::new();
    let flood_count = 200usize;

    let mut accepted = 0usize;
    for nonce in 0..flood_count {
        if pool.submit(make_transfer(0xAB, nonce as u64)) {
            accepted += 1;
        }
    }
    assert!(
        accepted < DOCUMENTED_MAX_MEMPOOL_SIZE,
        "single-sender accepted count must be capped BELOW global max_size; \
         got accepted={}, global_cap={}",
        accepted,
        DOCUMENTED_MAX_MEMPOOL_SIZE
    );
    assert!(
        accepted <= DOCUMENTED_MAX_TXS_PER_ACCOUNT,
        "per-account cap should be ≤{} (the documented MAX_TXS_PER_ACCOUNT); \
         got accepted={}",
        DOCUMENTED_MAX_TXS_PER_ACCOUNT,
        accepted
    );
    // Sanity: the rest were rejected.
    let expected_rejections = (flood_count - accepted) as u64;
    assert!(
        pool.rejected_count() >= expected_rejections,
        "rejected_count must reflect the cap-driven rejections"
    );
}

// ─── Vector 4 — Encrypted mempool reveal flood ───────────────────────
//
// The EncryptedMempool implements commit-reveal MEV protection
// (see `evaporchain-consensus::encrypted_mempool`). Submission is
// `submit_encrypted(EncryptedTransaction)` which pushes onto a Vec
// with NO admission cap today — no max size, no per-sender cap, no
// global byte cap.
//
// What this file ships for V4:
//   - dos_v4_encrypted_mempool_admission_cap_fires_on_flood: locks
//     the MAX_ENCRYPTED_PENDING = 10_000 admission cap. 15K
//     submissions; first 10K accepted, last 5K rejected, pool stays
//     at exactly cap. (Flipped from earlier GAP-marker test once the
//     substrate fix landed.)
//   - dos_v4_reveal_too_early_rejected: positive test for the
//     temporal gate on reveals — submitting at epoch E and trying
//     to reveal before E + reveal_delay returns RevealTooEarly.
//   - dos_v4_unrevealed_commitments_expire_at_reveal_epoch:
//     process_reveals drops encrypted txs whose reveal epoch has
//     passed without a nonce supplied. Confirms attackers cannot
//     keep stale commitments alive forever in the pool.

use evaporchain_consensus::encrypted_mempool::{
    encrypt_transaction, EncryptedMempool, MevError,
};

fn make_encrypted(nonce_byte: u8, submitted_epoch: u64) -> evaporchain_consensus::encrypted_mempool::EncryptedTransaction {
    let tx = make_transfer(1, nonce_byte as u64);
    let nonce = [nonce_byte; 32];
    encrypt_transaction(&tx, &nonce, submitted_epoch)
}

/// Positive test — the encrypted mempool admission cap fires under
/// flood. MAX_ENCRYPTED_PENDING = 10_000 matches the plaintext
/// pool's MAX_MEMPOOL_SIZE; submit_encrypted returns false when
/// the pool is full, the over-flood submissions are rejected,
/// and the pool size stays at exactly the cap.
///
/// This test was originally `dos_v4_encrypted_mempool_has_no_admission_cap_GAP`,
/// a documented-gap fixture that asserted unbounded growth. The
/// substrate fix landed (parallel session) and the test was flipped
/// to lock the defensive behaviour. The assertion direction is now
/// "cap enforced", not "no cap".
#[test]
fn dos_v4_encrypted_mempool_admission_cap_fires_on_flood() {
    let mut pool = EncryptedMempool::new(2);
    let cap_target: usize = 10_000; // matches MAX_ENCRYPTED_PENDING
    let flood_count: usize = cap_target + 5_000;

    let mut accepted = 0usize;
    let mut rejected = 0usize;
    for i in 0..flood_count {
        let nonce_byte = (i % 256) as u8;
        let enc = make_encrypted(nonce_byte, 0);
        if pool.submit_encrypted(enc) {
            accepted += 1;
        } else {
            rejected += 1;
        }
    }

    assert_eq!(
        accepted, cap_target,
        "encrypted mempool MUST accept exactly MAX_ENCRYPTED_PENDING ({}), got {}",
        cap_target, accepted
    );
    assert_eq!(
        rejected,
        flood_count - cap_target,
        "every over-cap submission MUST be rejected by submit_encrypted",
    );
    let (encrypted_len, _) = pool.pending_count();
    assert_eq!(
        encrypted_len, cap_target,
        "pool size MUST stay at exactly the cap, got {}",
        encrypted_len
    );
}

/// Positive test — reveal before `submitted_epoch + reveal_delay`
/// returns RevealTooEarly. Locks the temporal commit-reveal gate
/// that makes the MEV-protection scheme honest.
#[test]
fn dos_v4_reveal_too_early_rejected() {
    let reveal_delay = 5;
    let pool = EncryptedMempool::new(reveal_delay);

    let submit_epoch = 10;
    let nonce_byte = 7;
    let enc = make_encrypted(nonce_byte, submit_epoch);
    let nonce = [nonce_byte; 32];

    // Same epoch — definitely too early.
    let result = pool.reveal(&enc, &nonce, submit_epoch);
    assert!(matches!(result, Err(MevError::RevealTooEarly { .. })));

    // Mid-delay — still too early.
    let result = pool.reveal(&enc, &nonce, submit_epoch + reveal_delay - 1);
    assert!(matches!(result, Err(MevError::RevealTooEarly { .. })));

    // Exactly at the boundary — must succeed.
    let result = pool.reveal(&enc, &nonce, submit_epoch + reveal_delay);
    assert!(
        result.is_ok(),
        "reveal at reveal_at MUST succeed: got {:?}",
        result.err()
    );

    // Past the boundary — also succeeds.
    let result = pool.reveal(&enc, &nonce, submit_epoch + reveal_delay + 100);
    assert!(result.is_ok());
}

/// Positive test — unrevealed encrypted txs are dropped at their
/// reveal epoch via process_reveals (no nonce supplied → dropped).
/// Prevents attackers from keeping stale commitments alive
/// indefinitely after their reveal window passes.
#[test]
fn dos_v4_unrevealed_commitments_expire_at_reveal_epoch() {
    let reveal_delay = 3;
    let mut pool = EncryptedMempool::new(reveal_delay);

    // Submit 5 encrypted txs at epoch 0.
    for i in 0..5 {
        let nonce_byte = (i + 1) as u8;
        pool.submit_encrypted(make_encrypted(nonce_byte, 0));
    }
    let (encrypted_len, _) = pool.pending_count();
    assert_eq!(encrypted_len, 5, "5 encrypted txs in flight");

    // Advance to reveal_delay - 1; nothing should be processed yet.
    let revealed = pool.process_reveals(reveal_delay - 1, &[]);
    assert!(revealed.is_empty(), "pre-reveal: nothing revealed");
    let (encrypted_len, _) = pool.pending_count();
    assert_eq!(encrypted_len, 5, "all 5 still pending pre-reveal-delay");

    // Advance to reveal_delay (boundary). With NO nonces supplied,
    // all 5 expire (dropped without being revealed).
    let revealed = pool.process_reveals(reveal_delay, &[]);
    assert!(
        revealed.is_empty(),
        "no nonces supplied → no reveals; got {} revealed",
        revealed.len()
    );
    let (encrypted_len, _) = pool.pending_count();
    assert_eq!(
        encrypted_len, 0,
        "all 5 stale encrypted txs MUST be expired at the reveal epoch"
    );
}
