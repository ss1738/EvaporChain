//! Mayfly — provably-mortal NFT.
//!
//! A `MayflyToken` is a thin wrapper around a Singh-Sabi `PatinaToken`
//! with one structural override: the `floor` is pinned at zero. Where
//! Singh-Sabi tokens age toward "ruined-beautiful," Mayflies *actually
//! die* — their score reaches zero, and the death is provable from
//! `(token_id, mint inputs, half_life)` alone.
//!
//! At mint time, before the token has decayed at all, we emit a
//! `DeathCertificate` — a Blake3 commitment over the mint inputs plus
//! the projected death epoch. The certificate exists from minute zero;
//! anyone who reads it can verify "this NFT will die at epoch T" *before*
//! the death actually happens. **That's the press claim, made operational.**
//!
//! Projected death epoch is derived from the half-life rule: with
//! integer-doubling decay, an NFT with `initial_energy = E` and
//! `half_life = h` reaches zero after `h * (1 + log2(E))` epochs in the
//! worst case (when E rounds to 0 only after many halvings) or
//! `h * log2(E + 1)` epochs in the standard case. We adopt the
//! *worst-case* projection so the certificate is always honored: the
//! token dies *no later than* the certified epoch, but may die sooner.

use evaporchain_singh_sabi::{patina_score, PatinaError, PatinaParams, PatinaState};
use evaporchain_types::{AccountAddress, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MayflyError {
    #[error("patina parameter setup failed: {0}")]
    Patina(#[from] PatinaError),
    #[error("transfer caller {caller:?} is not the owner {owner:?}")]
    NotOwner {
        caller: AccountAddress,
        owner: AccountAddress,
    },
    #[error("self-transfer is a no-op")]
    SelfTransfer,
    #[error("mayfly has died — transfers blocked")]
    Died,
}

/// Cryptographic certificate emitted at mint time. Anyone reading the
/// certificate before the death epoch knows that the token *will* die
/// no later than `projected_death_epoch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeathCertificate {
    pub token_id: [u8; 32],
    pub minted_at_epoch: Epoch,
    /// Worst-case epoch by which the token's score will be zero. The
    /// token may die earlier; never later.
    pub projected_death_epoch: Epoch,
    /// Blake3 commitment to (token_id ∥ minted_at ∥ initial_energy ∥
    /// half_life ∥ projected_death_epoch). Stable across validators.
    pub commitment: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MayflyToken {
    pub id: [u8; 32],
    pub minted_by: AccountAddress,
    pub owner: AccountAddress,
    pub minted_at_epoch: Epoch,
    /// Patina parameters with `floor = 0` (genuine death; not asymptote).
    pub params: PatinaParams,
    pub certificate: DeathCertificate,
}

impl MayflyToken {
    /// Mint a Mayfly. Floor is pinned to zero (overrides Singh-Sabi's
    /// 15% default). Emits a `DeathCertificate` at the same time.
    pub fn mint(
        id: [u8; 32],
        minted_by: AccountAddress,
        initial_energy: Energy,
        half_life: HalfLife,
        minted_at_epoch: Epoch,
    ) -> Result<Self, MayflyError> {
        // floor=0 ⇒ tokens actually die rather than asymptoting.
        let params = PatinaParams::new(initial_energy, 0, half_life)?;
        let projected_death_epoch =
            project_worst_case_death(initial_energy, half_life, minted_at_epoch);
        let commitment = compute_commitment(
            &id,
            minted_at_epoch,
            initial_energy,
            half_life,
            projected_death_epoch,
        );
        let certificate = DeathCertificate {
            token_id: id,
            minted_at_epoch,
            projected_death_epoch,
            commitment,
        };
        Ok(Self {
            id,
            minted_by,
            owner: minted_by,
            minted_at_epoch,
            params,
            certificate,
        })
    }

    /// Numeric score at `epoch_now`. Reaches zero — Mayflies don't
    /// asymptote.
    pub fn score_at(&self, epoch_now: Epoch) -> Energy {
        patina_score(&self.params, self.minted_at_epoch, epoch_now)
    }

    /// True iff the token has actually died (score = 0) at epoch_now.
    pub fn is_dead(&self, epoch_now: Epoch) -> bool {
        self.score_at(epoch_now) == 0
    }

    /// Read-only visual state (delegated to Singh-Sabi's entropy
    /// derivation; for Mayflies the "fully aged" state is reached at
    /// the death epoch since floor=0).
    pub fn witness(&self, epoch_now: Epoch) -> PatinaState {
        let score = self.score_at(epoch_now);
        evaporchain_singh_sabi::derive_state(&self.params, score)
    }

    /// Transfer ownership. Caller must be the current owner; mayflies
    /// that have died refuse all transfers.
    pub fn transfer(
        &mut self,
        caller: AccountAddress,
        new_owner: AccountAddress,
        epoch_now: Epoch,
    ) -> Result<(), MayflyError> {
        if caller != self.owner {
            return Err(MayflyError::NotOwner {
                caller,
                owner: self.owner,
            });
        }
        if new_owner == self.owner {
            return Err(MayflyError::SelfTransfer);
        }
        if self.is_dead(epoch_now) {
            return Err(MayflyError::Died);
        }
        self.owner = new_owner;
        Ok(())
    }

    /// Verify that the certificate matches the token's parameters.
    /// Pure function: re-computes the Blake3 commitment from the
    /// token's stored fields and checks it equals the certificate's
    /// commitment. Anyone reading the chain can run this check.
    pub fn verify_certificate(&self) -> bool {
        let recomputed = compute_commitment(
            &self.id,
            self.minted_at_epoch,
            self.params.initial,
            self.params.half_life,
            self.certificate.projected_death_epoch,
        );
        recomputed == self.certificate.commitment
    }
}

/// Worst-case projection: how many epochs after mint until score = 0?
///
/// `patina_score` (Singh-Sabi) with `floor = 0` reduces to
/// `energy_at_epoch(initial, half_life, elapsed)`, which is integer-
/// halving + linear interpolation. The score reaches zero when
/// `initial >> full_halvings == 0`, i.e. when `full_halvings >
/// log2(initial)`. So the worst-case bound is
/// `half_life * (1 + floor(log2(initial)) + 1)`.
fn project_worst_case_death(
    initial: Energy,
    half_life: HalfLife,
    minted_at_epoch: Epoch,
) -> Epoch {
    if initial == 0 {
        return minted_at_epoch;
    }
    // log2(initial) = position of highest set bit. Add 2 for safety
    // (one for the +1 inside the >> formula, one to cover the
    // fractional-decay path).
    let log2_initial = 64u64.saturating_sub(initial.leading_zeros() as u64);
    let halvings_needed = log2_initial.saturating_add(2);
    minted_at_epoch.saturating_add(half_life.saturating_mul(halvings_needed))
}

fn compute_commitment(
    token_id: &[u8; 32],
    minted_at_epoch: Epoch,
    initial_energy: Energy,
    half_life: HalfLife,
    projected_death_epoch: Epoch,
) -> [u8; 32] {
    // Order is part of the contract; do not reorder without a version bump.
    let mut hasher = blake3_hasher();
    hasher.update(token_id);
    hasher.update(&minted_at_epoch.to_le_bytes());
    hasher.update(&initial_energy.to_le_bytes());
    hasher.update(&half_life.to_le_bytes());
    hasher.update(&projected_death_epoch.to_le_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(hasher.finalize().as_bytes());
    out
}

fn blake3_hasher() -> blake3::Hasher {
    blake3::Hasher::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    #[test]
    fn mint_pins_floor_to_zero() {
        let m = MayflyToken::mint(id(1), addr(0xAA), 1000, 100, 0).unwrap();
        assert_eq!(m.params.floor, 0);
    }

    #[test]
    fn at_mint_token_is_alive() {
        let m = MayflyToken::mint(id(1), addr(0xAA), 1000, 100, 0).unwrap();
        assert!(!m.is_dead(0));
        assert_eq!(m.score_at(0), 1000);
    }

    #[test]
    fn mayfly_dies_eventually() {
        // Doctrine claim: Mayflies actually die. floor=0 means score
        // reaches zero in finite time.
        let m = MayflyToken::mint(id(1), addr(0xAA), 1000, 1, 0).unwrap();
        // After enough halvings, score=0.
        assert!(m.is_dead(10_000));
    }

    #[test]
    fn certificate_projected_death_is_an_upper_bound() {
        // The certificate promises the token dies *no later than*
        // projected_death_epoch. For any realistic param choice, the
        // token must indeed be dead at that epoch.
        for (initial, hl) in [(100u64, 5u64), (1000, 10), (50_000, 50)] {
            let m = MayflyToken::mint(id(1), addr(0xAA), initial, hl, 0).unwrap();
            let projected = m.certificate.projected_death_epoch;
            assert!(
                m.is_dead(projected),
                "token must be dead by projected={projected} for initial={initial} hl={hl}"
            );
        }
    }

    #[test]
    fn certificate_verifies_against_token_state() {
        let m = MayflyToken::mint(id(1), addr(0xAA), 1000, 100, 5).unwrap();
        assert!(m.verify_certificate());
    }

    #[test]
    fn certificate_commitment_is_deterministic() {
        let m1 = MayflyToken::mint(id(1), addr(0xAA), 1000, 100, 5).unwrap();
        let m2 = MayflyToken::mint(id(1), addr(0xAA), 1000, 100, 5).unwrap();
        assert_eq!(m1.certificate.commitment, m2.certificate.commitment);
    }

    #[test]
    fn certificate_commitment_changes_with_inputs() {
        let m1 = MayflyToken::mint(id(1), addr(0xAA), 1000, 100, 5).unwrap();
        let m2 = MayflyToken::mint(id(1), addr(0xAA), 1001, 100, 5).unwrap();
        assert_ne!(m1.certificate.commitment, m2.certificate.commitment);
    }

    #[test]
    fn transfer_after_death_is_blocked() {
        // Doctrine guard: dead Mayflies can't be transferred. They've
        // *provably* died; the chain refuses to ferry them.
        let mut m = MayflyToken::mint(id(1), addr(0xAA), 4, 1, 0).unwrap();
        // After many half-lives, score=0.
        let err = m.transfer(addr(0xAA), addr(0xBB), 1_000).unwrap_err();
        assert_eq!(err, MayflyError::Died);
    }

    #[test]
    fn transfer_requires_owner() {
        let mut m = MayflyToken::mint(id(1), addr(0xAA), 1000, 100, 0).unwrap();
        let err = m.transfer(addr(0xBB), addr(0xCC), 5).unwrap_err();
        assert!(matches!(err, MayflyError::NotOwner { .. }));
    }

    #[test]
    fn self_transfer_rejected() {
        let mut m = MayflyToken::mint(id(1), addr(0xAA), 1000, 100, 0).unwrap();
        let err = m.transfer(addr(0xAA), addr(0xAA), 5).unwrap_err();
        assert_eq!(err, MayflyError::SelfTransfer);
    }

    #[test]
    fn round_trip_serde() {
        let m = MayflyToken::mint(id(7), addr(0xAB), 5_000, 365, 42).unwrap();
        let s = serde_json::to_string(&m).unwrap();
        let back: MayflyToken = serde_json::from_str(&s).unwrap();
        assert_eq!(m, back);
        assert!(back.verify_certificate());
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // "It is the first thing humans have made that is provably
        // going to die." Operationalised: the certificate is emitted
        // at mint time, before any decay; verifies against the token;
        // and the token is provably dead at the certified epoch.
        let m = MayflyToken::mint(id(0xAA), addr(1), 1000, 50, 0).unwrap();
        // At mint time (epoch 0), token is alive but the certificate
        // already promises a death epoch:
        assert!(!m.is_dead(0));
        let promised = m.certificate.projected_death_epoch;
        assert!(promised > 0);
        // The certificate verifies cryptographically against the token:
        assert!(m.verify_certificate());
        // And by the promised epoch, the token is provably dead:
        assert!(m.is_dead(promised));
    }
}
