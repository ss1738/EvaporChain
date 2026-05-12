//! `Testament` — the on-chain sealed confession.
//!
//! Lifecycle:
//!
//! ```text
//!     seal() ──► Sealed { issuer, vault, half_life }
//!         │
//!         │ accept_death_certificate(cert) — committee threshold met
//!         ▼
//!     Revealed { revealed_at, ciphertext_hash, half_life clock starts }
//!         │
//!         │ epoch_now > revealed_at + half_life × log2(...)
//!         ▼
//!     Memorial { memorial_marker } — readable form has decayed;
//!                                    permanent attestation remains.
//! ```
//!
//! Decay is **suspended** while Sealed. The half-life clock starts
//! the moment a valid `DeathCertificate` is accepted — not at mint.
//! The cipher-pointer fades over the half-life into a 32-byte
//! `MemorialMarker` that the chain keeps forever.

use evaporchain_types::{energy_at_epoch, AccountAddress, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::certificate::{verify_certificate, CertificateError, DeathCertificate};
use crate::vault::SealedVault;

pub type TestamentId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TestamentError {
    #[error("half_life must be > 0")]
    ZeroHalfLife,
    #[error("initial_visible_energy must be > 0")]
    ZeroInitialEnergy,
    #[error("testament is not Sealed (already Revealed or Memorial)")]
    NotSealed,
    #[error("testament has not yet been Revealed; no readable form to fade")]
    NotRevealed,
    #[error("certificate verification failed: {0}")]
    Certificate(#[from] CertificateError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestamentStatus {
    /// Issuer alive; payload encrypted; decay suspended.
    Sealed,
    /// Death certified; payload pointer is now public; half-life
    /// clock running.
    Revealed { revealed_at_epoch: Epoch },
    /// Half-life elapsed; the readable form is gone. Only the
    /// `MemorialMarker` remains.
    Memorial(MemorialMarker),
}

/// 32-byte permanent attestation that a testament existed, was
/// issued by X, and was revealed at Y. Stays on chain forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorialMarker {
    pub testament_id: TestamentId,
    pub issuer: AccountAddress,
    pub revealed_at_epoch: Epoch,
    /// BLAKE3 commitment over (testament_id ∥ issuer ∥
    /// revealed_at_epoch ∥ ciphertext_hash). Reproducible from the
    /// chain alone; permanent record.
    pub commitment: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Testament {
    pub id: TestamentId,
    pub issuer: AccountAddress,
    pub vault: SealedVault,
    /// Half-life applied to `initial_visible_energy` *after* reveal.
    /// While Sealed, this knob is dormant.
    pub half_life: HalfLife,
    /// "Visible energy" budget for the post-reveal fade. The shape
    /// matches every other decaying object on chain — a number that
    /// halves every `half_life` epochs starting at reveal time.
    pub initial_visible_energy: Energy,
    pub sealed_at_epoch: Epoch,
    pub status: TestamentStatus,
}

impl Testament {
    /// Seal a testament. Decay does not start yet; the issuer is
    /// presumed alive.
    pub fn seal(
        id: TestamentId,
        issuer: AccountAddress,
        vault: SealedVault,
        half_life: HalfLife,
        initial_visible_energy: Energy,
        sealed_at_epoch: Epoch,
    ) -> Result<Self, TestamentError> {
        if half_life == 0 {
            return Err(TestamentError::ZeroHalfLife);
        }
        if initial_visible_energy == 0 {
            return Err(TestamentError::ZeroInitialEnergy);
        }
        Ok(Self {
            id,
            issuer,
            vault,
            half_life,
            initial_visible_energy,
            sealed_at_epoch,
            status: TestamentStatus::Sealed,
        })
    }

    pub fn is_sealed(&self) -> bool {
        matches!(self.status, TestamentStatus::Sealed)
    }

    pub fn is_revealed(&self) -> bool {
        matches!(self.status, TestamentStatus::Revealed { .. })
    }

    pub fn is_memorial(&self) -> bool {
        matches!(self.status, TestamentStatus::Memorial(_))
    }

    /// Accept a `DeathCertificate`. If it verifies against the
    /// vault's threshold + committee, transition the testament to
    /// `Revealed`. The reveal epoch is the certificate's death epoch
    /// (not "now") — that's what the issuer's testament was
    /// promised against.
    pub fn accept_death_certificate<F>(
        &mut self,
        cert: &DeathCertificate,
        is_signature_valid: F,
    ) -> Result<(), TestamentError>
    where
        F: Fn(&AccountAddress, &[u8], &[u8]) -> bool,
    {
        if !self.is_sealed() {
            return Err(TestamentError::NotSealed);
        }
        verify_certificate(cert, self.id, &self.vault, is_signature_valid)?;
        self.status = TestamentStatus::Revealed {
            revealed_at_epoch: cert.death_epoch,
        };
        Ok(())
    }

    /// Visible energy at `epoch_now`. While Sealed, returns the full
    /// initial — *decay is suspended*. Once Revealed, decays per the
    /// standard half-life rule from `revealed_at`. Once Memorial,
    /// returns 0.
    pub fn visible_energy_at(&self, epoch_now: Epoch) -> Energy {
        match &self.status {
            TestamentStatus::Sealed => self.initial_visible_energy,
            TestamentStatus::Revealed { revealed_at_epoch } => {
                let elapsed = epoch_now.saturating_sub(*revealed_at_epoch);
                energy_at_epoch(self.initial_visible_energy, self.half_life, elapsed)
            }
            TestamentStatus::Memorial(_) => 0,
        }
    }

    /// Fade the readable form to a `MemorialMarker` if the visible
    /// energy has reached zero. Errors if the testament isn't
    /// currently Revealed (Sealed has no readable form yet; Memorial
    /// has already faded).
    pub fn fade_to_memorial(&mut self, epoch_now: Epoch) -> Result<(), TestamentError> {
        let revealed_at = match self.status {
            TestamentStatus::Revealed { revealed_at_epoch } => revealed_at_epoch,
            TestamentStatus::Sealed => return Err(TestamentError::NotRevealed),
            TestamentStatus::Memorial(_) => return Err(TestamentError::NotRevealed),
        };
        if self.visible_energy_at(epoch_now) > 0 {
            return Err(TestamentError::NotRevealed);
        }
        let marker = MemorialMarker {
            testament_id: self.id,
            issuer: self.issuer,
            revealed_at_epoch: revealed_at,
            commitment: compute_marker_commitment(
                &self.id,
                &self.issuer,
                revealed_at,
                &self.vault.ciphertext_hash,
            ),
        };
        self.status = TestamentStatus::Memorial(marker);
        Ok(())
    }
}

fn compute_marker_commitment(
    id: &TestamentId,
    issuer: &AccountAddress,
    revealed_at: Epoch,
    ciphertext_hash: &[u8; 32],
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(id);
    hasher.update(issuer);
    hasher.update(&revealed_at.to_le_bytes());
    hasher.update(ciphertext_hash);
    hasher.update(b"singh-posthuma:memorial:v1");
    let mut out = [0u8; 32];
    out.copy_from_slice(hasher.finalize().as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::certificate::{Attestation, DeathCertificate};
    use crate::vault::SealedVault;

    fn validator(b: u8) -> AccountAddress {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    fn vault(threshold: usize) -> SealedVault {
        SealedVault::new(
            [0xAB; 32],
            1024,
            threshold,
            vec![
                validator(1),
                validator(2),
                validator(3),
                validator(4),
                validator(5),
            ],
            [0xCD; 32],
        )
        .unwrap()
    }

    fn fresh(id_byte: u8, issuer_byte: u8, hl: HalfLife) -> Testament {
        let mut id = [0u8; 32];
        id[0] = id_byte;
        let mut issuer = [0u8; 32];
        issuer[0] = issuer_byte;
        Testament::seal(id, issuer, vault(2), hl, 1000, 0).unwrap()
    }

    fn cert_for(id: TestamentId, death_epoch: Epoch, signers: Vec<u8>) -> DeathCertificate {
        let attestations = signers
            .into_iter()
            .map(|b| Attestation {
                validator: validator(b),
                signature: vec![b; 8],
            })
            .collect();
        DeathCertificate {
            testament_id: id,
            death_epoch,
            nonce: [0xEE; 32],
            attestations,
        }
    }

    fn always_valid(_v: &AccountAddress, _b: &[u8], _s: &[u8]) -> bool {
        true
    }

    #[test]
    fn seal_rejects_zero_half_life() {
        let v = vault(2);
        let mut id = [0u8; 32];
        id[0] = 1;
        let err = Testament::seal(id, [0xAA; 32], v, 0, 1000, 0).unwrap_err();
        assert_eq!(err, TestamentError::ZeroHalfLife);
    }

    #[test]
    fn seal_rejects_zero_visible_energy() {
        let v = vault(2);
        let mut id = [0u8; 32];
        id[0] = 1;
        let err = Testament::seal(id, [0xAA; 32], v, 100, 0, 0).unwrap_err();
        assert_eq!(err, TestamentError::ZeroInitialEnergy);
    }

    #[test]
    fn newly_sealed_is_in_sealed_status() {
        let t = fresh(1, 0xAA, 100);
        assert!(t.is_sealed());
        assert!(!t.is_revealed());
        assert!(!t.is_memorial());
    }

    #[test]
    fn decay_is_suspended_while_sealed() {
        // Doctrine claim: "Decay suspended while issuer is verifiably
        // alive." Even after thousands of epochs, a Sealed testament
        // shows its full initial visible_energy — the half-life clock
        // hasn't started.
        let t = fresh(1, 0xAA, 100);
        assert_eq!(t.visible_energy_at(0), 1000);
        assert_eq!(t.visible_energy_at(1_000_000), 1000);
    }

    #[test]
    fn accept_certificate_transitions_to_revealed() {
        let mut t = fresh(1, 0xAA, 100);
        let mut id = [0u8; 32];
        id[0] = 1;
        let c = cert_for(id, 500, vec![1, 2]);
        t.accept_death_certificate(&c, always_valid).unwrap();
        assert!(t.is_revealed());
        match t.status {
            TestamentStatus::Revealed { revealed_at_epoch } => {
                assert_eq!(revealed_at_epoch, 500);
            }
            other => panic!("expected Revealed, got {other:?}"),
        }
    }

    #[test]
    fn cannot_accept_certificate_twice() {
        let mut t = fresh(1, 0xAA, 100);
        let mut id = [0u8; 32];
        id[0] = 1;
        let c = cert_for(id, 500, vec![1, 2]);
        t.accept_death_certificate(&c, always_valid).unwrap();
        let err = t.accept_death_certificate(&c, always_valid).unwrap_err();
        assert_eq!(err, TestamentError::NotSealed);
    }

    #[test]
    fn certificate_below_threshold_does_not_reveal() {
        let mut t = fresh(1, 0xAA, 100);
        let mut id = [0u8; 32];
        id[0] = 1;
        // Only 1 signer; threshold is 2.
        let c = cert_for(id, 500, vec![1]);
        let err = t.accept_death_certificate(&c, always_valid).unwrap_err();
        assert!(matches!(err, TestamentError::Certificate(_)));
        // Testament must remain Sealed on rejection.
        assert!(t.is_sealed());
    }

    #[test]
    fn revealed_testament_decays_per_half_life() {
        let mut t = fresh(1, 0xAA, 100);
        let mut id = [0u8; 32];
        id[0] = 1;
        let c = cert_for(id, 500, vec![1, 2]);
        t.accept_death_certificate(&c, always_valid).unwrap();
        // At reveal epoch, energy ≈ initial.
        assert_eq!(t.visible_energy_at(500), 1000);
        // After one half-life (epoch 600), energy ≈ 500.
        let e_one_hl = t.visible_energy_at(600);
        assert!(e_one_hl < 1000);
        assert!(e_one_hl > 0);
    }

    #[test]
    fn fade_to_memorial_succeeds_when_visible_zero() {
        let mut t = fresh(1, 0xAA, 1); // very fast decay
        let mut id = [0u8; 32];
        id[0] = 1;
        let c = cert_for(id, 0, vec![1, 2]);
        t.accept_death_certificate(&c, always_valid).unwrap();
        // After many half-lives, visible energy is zero.
        t.fade_to_memorial(10_000).unwrap();
        assert!(t.is_memorial());
        match &t.status {
            TestamentStatus::Memorial(m) => {
                assert_eq!(m.testament_id[0], 1);
                assert_eq!(m.revealed_at_epoch, 0);
                assert_ne!(m.commitment, [0u8; 32]);
            }
            other => panic!("expected Memorial, got {other:?}"),
        }
    }

    #[test]
    fn fade_to_memorial_rejected_while_still_visible() {
        let mut t = fresh(1, 0xAA, 100);
        let mut id = [0u8; 32];
        id[0] = 1;
        let c = cert_for(id, 0, vec![1, 2]);
        t.accept_death_certificate(&c, always_valid).unwrap();
        // Way before the half-life, visible energy is still positive.
        let err = t.fade_to_memorial(10).unwrap_err();
        assert_eq!(err, TestamentError::NotRevealed);
    }

    #[test]
    fn fade_rejected_while_sealed() {
        let mut t = fresh(1, 0xAA, 100);
        let err = t.fade_to_memorial(0).unwrap_err();
        assert_eq!(err, TestamentError::NotRevealed);
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // "the first NFT that's a deathbed confession." Operationalised:
        // 1. Decay suspended while alive (Sealed).
        // 2. Multisig committee certifies death.
        // 3. Testament reveals + half-life clock starts.
        // 4. Visible form fades; permanent marker remains.
        let mut t = fresh(1, 0xAA, 1); // fast decay so the test is cheap
                                       // Step 1: while alive, energy preserved indefinitely.
        assert_eq!(t.visible_energy_at(1_000_000), 1000);
        // Step 2 + 3: committee certifies death; reveal happens.
        let mut id = [0u8; 32];
        id[0] = 1;
        let c = cert_for(id, 100, vec![1, 2, 3]);
        t.accept_death_certificate(&c, always_valid).unwrap();
        assert!(t.is_revealed());
        // Step 4: the readable form fades; the marker persists.
        t.fade_to_memorial(1_000).unwrap();
        assert!(t.is_memorial());
    }

    #[test]
    fn memorial_commitment_changes_with_inputs() {
        let mut t1 = fresh(1, 0xAA, 1);
        let mut t2 = fresh(2, 0xAA, 1);
        let mut id1 = [0u8; 32];
        id1[0] = 1;
        let mut id2 = [0u8; 32];
        id2[0] = 2;
        // Different testament ids ⇒ different commitments.
        let c1 = cert_for(id1, 0, vec![1, 2]);
        let c2 = cert_for(id2, 0, vec![1, 2]);
        t1.accept_death_certificate(&c1, always_valid).unwrap();
        t2.accept_death_certificate(&c2, always_valid).unwrap();
        t1.fade_to_memorial(10_000).unwrap();
        t2.fade_to_memorial(10_000).unwrap();
        let m1 = match &t1.status {
            TestamentStatus::Memorial(m) => m.commitment,
            _ => unreachable!(),
        };
        let m2 = match &t2.status {
            TestamentStatus::Memorial(m) => m.commitment,
            _ => unreachable!(),
        };
        assert_ne!(m1, m2);
    }

    #[test]
    fn round_trip_serde_full_lifecycle() {
        // Sealed:
        let t_sealed = fresh(7, 0xAB, 365);
        let s = serde_json::to_string(&t_sealed).unwrap();
        let back: Testament = serde_json::from_str(&s).unwrap();
        assert_eq!(t_sealed, back);

        // Revealed:
        let mut t_rev = t_sealed.clone();
        let mut id = [0u8; 32];
        id[0] = 7;
        let c = cert_for(id, 100, vec![1, 2]);
        t_rev.accept_death_certificate(&c, always_valid).unwrap();
        let s = serde_json::to_string(&t_rev).unwrap();
        let back: Testament = serde_json::from_str(&s).unwrap();
        assert_eq!(t_rev, back);

        // Memorial:
        let mut t_mem = fresh(7, 0xAB, 1);
        let c2 = cert_for(id, 0, vec![1, 2]);
        t_mem.accept_death_certificate(&c2, always_valid).unwrap();
        t_mem.fade_to_memorial(10_000).unwrap();
        let s = serde_json::to_string(&t_mem).unwrap();
        let back: Testament = serde_json::from_str(&s).unwrap();
        assert_eq!(t_mem, back);
    }

    /// T1.20 — Testament::visible_energy_at on a Sealed
    /// testament returns initial_visible_energy (line 156).
    /// Sealed branch was uncovered.
    #[test]
    fn t1_20_visible_energy_at_on_sealed_returns_initial() {
        let t = fresh(1, 0xAA, 100);
        // Sealed → returns the initial pool regardless of epoch.
        assert_eq!(t.visible_energy_at(0), 1000);
        assert_eq!(t.visible_energy_at(99_999), 1000);
    }

    /// T1.20 — visible_energy_at on a Memorial testament returns 0
    /// (line 161). Memorial branch was uncovered for this getter.
    #[test]
    fn t1_20_visible_energy_at_on_memorial_returns_zero() {
        let mut t = fresh(1, 0xAA, 100);
        let mut id = [0u8; 32];
        id[0] = 1;
        let c = cert_for(id, 0, vec![1, 2]);
        t.accept_death_certificate(&c, always_valid).unwrap();
        // Walk well past the half-life so visible energy reaches 0.
        let big_epoch = 10_000_000;
        // Now fade to memorial.
        t.fade_to_memorial(big_epoch).unwrap();
        assert_eq!(t.visible_energy_at(big_epoch), 0);
        assert_eq!(t.visible_energy_at(big_epoch + 1000), 0);
    }

    /// T1.20 — fade_to_memorial on an already-Memorial testament
    /// returns NotRevealed (line 173, the Memorial arm).
    #[test]
    fn t1_20_fade_to_memorial_twice_rejected() {
        let mut t = fresh(1, 0xAA, 100);
        let mut id = [0u8; 32];
        id[0] = 1;
        let c = cert_for(id, 0, vec![1, 2]);
        t.accept_death_certificate(&c, always_valid).unwrap();
        t.fade_to_memorial(10_000_000).unwrap();
        // Already Memorial — second fade rejects.
        let err = t.fade_to_memorial(20_000_000).unwrap_err();
        assert_eq!(err, TestamentError::NotRevealed);
    }
}
