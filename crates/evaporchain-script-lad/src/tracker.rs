//! Per-call LAD resource tracker.
//!
//! Before each contract call the tracker is populated from the annotations
//! (one `Resource<u64>` slot per annotated field). After the call completes,
//! the caller ticks each resource and inspects its verdict.

use crate::annotation::LadAnnotation;
use crate::error::LadScriptError;
use evaporchain_lad_vm::{drop_resource, tick_decay, use_resource, Mode, OpError, Resource};
use std::collections::BTreeMap;

/// Verdict for a single LAD resource after one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceVerdict {
    /// Resource is still live; current value.
    Live { value: u64 },
    /// Resource was consumed (`use_resource` succeeded).
    Consumed,
    /// Resource was dropped (`drop_resource` succeeded).
    Dropped,
    /// Resource evaporated (decaying resource past its window).
    Evaporated,
}

impl ResourceVerdict {
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Live { .. })
    }
    pub fn is_consumed(&self) -> bool {
        *self == Self::Consumed
    }
    pub fn is_evaporated(&self) -> bool {
        *self == Self::Evaporated
    }
}

/// State for one resource slot in the tracker.
#[derive(Debug, Clone)]
enum Slot {
    /// Resource is still available.
    Live(Resource<u64>),
    /// Resource was consumed (use) or dropped.
    Consumed,
    /// Resource was evaporated by tick_decay.
    Evaporated,
}

impl Slot {
    #[allow(dead_code)]
    fn verdict(&self) -> ResourceVerdict {
        match self {
            Self::Live(r) => ResourceVerdict::Live { value: r.value },
            Self::Consumed => ResourceVerdict::Consumed,
            Self::Evaporated => ResourceVerdict::Evaporated,
        }
    }
}

/// Tracks LAD resources for a single script contract call.
#[derive(Debug, Clone, Default)]
pub struct LadResourceTracker {
    slots: BTreeMap<String, Slot>,
}

impl LadResourceTracker {
    /// Initialise from annotation declarations at `created_epoch`.
    pub fn from_annotations(annotations: &[LadAnnotation], created_epoch: u64) -> Self {
        let mut slots = BTreeMap::new();
        for ann in annotations {
            let resource: Resource<u64> = match ann.mode {
                Mode::Linear => Resource::linear(ann.initial_value, created_epoch),
                Mode::Affine => Resource::affine(ann.initial_value, created_epoch),
                Mode::Decaying => {
                    let window = ann.decay_window.unwrap_or(100);
                    Resource::decaying(ann.initial_value, created_epoch, window)
                }
            };
            slots.insert(ann.field_name.clone(), Slot::Live(resource));
        }
        Self { slots }
    }

    /// Attempt to consume (use) a named resource at `current_epoch`.
    pub fn use_resource(
        &mut self,
        name: &str,
        current_epoch: u64,
    ) -> Result<ResourceVerdict, LadScriptError> {
        let slot = self
            .slots
            .get_mut(name)
            .ok_or_else(|| LadScriptError::UnknownResource(name.to_string()))?;

        let resource = match std::mem::replace(slot, Slot::Consumed) {
            Slot::Live(r) => r,
            other => {
                *slot = other;
                return Err(LadScriptError::LadVmError {
                    name: name.to_string(),
                    detail: "resource is not live (consumed or evaporated)".into(),
                });
            }
        };

        match use_resource(resource, current_epoch) {
            Ok(_) => {
                *slot = Slot::Consumed;
                Ok(ResourceVerdict::Consumed)
            }
            Err(OpError::AlreadyConsumed) => {
                *slot = Slot::Consumed;
                Err(LadScriptError::LadVmError {
                    name: name.to_string(),
                    detail: "already consumed".into(),
                })
            }
            Err(OpError::Evaporated) => {
                *slot = Slot::Evaporated;
                Err(LadScriptError::LadVmError {
                    name: name.to_string(),
                    detail: "evaporated".into(),
                })
            }
            Err(e) => Err(LadScriptError::LadVmError {
                name: name.to_string(),
                detail: e.to_string(),
            }),
        }
    }

    /// Attempt to drop a named resource. Linear resources cannot be dropped.
    pub fn drop_resource(&mut self, name: &str) -> Result<ResourceVerdict, LadScriptError> {
        let slot = self
            .slots
            .get_mut(name)
            .ok_or_else(|| LadScriptError::UnknownResource(name.to_string()))?;

        let resource = match std::mem::replace(slot, Slot::Consumed) {
            Slot::Live(r) => r,
            other => {
                *slot = other;
                return Err(LadScriptError::LadVmError {
                    name: name.to_string(),
                    detail: "resource is not live (consumed or evaporated)".into(),
                });
            }
        };

        match drop_resource(resource) {
            Ok(()) => {
                *slot = Slot::Consumed;
                Ok(ResourceVerdict::Dropped)
            }
            Err(OpError::LinearCannotDrop) => {
                // Reconstruct — we consumed the value above, so we restore a sentinel.
                // The error means the call is invalid; the resource should remain live.
                // We can't restore the value since drop_resource took ownership.
                // Use a sentinel value — callers must not call drop on Linear.
                *slot = Slot::Evaporated; // mark as evaporated (irrecoverable after wrong call)
                Err(LadScriptError::LadVmError {
                    name: name.to_string(),
                    detail: "linear resources cannot be dropped — must be used exactly once".into(),
                })
            }
            Err(e) => Err(LadScriptError::LadVmError {
                name: name.to_string(),
                detail: e.to_string(),
            }),
        }
    }

