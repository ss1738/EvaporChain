//! §DA — Reed-Solomon erasure coding e2e
//!
//! Scenario: "EvaporChain DA sampling round" — a block producer
//! YUSUF encodes a 1 KiB block payload into 4 data + 4 parity shards.
//! Four light clients each hold one shard. Any 4-of-8 must reconstruct
//! the original payload. IVAN (adversarial) tampers with one shard;
//! the hash check catches him immediately.
//!
//! The suite proves: any minimal quorum of data shards reconstructs;
//! parity shards alone also reconstruct; tampering is detectable;
//! empty payloads and insufficient quorums fail closed.

use evaporchain_da::{
    ErasureEncoder,
    erasure::{ErasureConfig, EncodedData},
};

// ── Helpers ───────────────────────────────────────────────────────────────

fn yusuf_payload() -> Vec<u8> {
    // 1 KiB block payload: sequential byte values.
    (0u32..1024).map(|i| i as u8).collect()
}

fn encoder_4_4() -> ErasureEncoder {
    ErasureEncoder::new(ErasureConfig { data_shards: 4, parity_shards: 4 }).unwrap()
}

fn encode_payload() -> EncodedData {
    encoder_4_4().encode(&yusuf_payload()).unwrap()
}

/// Build the subset vec with exactly the given shard indices present.
fn subset_from(encoded: &EncodedData, indices: &[usize]) -> Vec<Option<Vec<u8>>> {
    (0..encoded.shards.len())
        .map(|i| {
            if indices.contains(&i) {
                Some(encoded.shards[i].data.clone())
            } else {
                None
            }
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn encode_produces_eight_shards() {
    let encoded = encode_payload();
    assert_eq!(encoded.shards.len(), 8, "4 data + 4 parity must = 8 shards");
}

#[test]
fn all_shards_pass_hash_verification() {
    // Each shard's embedded hash must match its data at encode time.
    for (i, shard) in encode_payload().shards.iter().enumerate() {
        assert!(ErasureEncoder::verify_shard(shard),
            "shard {i} must verify immediately after encoding");
    }
}

#[test]
fn data_shards_only_reconstruct_exactly() {
    // Light clients holding only the 4 data shards (0,1,2,3) reconstruct.
    let encoded = encode_payload();
    let payload = yusuf_payload();
    let subset = subset_from(&encoded, &[0, 1, 2, 3]);
    let out = encoder_4_4().reconstruct(subset).unwrap();
    assert_eq!(&out[..payload.len()], &payload[..],
        "4 data shards must reconstruct the original payload byte-for-byte");
}

#[test]
fn parity_shards_only_reconstruct_exactly() {
    // Light clients holding only the 4 parity shards (4,5,6,7) reconstruct.
    let encoded = encode_payload();
    let payload = yusuf_payload();
    let subset = subset_from(&encoded, &[4, 5, 6, 7]);
    let out = encoder_4_4().reconstruct(subset).unwrap();
    assert_eq!(&out[..payload.len()], &payload[..],
        "4 parity shards must also reconstruct");
}

#[test]
fn mixed_quorum_reconstructs() {
    // Any 4-of-8 (e.g. shards 1, 3, 5, 7) must reconstruct.
    let encoded = encode_payload();
    let payload = yusuf_payload();
    let subset = subset_from(&encoded, &[1, 3, 5, 7]);
    let out = encoder_4_4().reconstruct(subset).unwrap();
    assert_eq!(&out[..payload.len()], &payload[..],
        "mixed 4-of-8 quorum must reconstruct");
}

#[test]
fn tampered_shard_fails_hash_verification() {
    // IVAN flips one byte in shard 0 — verify_shard must catch it.
    let mut encoded = encode_payload();
    encoded.shards[0].data[0] ^= 0xFF;
    assert!(!ErasureEncoder::verify_shard(&encoded.shards[0]),
        "tampered shard must fail hash verification");
    // Other shards are still clean.
    assert!(ErasureEncoder::verify_shard(&encoded.shards[1]));
}

#[test]
fn insufficient_shards_fail_closed() {
    // Only 3 of 4 required data shards → reconstruct must fail.
    let encoded = encode_payload();
    let subset = subset_from(&encoded, &[0, 1, 2]); // only 3
    assert!(encoder_4_4().reconstruct(subset).is_err(),
        "3-of-8 must fail closed (need 4)");
}

#[test]
fn empty_payload_fails_closed() {
    // Empty input must return a typed error, not silent empty encoding.
    assert!(encoder_4_4().encode(&[]).is_err(),
        "empty payload must be rejected");
}

#[test]
fn different_payloads_produce_different_shards() {
    let enc = encoder_4_4();
    let p1: Vec<u8> = vec![0xAA; 256];
    let p2: Vec<u8> = vec![0xBB; 256];
    let e1 = enc.encode(&p1).unwrap();
    let e2 = enc.encode(&p2).unwrap();
    assert_ne!(e1.shards[0].data, e2.shards[0].data,
        "different payloads must produce different shard content");
}

#[test]
fn yusuf_da_round_full_arc() {
    // Full arc: YUSUF encodes a 1 KiB block, distributes to 8 light
    // clients. 4 clients are offline (drop shards 0,2,4,6). The
    // remaining 4 (shards 1,3,5,7) reconstruct exactly.
    let payload = yusuf_payload();
    let encoded = encode_payload();

    // Verify all shards are clean before distribution.
    for s in &encoded.shards {
        assert!(ErasureEncoder::verify_shard(s));
    }

    // Reconstruct from 4 online clients.
    let subset = subset_from(&encoded, &[1, 3, 5, 7]);
    let out = encoder_4_4().reconstruct(subset).unwrap();
    assert_eq!(&out[..payload.len()], &payload[..],
        "DA round must reconstruct even with 4 offline clients");
}
