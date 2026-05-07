//! Verkle state-query verification.
//!
//! After [`crate::LightClient::ingest_block`] has trusted a header
//! (and thus a `state_root`), consumers can verify state queries
//! against that root via Verkle Merkle proofs. Wraps
//! [`evaporchain_crypto::verkle::VerkleTrie::verify`].
//!
//! ## Contract
//!
//! Given:
//!   - a [`VerkleProof`] returned by a node (typically via an HTTP
//!     RPC like `/api/state/proof`)
//!   - the expected value (or `None` if the caller is verifying
//!     non-membership)
//!
//! [`crate::LightClient::verify_state`] succeeds iff:
//!   1. The proof verifies against the trusted `state_root`.
//!   2. The proof's value matches `expected_value` (membership)
//!      or both are `None` (non-membership).
//!
//! Either failure surfaces as a [`crate::LightClientError`].

use evaporchain_crypto::verkle::{VerkleProof, VerkleTrie};

use crate::client::LightClient;
use crate::error::LightClientError;

impl LightClient {
    /// Verify a Verkle state-query proof against the trusted
    /// state root from the most-recently-ingested block.
    ///
    /// `expected_value` follows Verkle's `Option<[u8; 32]>`
    /// convention: `Some(v)` for membership proofs, `None` for
    /// non-membership.
    ///
    /// On success, returns `Ok(())`. On failure, returns:
    ///   - [`LightClientError::VerkleProof`] if the proof itself
    ///     does not verify against the trusted root.
    ///   - [`LightClientError::StateValueMismatch`] if the proof
    ///     verifies but the embedded value differs from
    ///     `expected_value`.
    pub fn verify_state(
        &self,
        proof: &VerkleProof,
        expected_value: Option<[u8; 32]>,
    ) -> Result<(), LightClientError> {
        let state_root = self.current_state_root();

        // Step 1: cryptographic verification — the proof binds to
        // the trusted state root.
        if !VerkleTrie::verify(proof, &state_root) {
            return Err(LightClientError::VerkleProof {
                state_root_hex: bytes_to_hex(&state_root),
            });
        }

        // Step 2: value match. The proof's `value` field tells us
        // what the prover claims; we cross-check against the
        // caller's expectation.
        if proof.value != expected_value {
            return Err(LightClientError::StateValueMismatch {
                key_hex: bytes_to_hex(&proof.key),
                expected_hex: expected_value.map(|v| bytes_to_hex(&v)),
                actual_hex: proof.value.map(|v| bytes_to_hex(&v)),
            });
        }

        Ok(())
    }
}

// ───────────────────────── helpers ─────────────────────────────

/// Hex-encode a byte slice in lowercase. Local helper so the SDK
/// doesn't depend on the `hex` crate (keeps WASM-target builds
/// lean).
fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(nibble_hex(b >> 4));
        s.push(nibble_hex(b & 0x0f));
    }
    s
}

fn nibble_hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => '?',
    }
}

