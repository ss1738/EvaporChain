//! `BoundedParameter` — chain parameter with hard-coded bounds.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type ParameterId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BoundedParameter {
    pub id: ParameterId,
    pub current: u64,
    pub min: u64,
    pub max: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParameterError {
    #[error("min ({min}) > max ({max}) — bounds inverted")]
    BoundsInverted { min: u64, max: u64 },
    #[error("current ({current}) outside bounds [{min}, {max}]")]
    CurrentOutOfBounds { current: u64, min: u64, max: u64 },
}

impl BoundedParameter {
    pub fn new(id: ParameterId, current: u64, min: u64, max: u64) -> Result<Self, ParameterError> {
        if min > max {
            return Err(ParameterError::BoundsInverted { min, max });
        }
        if current < min || current > max {
            return Err(ParameterError::CurrentOutOfBounds {
                current,
                min,
                max,
            });
        }
        Ok(Self { id, current, min, max })
    }

    /// Clamp a proposed value into the parameter's bounds.
    pub fn clamp(&self, proposed: u64) -> u64 {
        proposed.clamp(self.min, self.max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_construction() {
        let p = BoundedParameter::new(1, 50, 0, 100).unwrap();
        assert_eq!(p.current, 50);
    }

    #[test]
    fn bounds_inverted_rejected() {
        assert!(matches!(
            BoundedParameter::new(1, 5, 100, 50).unwrap_err(),
            ParameterError::BoundsInverted { .. }
        ));
    }

    #[test]
    fn current_out_of_bounds_rejected() {
        assert!(matches!(
            BoundedParameter::new(1, 200, 0, 100).unwrap_err(),
            ParameterError::CurrentOutOfBounds { .. }
        ));
    }

    #[test]
    fn clamp_inside_bounds_passes() {
        let p = BoundedParameter::new(1, 50, 0, 100).unwrap();
        assert_eq!(p.clamp(75), 75);
        assert_eq!(p.clamp(150), 100);
        assert_eq!(p.clamp(0), 0);
    }
}
