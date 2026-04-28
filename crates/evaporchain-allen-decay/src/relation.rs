//! `AllenRelation` — Allen 1983's 13 interval relations.
//!
//! For two intervals `A = [a_s, a_e)` and `B = [b_s, b_e)`:
//!
//! ```text
//!   Before        :  a_e < b_s
//!   Meets         :  a_e = b_s
//!   Overlaps      :  a_s < b_s < a_e < b_e
//!   Starts        :  a_s = b_s, a_e < b_e
//!   During        :  a_s > b_s, a_e < b_e
//!   Finishes      :  a_s > b_s, a_e = b_e
//!   Equals        :  a_s = b_s, a_e = b_e
//!   FinishedBy    :  a_s < b_s, a_e = b_e
//!   Contains      :  a_s < b_s, a_e > b_e
//!   StartedBy     :  a_s = b_s, a_e > b_e
//!   OverlappedBy  :  b_s < a_s < b_e < a_e
//!   MetBy         :  a_s = b_e
//!   After         :  a_s > b_e
//! ```

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AllenRelation {
    Before,
    Meets,
    Overlaps,
    Starts,
    During,
    Finishes,
    Equals,
    FinishedBy,
    Contains,
    StartedBy,
    OverlappedBy,
    MetBy,
    After,
}

impl AllenRelation {
    /// Pair-flip inverse: `inverse(R(A, B)) == R(B, A)`.
    pub const fn inverse(self) -> Self {
        match self {
            Self::Before => Self::After,
            Self::Meets => Self::MetBy,
            Self::Overlaps => Self::OverlappedBy,
            Self::Starts => Self::StartedBy,
            Self::During => Self::Contains,
            Self::Finishes => Self::FinishedBy,
            Self::Equals => Self::Equals,
            Self::FinishedBy => Self::Finishes,
            Self::Contains => Self::During,
            Self::StartedBy => Self::Starts,
            Self::OverlappedBy => Self::Overlaps,
            Self::MetBy => Self::Meets,
            Self::After => Self::Before,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equals_is_self_inverse() {
        assert_eq!(AllenRelation::Equals.inverse(), AllenRelation::Equals);
    }

    #[test]
    fn before_inverse_after() {
        assert_eq!(AllenRelation::Before.inverse(), AllenRelation::After);
        assert_eq!(AllenRelation::After.inverse(), AllenRelation::Before);
    }

    #[test]
    fn double_inverse_identity() {
        for r in [
            AllenRelation::Before,
            AllenRelation::Meets,
            AllenRelation::Overlaps,
            AllenRelation::Starts,
            AllenRelation::During,
            AllenRelation::Finishes,
            AllenRelation::Equals,
            AllenRelation::FinishedBy,
            AllenRelation::Contains,
            AllenRelation::StartedBy,
            AllenRelation::OverlappedBy,
            AllenRelation::MetBy,
            AllenRelation::After,
        ] {
            assert_eq!(r.inverse().inverse(), r);
        }
    }
}
