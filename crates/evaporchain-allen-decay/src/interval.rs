//! `Interval` — energy-bounded `[start, end)` half-open interval.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Interval {
    pub start: Energy,
    pub end: Energy,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IntervalError {
    #[error("interval is empty or inverted: start={start} ≥ end={end}")]
    EmptyOrInverted { start: Energy, end: Energy },
}

impl Interval {
    pub fn new(start: Energy, end: Energy) -> Result<Self, IntervalError> {
        if start >= end {
            return Err(IntervalError::EmptyOrInverted { start, end });
        }
        Ok(Self { start, end })
    }

    pub fn duration(&self) -> Energy {
        self.end - self.start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_interval() {
        let i = Interval::new(1, 5).unwrap();
        assert_eq!(i.duration(), 4);
    }

    #[test]
    fn empty_rejected() {
        assert!(Interval::new(5, 5).is_err());
    }

    #[test]
    fn inverted_rejected() {
        assert!(Interval::new(5, 1).is_err());
    }
}