    /// Tick decay for all live resources. Returns the verdict map.
    pub fn tick_all(&mut self, current_epoch: u64) -> BTreeMap<String, ResourceVerdict> {
        let mut verdicts = BTreeMap::new();
        for (name, slot) in &mut self.slots {
            match std::mem::replace(slot, Slot::Consumed) {
                Slot::Live(r) => {
                    let after = tick_decay(r, current_epoch);
                    if after.consumed {
                        *slot = Slot::Evaporated;
                        verdicts.insert(name.clone(), ResourceVerdict::Evaporated);
                    } else {
                        let v = after.value;
                        *slot = Slot::Live(after);
                        verdicts.insert(name.clone(), ResourceVerdict::Live { value: v });
                    }
                }
                Slot::Consumed => {
                    *slot = Slot::Consumed;
                    verdicts.insert(name.clone(), ResourceVerdict::Consumed);
                }
                Slot::Evaporated => {
                    *slot = Slot::Evaporated;
                    verdicts.insert(name.clone(), ResourceVerdict::Evaporated);
                }
            }
        }
        verdicts
    }

    /// Snapshot current verdicts without mutating state.
    pub fn snapshot(&self, current_epoch: u64) -> BTreeMap<String, ResourceVerdict> {
        self.slots
            .iter()
            .map(|(name, slot)| {
                let verdict = match slot {
                    Slot::Live(r) => {
                        if r.is_evaporated(current_epoch) {
                            ResourceVerdict::Evaporated
                        } else {
                            ResourceVerdict::Live { value: r.value }
                        }
                    }
                    Slot::Consumed => ResourceVerdict::Consumed,
                    Slot::Evaporated => ResourceVerdict::Evaporated,
                };
                (name.clone(), verdict)
            })
            .collect()
    }

    /// All current verdicts (via snapshot).
    pub fn verdicts(&self, current_epoch: u64) -> BTreeMap<String, ResourceVerdict> {
        self.snapshot(current_epoch)
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::LadAnnotation;
    use evaporchain_lad_vm::Mode;

    fn ann(name: &str, mode: Mode, value: u64, window: Option<u64>) -> LadAnnotation {
        LadAnnotation {
            field_name: name.to_string(),
            mode,
            decay_window: window,
            initial_value: value,
        }
    }

    #[test]
    fn tracker_from_annotations() {
        let anns = vec![
            ann("tok", Mode::Linear, 100, None),
            ann("cap", Mode::Affine, 200, None),
        ];
        let tracker = LadResourceTracker::from_annotations(&anns, 0);
        assert_eq!(tracker.len(), 2);
    }

    #[test]
    fn use_linear_resource_succeeds() {
        let anns = vec![ann("tok", Mode::Linear, 100, None)];
        let mut tracker = LadResourceTracker::from_annotations(&anns, 0);
        let v = tracker.use_resource("tok", 1).unwrap();
        assert_eq!(v, ResourceVerdict::Consumed);
    }

    #[test]
    fn use_twice_fails() {
        let anns = vec![ann("tok", Mode::Linear, 100, None)];
        let mut tracker = LadResourceTracker::from_annotations(&anns, 0);
        tracker.use_resource("tok", 1).unwrap();
        assert!(tracker.use_resource("tok", 2).is_err());
    }

    #[test]
    fn drop_linear_fails() {
        let anns = vec![ann("tok", Mode::Linear, 100, None)];
        let mut tracker = LadResourceTracker::from_annotations(&anns, 0);
        let err = tracker.drop_resource("tok").unwrap_err();
        assert!(matches!(err, LadScriptError::LadVmError { .. }));
    }

    #[test]
    fn drop_affine_succeeds() {
        let anns = vec![ann("cap", Mode::Affine, 200, None)];
        let mut tracker = LadResourceTracker::from_annotations(&anns, 0);
        let v = tracker.drop_resource("cap").unwrap();
        assert_eq!(v, ResourceVerdict::Dropped);
    }

    #[test]
    fn decaying_resource_evaporates_after_window() {
        let anns = vec![ann("voucher", Mode::Decaying, 100, Some(5))];
        let mut tracker = LadResourceTracker::from_annotations(&anns, 0);
        let verdicts = tracker.tick_all(10); // well past window=5
        assert_eq!(verdicts["voucher"], ResourceVerdict::Evaporated);
    }

    #[test]
    fn decaying_resource_live_before_window() {
        let anns = vec![ann("voucher", Mode::Decaying, 100, Some(50))];
        let tracker = LadResourceTracker::from_annotations(&anns, 0);
        let snap = tracker.snapshot(10); // epoch 10 < window 50
        assert!(matches!(
            snap["voucher"],
            ResourceVerdict::Live { value: 100 }
        ));
    }

    #[test]
    fn snapshot_live_resources() {
        let anns = vec![ann("tok", Mode::Linear, 50, None)];
        let tracker = LadResourceTracker::from_annotations(&anns, 0);
        let snap = tracker.snapshot(1);
        assert!(matches!(snap["tok"], ResourceVerdict::Live { value: 50 }));
    }

    #[test]
    fn tick_updates_slot_to_consumed() {
        let anns = vec![ann("cap", Mode::Affine, 100, None)];
        let mut tracker = LadResourceTracker::from_annotations(&anns, 0);
        tracker.use_resource("cap", 1).unwrap();
        let verdicts = tracker.tick_all(2);
        assert_eq!(verdicts["cap"], ResourceVerdict::Consumed);
    }

    #[test]
    fn unknown_resource_returns_error() {
        let anns = vec![ann("tok", Mode::Linear, 100, None)];
        let mut tracker = LadResourceTracker::from_annotations(&anns, 0);
        assert!(tracker.use_resource("nonexistent", 1).is_err());
    }
}
