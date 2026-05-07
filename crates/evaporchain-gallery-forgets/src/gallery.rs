//! Gallery — the orchestration contract.
//!
//! A `Gallery` collects exhibits (works deposited by artists with
//! their own declared half-lives) and tracks the *thermodynamic
//! closing date*: the epoch at which **all** exhibits have passed
//! their projected death.
//!
//! There is no calendrical "close on March 14" date. The gallery
//! closes when its last exhibit dies. Reciprocally, the gallery
//! reopens — i.e., is back in `Open` — only if at least one new
//! exhibit is admitted before the last has died.
//!
//! The gallery's status is computed pure-function from the exhibit
//! list + `epoch_now`:
//!
//! - `Open` — at least one exhibit's projected_death_epoch > now AND
//!   no exhibit has died yet.
//! - `Closing` — at least one exhibit has died, but at least one is
//!   still alive.
//! - `Closed` — every exhibit's projected_death_epoch ≤ now.
//!
//! Exhibits cannot be removed once deposited (the artist commits to
//! the work's death; the gallery doesn't curate corpses out). This is
//! the "performance art" property of A2.3 — *the closing date is
//! thermodynamic*.

use evaporchain_types::{AccountAddress, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::mayfly::MayflyToken;

pub type ExhibitId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GalleryError {
    #[error("artist {caller:?} is not the gallery curator {curator:?}")]
    NotCurator {
        caller: AccountAddress,
        curator: AccountAddress,
    },
    #[error("exhibit with id {0:?} already deposited")]
    DuplicateExhibit(ExhibitId),
    #[error("invalid mayfly certificate — refuses to admit")]
    InvalidCertificate,
}

/// One artist's contribution to the gallery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exhibit {
    pub id: ExhibitId,
    pub artist: AccountAddress,
    pub mayfly: MayflyToken,
    pub deposited_at_epoch: Epoch,
}

/// Aggregate gallery lifecycle state. Pure-function of the exhibits +
/// epoch_now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GalleryStatus {
    /// All exhibits still alive.
    Open,
    /// At least one exhibit has died; at least one still alive.
    Closing,
    /// Every exhibit has died.
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gallery {
    pub id: [u8; 32],
    pub name: String,
    pub curator: AccountAddress,
    pub opened_at_epoch: Epoch,
    pub exhibits: Vec<Exhibit>,
}

