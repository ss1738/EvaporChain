//! `Mode` — substructural mode of a resource.

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
