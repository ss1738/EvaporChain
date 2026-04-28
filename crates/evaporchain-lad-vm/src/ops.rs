//! Operational semantics: `use_resource`, `drop_resource`, `tick_decay`.

use thiserror::Error;

use crate::mode::Mode;
use crate::resource::Resource;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OpError {
    #[error("resource has already been consumed")]
    AlreadyConsumed,
    #[error("resource has evaporated past its decay window")]
    Evaporated,
    #[error(
        "Linear resource cannot be dropped — must be consumed exactly once"
    )]
    LinearCannotDrop,
}

/// Consume a resource, returning its inner value. Marks the resource
/// as consumed. Errors:
///
/// - `AlreadyConsumed` if the resource was already used.
/// - `Evaporated` if the resource is decaying and past its window.
pub fn use_resource<T>(
    mut r: Resource<T>,
    current_epoch: u64,
) -> Result<(T, Resource<()>), OpError> {
    if r.consumed {
        return Err(OpError::AlreadyConsumed);
    }
    if r.is_evaporated(current_epoch) {
        return Err(OpError::Evaporated);
    }
    let value = r.value;
    let receipt = Resource {
        value: (),
        mode: r.mode,
        created_at_epoch: r.created_at_epoch,
        decay_window: r.decay_window,
        consumed: true,
    };
    r.consumed = true;
    let _ = r; // we drop the original now-consumed wrapper
    Ok((value, receipt))
}

/// Drop a resource (without using its value). Allowed for Affine and
/// Decaying; rejected for Linear.
pub fn drop_resource<T>(r: Resource<T>) -> Result<(), OpError> {
    match r.mode {
        Mode::Linear => Err(OpError::LinearCannotDrop),
        Mode::Affine | Mode::Decaying => Ok(()),
    }
}

/// Advance a resource's decay clock. If the resource is decaying and
/// past its window, mark it consumed (== evaporated). Returns the
/// (possibly-updated) resource.
pub fn tick_decay<T>(mut r: Resource<T>, current_epoch: u64) -> Resource<T> {
    if r.consumed {
        return r;
    }
    if r.is_evaporated(current_epoch) {
        r.consumed = true;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_linear_consumes_and_returns_value() {
        let r = Resource::linear(42u64, 0);
        let (v, receipt) = use_resource(r, 1).unwrap();
        assert_eq!(v, 42);
        assert!(receipt.consumed);
    }

    #[test]
    fn use_already_consumed_rejected() {
        let mut r = Resource::affine(7u8, 0);
        r.consumed = true;
        let err = use_resource(r, 1).unwrap_err();
        assert_eq!(err, OpError::AlreadyConsumed);
    }

    #[test]
    fn use_evaporated_rejected() {
        let r = Resource::decaying(1u8, 0, 10);
        let err = use_resource(r, 100).unwrap_err();
        assert_eq!(err, OpError::Evaporated);
    }

    #[test]
    fn drop_linear_rejected() {
        let r = Resource::linear(0u8, 0);
        assert_eq!(drop_resource(r).unwrap_err(), OpError::LinearCannotDrop);
    }

    #[test]
    fn drop_affine_ok() {
        let r = Resource::affine(0u8, 0);
        drop_resource(r).unwrap();
    }

    #[test]
    fn drop_decaying_ok() {
        let r = Resource::decaying(0u8, 0, 10);
        drop_resource(r).unwrap();
    }

    #[test]
    fn tick_evaporates_past_window() {
        let r = Resource::decaying(7u8, 0, 10);
        assert!(!r.consumed);
        let after = tick_decay(r, 11);
        assert!(after.consumed);
    }

    #[test]
    fn tick_within_window_no_op() {
        let r = Resource::decaying(7u8, 0, 10);
        let after = tick_decay(r, 5);
        assert!(!after.consumed);
    }

    #[test]
    fn tick_already_consumed_no_op() {
        let mut r = Resource::decaying(7u8, 0, 10);
        r.consumed = true;
        let after = tick_decay(r, 11);
        assert!(after.consumed);
    }

    #[test]
    fn tick_linear_never_evaporates() {
        let r = Resource::linear(7u8, 0);
        let after = tick_decay(r, u64::MAX);
        assert!(!after.consumed);
    }
}
