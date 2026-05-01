//! `Mode` — substructural mode of a resource.

use evaporchain_types::LadMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    /// Must be consumed exactly once. `drop` is forbidden.
    Linear,
    /// May be consumed at most once. `drop` is allowed.
    Affine,
    /// Affine + decays automatically after a window. The window is
    /// stored on the `Resource`.
    Decaying,
}

// ─── Bridge to/from evaporchain-types::LadMode ───────────────────────
//
// `evaporchain-types` carries a parallel `LadMode` enum so that the
// types crate doesn't have to depend on this one (which would be a
// circular dep — lad-vm already depends on types). The conversions
// here keep the two representations in lockstep: any code that needs
// the runtime semantics works with `Mode` from this crate; any code
// that touches `StateObject` (state, exec, api wire layer, wallet)
// works with `LadMode`. The conversions are total and lossless.

impl From<Mode> for LadMode {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Linear => LadMode::Linear,
            Mode::Affine => LadMode::Affine,
            Mode::Decaying => LadMode::Decaying,
        }
    }
}

impl From<LadMode> for Mode {
    fn from(m: LadMode) -> Self {
        match m {
            LadMode::Linear => Mode::Linear,
            LadMode::Affine => Mode::Affine,
            LadMode::Decaying => Mode::Decaying,
        }
    }
}

#[cfg(test)]
mod conv_tests {
    use super::*;

    #[test]
    fn mode_roundtrip_through_lad_mode() {
        for m in [Mode::Linear, Mode::Affine, Mode::Decaying] {
            let back: Mode = LadMode::from(m).into();
            assert_eq!(m, back);
        }
    }

    #[test]
    fn lad_mode_roundtrip_through_mode() {
        for m in [LadMode::Linear, LadMode::Affine, LadMode::Decaying] {
            let back: LadMode = Mode::from(m).into();
            assert_eq!(m, back);
        }
    }

    #[test]
    fn lad_mode_serialises_lowercase() {
        // The wire format (used by /api/objects) must be the lowercase
        // string the wallet expects: "linear" | "affine" | "decaying".
        let s = serde_json::to_string(&LadMode::Linear).unwrap();
        assert_eq!(s, "\"linear\"");
        let s = serde_json::to_string(&LadMode::Affine).unwrap();
        assert_eq!(s, "\"affine\"");
        let s = serde_json::to_string(&LadMode::Decaying).unwrap();
        assert_eq!(s, "\"decaying\"");
    }
}
