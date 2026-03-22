/// Compute a BLAKE3 hash of the input data.
pub fn blake3_hash(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

/// Compute a Poseidon hash (ZK-friendly hash function).
///
/// Not yet implemented — requires a finite field library.
pub fn poseidon_hash(_data: &[u8]) -> [u8; 32] {
    todo!("Poseidon hash requires field arithmetic implementation")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blake3_hash_deterministic() {
        let data = b"evaporchain";
        let h1 = blake3_hash(data);
        let h2 = blake3_hash(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_blake3_hash_different_inputs() {
        let h1 = blake3_hash(b"hello");
        let h2 = blake3_hash(b"world");
        assert_ne!(h1, h2);
    }
}
