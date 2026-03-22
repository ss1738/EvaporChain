/// Trait for digital signature schemes.
pub trait Signer {
    /// Sign a message, returning the signature bytes.
    fn sign(&self, msg: &[u8]) -> Vec<u8>;
    /// Verify a signature against a message.
    fn verify(&self, msg: &[u8], sig: &[u8]) -> bool;
}

/// ML-DSA (formerly Dilithium) post-quantum signature scheme.
pub struct MlDsaSigner;

impl Signer for MlDsaSigner {
    fn sign(&self, _msg: &[u8]) -> Vec<u8> {
        todo!("ML-DSA signing not yet implemented")
    }

    fn verify(&self, _msg: &[u8], _sig: &[u8]) -> bool {
        todo!("ML-DSA verification not yet implemented")
    }
}

/// BLS aggregate signature scheme.
pub struct BlsSigner;

impl Signer for BlsSigner {
    fn sign(&self, _msg: &[u8]) -> Vec<u8> {
        todo!("BLS signing not yet implemented")
    }

    fn verify(&self, _msg: &[u8], _sig: &[u8]) -> bool {
        todo!("BLS verification not yet implemented")
    }
}
