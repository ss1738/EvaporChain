//! Integration-level Section 2 constants byte-parity test.
//!
//! Runs the same logic as the `check-neptune-parity` binary
//! against a real neptune dump on disk, programmatically. Marked
//! `#[ignore]` so CI without the dump file isn't broken; operators
//! run it via:
//!
//! ```bash
//! cargo run -p evaporchain-nova-bridge --bin dump-neptune-constants -- \
//!     --out /tmp/neptune-bn256-standard.json
//! cargo test -p evaporchain-nova-bridge --test section2_parity -- --ignored
//! ```
//!
//! When `compress_full`, `grain_lfsr`, or the matrix primitives
//! regress, this test fires with a diagnostic count of mismatching
//! crc entries.

use ark_ff::{BigInteger, PrimeField};
use evaporchain_nova_bridge::compress_ark::compress_full;
use evaporchain_nova_bridge::grain_lfsr::generate_round_constants_bn254_arity_24_standard;
use evaporchain_nova_bridge::neptune_dump_parser::{
    extract_compressed_round_constants, extract_mds_inverse_matrix,
};

const DUMP_PATH: &str = "/tmp/neptune-bn256-standard.json";

/// All 259 crc entries of our `compress_full` output must match
/// neptune's `crc` byte-for-byte. Detailed diagnostic on mismatch.
#[test]
#[ignore = "requires /tmp/neptune-bn256-standard.json from dump-neptune-constants"]
fn full_compressed_ark_matches_neptune_crc() {
    let m_inv = extract_mds_inverse_matrix(DUMP_PATH).expect("extract m_inv");
    let neptune_crc =
        extract_compressed_round_constants(DUMP_PATH).expect("extract crc");

    let plain_ark = generate_round_constants_bn254_arity_24_standard();
    let ours = compress_full(&plain_ark, &m_inv, 25, 8, 59);

    assert_eq!(
        ours.len(),
        neptune_crc.len(),
        "length mismatch: ours={} neptune={}",
        ours.len(),
        neptune_crc.len()
    );
    assert_eq!(ours.len(), 259, "expected `(rf+rp)*0 + rf*width + rp = 259`");

    let mut mismatches: Vec<usize> = Vec::new();
    for i in 0..ours.len() {
        if ours[i] != neptune_crc[i] {
            mismatches.push(i);
        }
    }

    if !mismatches.is_empty() {
        // Diagnostic: print the first mismatched LE bytes.
        let i = mismatches[0];
        let ours_le: Vec<u8> = ours[i].into_bigint().to_bytes_le();
        let theirs_le: Vec<u8> = neptune_crc[i].into_bigint().to_bytes_le();
        eprintln!(
            "section2 parity FAILED: {} of {} entries differ",
            mismatches.len(),
            ours.len()
        );
        eprintln!(
            "  first 10 mismatch indices: {:?}",
            &mismatches[..mismatches.len().min(10)]
        );
        eprintln!("  ours[{i}]    LE: {ours_le:?}");
        eprintln!("  theirs[{i}]  LE: {theirs_le:?}");
    }

    assert!(
        mismatches.is_empty(),
        "Section 2 constants byte parity regressed — {} mismatches",
        mismatches.len()
    );
}

/// Same byte-parity check, but read the first three entries
/// (the plain-ARK round 0 trio neptune always emits unchanged)
/// without invoking `compress_full`. Catches a hypothetical
/// future regression in `grain_lfsr` alone.
#[test]
#[ignore = "requires /tmp/neptune-bn256-standard.json"]
fn plain_round_zero_matches_first_three_crc_entries() {
    let neptune_crc =
        extract_compressed_round_constants(DUMP_PATH).expect("extract crc");
    let plain_ark = generate_round_constants_bn254_arity_24_standard();

    for i in 0..3 {
        assert_eq!(
            plain_ark[i], neptune_crc[i],
            "plain ARK[{i}] != neptune crc[{i}]"
        );
    }
}
