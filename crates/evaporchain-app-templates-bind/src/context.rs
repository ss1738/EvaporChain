//! `BindContext` — the runtime context the chain provides when
//! binding a deploy.
//!
//! `TypedInit` is the *deploy intent* (what to build). `BindContext`
//! is the *deploy environment* (when, by whom, under what handle).
//! Together they give the chain's `ContractEngine` everything it
//! needs to call the per-primitive constructor.

use serde::{Deserialize, Serialize};

use evaporchain_app_templates_engine::TypedInit;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindContext {
    /// 32-byte account address of the deployer (signer of the
    /// deploy request). Most primitives' constructors use this as
    /// the initial owner / minter.
    pub deployer: [u8; 32],
    /// 32-byte deterministic on-chain handle for this instance.
    /// Same id every validator derives from the same request — see
    /// `evaporchain-app-templates-materialise::derive_instance_id`.
    pub instance_id: [u8; 32],
    /// Epoch at which the bind is happening. Most primitives need
    /// this for `minted_at_epoch` / `created_at_epoch` initialization.
    pub current_epoch: u64,
}

impl BindContext {
    pub fn new(deployer: [u8; 32], instance_id: [u8; 32], current_epoch: u64) -> Self {
        Self {
            deployer,
            instance_id,
            current_epoch,
        }
    }

    /// Convenience: pair a context with a typed init for the chain's
    /// `ContractEngine` to consume as a single argument.
    pub fn with_init(self, typed: TypedInit) -> (Self, TypedInit) {
        (self, typed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_serde() {
        let c = BindContext::new([0xAB; 32], [0xCD; 32], 1234);
        let s = serde_json::to_string(&c).unwrap();
        let back: BindContext = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn with_init_pairs_correctly() {
        use evaporchain_app_templates_engine::init_mayfly;
        let ctx = BindContext::new([0; 32], [1; 32], 0);
        let typed = TypedInit::Mayfly(init_mayfly::InitConfig {
            initial_energy: 100,
            half_life: 10,
        });
        let (c, t) = ctx.clone().with_init(typed.clone());
        assert_eq!(c, ctx);
        assert_eq!(t, typed);
    }
}
