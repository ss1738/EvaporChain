#![no_main]
//! Phase 6.5 of LAMBDA_FOLD_NOVA_PLAN — Nova verify-path DoS fuzz.
//!
//! Light clients hold a preprocessed `vk_bytes` and call
//! `verify_with_vk_bytes` on every proof they receive. A malicious
//! prover (or a corrupted relay) could feed garbage bytes; the
//! verifier MUST surface this as a clean error rather than panic.
//! Any panic caught by libFuzzer is a soundness violation:
//!
//!   - panic = crash = DoS
//!   - Light clients deployed in user wallets / mobile apps cannot
//!     tolerate a panic on input from the network.
//!
//! Run via:
//!
//!   cd fuzz/
//!   cargo +nightly fuzz run fuzz_nova_verify -- -max_total_time=300

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Need at least a few bytes to have anything to split.
    if data.len() < 16 {
        return;
    }

    // Split point chosen from the input itself — gives the fuzzer
    // freedom to put the boundary anywhere. min(1) so split is at
    // least 1.
    let split = ((data[0] as usize).wrapping_mul(7) % data.len()).max(1);
    let proof_bytes = data[..split].to_vec();
    let vk_bytes = &data[split..];

    // Use one of the input bytes as a synthetic z0_bytes prefix to
    // exercise the z0 deserialize path independently of the proof
    // payload.
    let z0_split = ((data[1] as usize).wrapping_mul(11) % data.len()).max(1);
    let z0_bytes = data[..z0_split].to_vec();

    let proof = evaporchain_proving::CompressedProof {
        proof_bytes,
        num_steps: data[2] as usize,
        z0_bytes,
    };

    // Must not panic for ANY input. A clean Err is acceptable; a
    // bool false is acceptable; only panics fail the fuzz.
    let _ = evaporchain_proving::nova::RealBlockProver::verify_with_vk_bytes(
        &proof,
        proof.num_steps,
        vk_bytes,
    );
});