impl Gallery {
    pub fn open(
        id: [u8; 32],
        name: impl Into<String>,
        curator: AccountAddress,
        opened_at_epoch: Epoch,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            curator,
            opened_at_epoch,
            exhibits: Vec::new(),
        }
    }

    /// Deposit a Mayfly into the gallery. Caller must be the curator.
    /// The Mayfly's certificate must verify (we never admit a
    /// commitment we can't reproduce).
    pub fn deposit(
        &mut self,
        caller: AccountAddress,
        exhibit_id: ExhibitId,
        artist: AccountAddress,
        mayfly: MayflyToken,
        epoch_now: Epoch,
    ) -> Result<(), GalleryError> {
        if caller != self.curator {
            return Err(GalleryError::NotCurator {
                caller,
                curator: self.curator,
            });
        }
        if self.exhibits.iter().any(|e| e.id == exhibit_id) {
            return Err(GalleryError::DuplicateExhibit(exhibit_id));
        }
        if !mayfly.verify_certificate() {
            return Err(GalleryError::InvalidCertificate);
        }
        self.exhibits.push(Exhibit {
            id: exhibit_id,
            artist,
            mayfly,
            deposited_at_epoch: epoch_now,
        });
        Ok(())
    }

    /// Aggregate status at `epoch_now`.
    pub fn status_at(&self, epoch_now: Epoch) -> GalleryStatus {
        if self.exhibits.is_empty() {
            // An empty gallery is conceptually "Open" — no exhibits
            // have died because none exist. The curator can deposit.
            return GalleryStatus::Open;
        }
        let any_dead = self.exhibits.iter().any(|e| e.mayfly.is_dead(epoch_now));
        let all_dead = self.exhibits.iter().all(|e| e.mayfly.is_dead(epoch_now));
        match (any_dead, all_dead) {
            (false, _) => GalleryStatus::Open,
            (true, true) => GalleryStatus::Closed,
            (true, false) => GalleryStatus::Closing,
        }
    }

    /// **The thermodynamic closing date** — the epoch by which the
    /// gallery is guaranteed to be `Closed`. Equals the max of all
    /// exhibits' projected_death_epoch. Returns `None` for an empty
    /// gallery (no thermodynamic clock without exhibits).
    pub fn thermodynamic_close_epoch(&self) -> Option<Epoch> {
        self.exhibits
            .iter()
            .map(|e| e.mayfly.certificate.projected_death_epoch)
            .max()
    }

    /// Count of exhibits still alive at `epoch_now`.
    pub fn alive_count(&self, epoch_now: Epoch) -> usize {
        self.exhibits
            .iter()
            .filter(|e| !e.mayfly.is_dead(epoch_now))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mayfly::MayflyToken;

    fn id(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    fn fresh_gallery() -> Gallery {
        Gallery::open(id(0xFF), "The Gallery That Forgets", addr(0xC1), 0)
    }

    fn mayfly(token_byte: u8, initial: u64, hl: u64) -> MayflyToken {
        MayflyToken::mint(id(token_byte), addr(0xAA), initial, hl, 0).unwrap()
    }

    #[test]
    fn empty_gallery_is_open() {
        let g = fresh_gallery();
        assert_eq!(g.status_at(0), GalleryStatus::Open);
        assert_eq!(g.thermodynamic_close_epoch(), None);
        assert_eq!(g.alive_count(0), 0);
    }

    #[test]
    fn deposit_requires_curator() {
        let mut g = fresh_gallery();
        let m = mayfly(1, 1000, 100);
        let err = g.deposit(addr(0xBB), id(1), addr(0xAA), m, 0).unwrap_err();
        assert!(matches!(err, GalleryError::NotCurator { .. }));
    }

    #[test]
    fn duplicate_exhibit_rejected() {
        let mut g = fresh_gallery();
        let m1 = mayfly(1, 1000, 100);
        let m2 = mayfly(2, 1000, 100);
        g.deposit(addr(0xC1), id(99), addr(0xAA), m1, 0).unwrap();
        let err = g
            .deposit(addr(0xC1), id(99), addr(0xBB), m2, 0)
            .unwrap_err();
        assert_eq!(err, GalleryError::DuplicateExhibit(id(99)));
    }

    #[test]
    fn invalid_certificate_rejected() {
        let mut g = fresh_gallery();
        let mut m = mayfly(1, 1000, 100);
        // Tamper the commitment.
        m.certificate.commitment[0] ^= 0xFF;
        let err = g.deposit(addr(0xC1), id(1), addr(0xAA), m, 0).unwrap_err();
        assert_eq!(err, GalleryError::InvalidCertificate);
    }

    #[test]
    fn status_open_when_all_alive() {
        let mut g = fresh_gallery();
        for i in 1..=3u8 {
            let m = mayfly(i, 1_000_000, 1000); // long-lived
            g.deposit(addr(0xC1), id(i), addr(0xAA), m, 0).unwrap();
        }
        assert_eq!(g.status_at(10), GalleryStatus::Open);
        assert_eq!(g.alive_count(10), 3);
    }

    #[test]
    fn status_closing_when_some_dead() {
        // Doctrine claim: the gallery's visual state changes daily.
        // Mix short-lived and long-lived exhibits so the gallery
        // transitions Open → Closing → Closed deterministically.
        let mut g = fresh_gallery();
        let short = mayfly(1, 4, 1); // dies fast
        let long = mayfly(2, 1_000_000, 1000); // survives long
        g.deposit(addr(0xC1), id(1), addr(0xAA), short, 0).unwrap();
        g.deposit(addr(0xC1), id(2), addr(0xBB), long, 0).unwrap();
        // At epoch 1000, the short one is dead; the long one is alive.
        assert_eq!(g.status_at(1000), GalleryStatus::Closing);
        assert_eq!(g.alive_count(1000), 1);
    }

    #[test]
    fn status_closed_when_all_dead() {
        let mut g = fresh_gallery();
        for i in 1..=3u8 {
            let m = mayfly(i, 4, 1); // all die fast
            g.deposit(addr(0xC1), id(i), addr(0xAA), m, 0).unwrap();
        }
        // After many half-lives, everything has died.
        assert_eq!(g.status_at(10_000), GalleryStatus::Closed);
        assert_eq!(g.alive_count(10_000), 0);
    }

    #[test]
    fn thermodynamic_close_epoch_is_max_of_certificates() {
        let mut g = fresh_gallery();
        let short = mayfly(1, 4, 1);
        let medium = mayfly(2, 100, 5);
        let long = mayfly(3, 1_000_000, 100);
        let projected_long = long.certificate.projected_death_epoch;
        g.deposit(addr(0xC1), id(1), addr(0xAA), short, 0).unwrap();
        g.deposit(addr(0xC1), id(2), addr(0xBB), medium, 0).unwrap();
        g.deposit(addr(0xC1), id(3), addr(0xCC), long, 0).unwrap();
        // The thermodynamic close is bounded by the longest-lived
        // exhibit — the gallery is guaranteed Closed by then.
        let close = g.thermodynamic_close_epoch().unwrap();
        assert_eq!(close, projected_long);
        // Sanity: at the thermodynamic close, all exhibits are dead.
        assert_eq!(g.status_at(close), GalleryStatus::Closed);
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // "Closing date is *thermodynamic*." Operationalised: there is
        // no external date; the gallery's `Closed` state is computed
        // pure-function from the exhibits' decay against `epoch_now`.
        // The thermodynamic_close_epoch is a worst-case upper bound —
        // by that epoch the gallery is *guaranteed* Closed; it may
        // close earlier if actual decay outpaces the bound.
        let mut g = fresh_gallery();
        // Big-energy exhibit: long-lived enough that a much-earlier
        // epoch is unambiguously Open while the projected close is far
        // in the future.
        let m = mayfly(1, 1_000_000_000, 100);
        g.deposit(addr(0xC1), id(1), addr(0xAA), m, 0).unwrap();
        let promised = g.thermodynamic_close_epoch().unwrap();
        // At a much-earlier epoch the gallery is still Open.
        assert_eq!(g.status_at(0), GalleryStatus::Open);
        // By the promised epoch it's guaranteed Closed.
        assert_eq!(g.status_at(promised), GalleryStatus::Closed);
        // The promised epoch comes from the chain alone — no calendar.
        assert!(promised > 0);
    }

    #[test]
    fn round_trip_serde() {
        let mut g = fresh_gallery();
        let m = mayfly(7, 5_000, 365);
        g.deposit(addr(0xC1), id(7), addr(0xAB), m, 42).unwrap();
        let s = serde_json::to_string(&g).unwrap();
        let back: Gallery = serde_json::from_str(&s).unwrap();
        assert_eq!(g, back);
    }
}
