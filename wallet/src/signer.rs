//! Transaction signing with ML-DSA keys.
//!
//! The `WalletSigner` wraps a decrypted ML-DSA keypair and provides
//! methods to sign any EvaporChain transaction type.

use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};
use evaporchain_types::{AccountAddress, Transaction};
use thiserror::Error;

use crate::address::derive_address;
use crate::keystore::{KeyStore, KeyStoreError};

#[derive(Debug, Error)]
pub enum SignerError {
    #[error("keystore error: {0}")]
    KeyStore(#[from] KeyStoreError),
    #[error("address mismatch: signer address {signer} does not match tx sender {sender}")]
    AddressMismatch { signer: String, sender: String },
}

/// A wallet signer backed by an unlocked ML-DSA keypair.
pub struct WalletSigner {
    keypair: MlDsaKeypair,
    address: AccountAddress,
}

impl WalletSigner {
    /// Create a signer from a raw keypair.
    pub fn from_keypair(keypair: MlDsaKeypair) -> Self {
        let address = derive_address(&keypair.public_key_bytes());
        Self { keypair, address }
    }

    /// Unlock a key from the keystore by name and create a signer.
    pub fn unlock(store: &KeyStore, name: &str, password: &str) -> Result<Self, SignerError> {
        let keypair = store.unlock_key(name, password)?;
        Ok(Self::from_keypair(keypair))
    }

    /// Unlock a key from the keystore by address and create a signer.
    pub fn unlock_by_address(
        store: &KeyStore,
        address: &AccountAddress,
        password: &str,
    ) -> Result<Self, SignerError> {
        let keypair = store.unlock_by_address(address, password)?;
        Ok(Self::from_keypair(keypair))
    }

    /// Get the signer's address.
    pub fn address(&self) -> &AccountAddress {
        &self.address
    }

    /// Get the signer's public key bytes.
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.keypair.public_key_bytes()
    }

    /// Sign raw bytes and return the signature.
    pub fn sign_bytes(&self, msg: &[u8]) -> Vec<u8> {
        self.keypair.sign(msg)
    }

    /// Sign a transaction in-place.
    /// Sets the `signature` and `public_key` fields on the transaction.
    pub fn sign_transaction(&self, tx: &mut Transaction) {
        let msg = tx.signable_bytes();
        let sig = self.keypair.sign(&msg);
        let pk = self.keypair.public_key_bytes();
        set_signature(tx, sig, pk);
    }

    /// Sign a transaction and return a new signed copy.
    pub fn sign(&self, tx: &Transaction) -> Transaction {
        let mut signed = tx.clone();
        self.sign_transaction(&mut signed);
        signed
    }
}

/// Set signature and public key on a transaction.
fn set_signature(tx: &mut Transaction, sig: Vec<u8>, pk: Vec<u8>) {
    match tx {
        Transaction::Transfer(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::Refresh(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::CreateObject(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::DeployContract(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::CallContract(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::DeployScript(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::CallScript(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::ValidatorStake(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::ValidatorExit(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_crypto::signatures::{MlDsaVerifier, Verifier};
    use evaporchain_types::TransferTx;

    fn make_signer() -> WalletSigner {
        let kp = MlDsaKeypair::generate();
        WalletSigner::from_keypair(kp)
    }

    #[test]
    fn test_sign_bytes() {
        let signer = make_signer();
        let msg = b"hello evaporchain";
        let sig = signer.sign_bytes(msg);
        assert!(!sig.is_empty());

        // Verify
        let pk = signer.public_key_bytes();
        assert!(MlDsaVerifier::verify(msg, &sig, &pk));
    }

    #[test]
    fn test_sign_transfer_transaction() {
        let signer = make_signer();
        let mut tx = Transaction::Transfer(TransferTx {
            from: *signer.address(),
            to: [2u8; 32],
            amount: 1000,
            nonce: 0,
            signature: None,
            public_key: None,
        });

        signer.sign_transaction(&mut tx);

        // Verify signature is set
        assert!(tx.signature().is_some());
        assert!(tx.public_key().is_some());

        // Verify signature is valid
        let msg = tx.signable_bytes();
        let sig = tx.signature().unwrap();
        let pk = tx.public_key().unwrap();
        assert!(MlDsaVerifier::verify(&msg, sig, pk));
    }

    #[test]
    fn test_sign_returns_new_copy() {
        let signer = make_signer();
        let tx = Transaction::Transfer(TransferTx {
            from: *signer.address(),
            to: [2u8; 32],
            amount: 500,
            nonce: 1,
            signature: None,
            public_key: None,
        });

        let signed = signer.sign(&tx);

        // Original unchanged
        assert!(tx.signature().is_none());
        // Signed copy has signature
        assert!(signed.signature().is_some());
    }

    #[test]
    fn test_signer_from_keystore() {
        let mut store = KeyStore::new();
        store.generate_key("signer_test", "pass").unwrap();

        let signer = WalletSigner::unlock(&store, "signer_test", "pass").unwrap();
        assert_ne!(*signer.address(), [0u8; 32]);

        // Sign something
        let sig = signer.sign_bytes(b"test");
        assert!(MlDsaVerifier::verify(
            b"test",
            &sig,
            &signer.public_key_bytes()
        ));
    }

    #[test]
    fn test_address_derivation_matches() {
        let signer = make_signer();
        let expected = derive_address(&signer.public_key_bytes());
        assert_eq!(*signer.address(), expected);
    }
}