// ───────────────────────── tests ───────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_consensus::light_client::LightBlockHeader;
    use evaporchain_consensus::validator_set::ValidatorSet;
    use evaporchain_crypto::verkle::VerkleTrie;
    use evaporchain_types::CommitCertificate;

    /// Build a header anchored at the given state_root for
    /// state-query test setup.
    fn header_with_state_root(state_root: [u8; 32]) -> LightBlockHeader {
        LightBlockHeader {
            height: 0,
            epoch: 0,
            block_hash: [0xaa; 32],
            parent_hash: [0u8; 32],
            state_root,
            timestamp: 1_700_000_000,
            validator_set: ValidatorSet::default(),
            commit_certificate: CommitCertificate {
                height: 0,
                round: 0,
                block_hash: [0xaa; 32],
                aggregate_signature: Vec::new(),
                signer_ids: Vec::new(),
            },
        }
    }

    /// Build a `LightClient` whose trusted state_root matches
    /// the given Verkle trie's root, using a synthetic genesis
    /// header. State-query tests don't drive BFT verification;
    /// they only exercise `verify_state`'s Verkle path.
    fn light_client_for_trie(trie: &VerkleTrie) -> crate::LightClient {
        let header = header_with_state_root(trie.root());
        crate::LightClient::new(
            header,
            1_700_000_000,
            #[cfg(feature = "nova")]
            None,
        )
    }

    #[test]
    fn verify_state_membership_proof_succeeds() {
        let mut trie = VerkleTrie::new();
        let key = [1u8; 32];
        let value = [42u8; 32];
        trie.insert(key, value);
        let proof = trie.prove(&key);

        let lc = light_client_for_trie(&trie);
        lc.verify_state(&proof, Some(value)).expect("membership proof must verify");
    }

    #[test]
    fn verify_state_value_mismatch_rejected() {
        let mut trie = VerkleTrie::new();
        let key = [1u8; 32];
        let stored_value = [42u8; 32];
        let wrong_expected = [43u8; 32];
        trie.insert(key, stored_value);
        let proof = trie.prove(&key);

        let lc = light_client_for_trie(&trie);
        let err = lc
            .verify_state(&proof, Some(wrong_expected))
            .expect_err("value mismatch must be rejected");
        assert!(matches!(err, LightClientError::StateValueMismatch { .. }));
    }

    #[test]
    fn verify_state_mismatched_state_root_rejected() {
        let mut trie = VerkleTrie::new();
        let key = [1u8; 32];
        let value = [42u8; 32];
        trie.insert(key, value);
        let proof = trie.prove(&key);

        // Build the light client against a DIFFERENT (wrong)
        // state root.
        let header = header_with_state_root([0xff; 32]);
        let lc = crate::LightClient::new(
            header,
            1_700_000_000,
            #[cfg(feature = "nova")]
            None,
        );

        let err = lc
            .verify_state(&proof, Some(value))
            .expect_err("wrong-root proof must be rejected");
        assert!(matches!(err, LightClientError::VerkleProof { .. }));
    }

    #[test]
    fn verify_state_tampered_proof_rejected() {
        let mut trie = VerkleTrie::new();
        let key = [1u8; 32];
        let value = [42u8; 32];
        trie.insert(key, value);
        let mut proof = trie.prove(&key);

        // Tamper with the proof's value (claim the trie maps key
        // to a different value than it does).
        proof.value = Some([99u8; 32]);

        let lc = light_client_for_trie(&trie);
        // Must be rejected — either because the cryptographic
        // verification fails (the tampered value produces a
        // different leaf hash, breaking the path) OR because the
        // value mismatch surfaces. Either error variant is correct.
        let err = lc
            .verify_state(&proof, Some([99u8; 32]))
            .expect_err("tampered-value proof must be rejected");
        assert!(matches!(
            err,
            LightClientError::VerkleProof { .. }
                | LightClientError::StateValueMismatch { .. }
        ));
    }

    #[test]
    fn verify_state_membership_value_mismatch_via_some_vs_none() {
        // Caller supplies `None` (non-membership expectation) but
        // proof claims membership with a value. Value-mismatch
        // path surfaces the error.
        let mut trie = VerkleTrie::new();
        let key = [1u8; 32];
        let value = [42u8; 32];
        trie.insert(key, value);
        let proof = trie.prove(&key); // proof.value = Some(value)

        let lc = light_client_for_trie(&trie);
        let err = lc
            .verify_state(&proof, None)
            .expect_err("Some-vs-None mismatch must be rejected");
        assert!(matches!(err, LightClientError::StateValueMismatch { .. }));
    }

    #[test]
    fn bytes_to_hex_round_trip() {
        let bytes: [u8; 8] = [0x00, 0x12, 0xab, 0xff, 0x7f, 0x80, 0x01, 0x10];
        assert_eq!(bytes_to_hex(&bytes), "0012abff7f800110");
    }
}
