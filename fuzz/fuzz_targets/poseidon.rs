#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = evaporchain_crypto::hash::poseidon_hash(data);
});
