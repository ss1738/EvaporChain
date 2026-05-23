//! Deployable-template catalogue for the new A5.* application
//! primitives.
//!
//! ## What this crate is for
//!
//! The chain ships a `ContractTemplate` enum (DecayingToken,
//! MortalNFT, …) that the dApp layer uses to pick *what kind of
//! contract to deploy*. The 19 application primitives this session
//! shipped (Singh-Sabi, Singh-Migrant, ChildKey, MnemoChain, Streak,
//! Resonance, Posthuma, Heartbeat, Lineage, etc.) are *Rust crates*,
//! not yet contract templates.
//!
//! This crate is the bridge: it registers each primitive as a
//! `TemplateDescriptor` — a metadata record (id + class + default
//! params + brief ABI hint) — so a wallet/dApp UI can:
//!
//! 1. Enumerate all available templates: `catalogue()`.
//! 2. Look up one by id: `find(class)`.
//! 3. Render a "deploy this primitive" form using the descriptor's
//!    `default_params` JSON shape and the `notes` field.
//!
//! ## What this crate is NOT
//!
//! It does NOT execute contracts. It does NOT touch state. It is
//! pure metadata + lookups. The actual on-chain execution happens
//! via the existing `ContractEngine` / EvaporScript path — this
//! crate just gives the dApp layer something to enumerate.
//!
//! Three structural decisions:
//!
//! 1. **`TemplateClass` is a stable u32 ID, not a Rust enum.** Adding
//!    a new template doesn't require recompiling consumers; they
//!    look up by id. The numeric range `0x0001_0000..0x0001_FFFF` is
//!    reserved for app templates so collisions with other on-chain
//!    typed handles (parameter ids, oracle ids, etc.) cannot happen.
//!
//! 2. **`default_params` is a JSON `serde_json::Value`**, not a typed
//!    struct. Each primitive has its own param shape; trying to
//!    unify them in Rust types would force a giant enum that
//!    couples this crate to every primitive. JSON is the lingua
//!    franca of dApp deployment forms anyway.
//!
//! 3. **The catalogue is sorted by class id and de-duplicated.**
//!    Validators reading the catalogue agree byte-for-byte; tests
//!    confirm `catalogue()` is canonical. (No `HashMap` — would be
//!    non-deterministic across processes.)
//!
//! ## Module map
//!
//! - [`class`] — [`TemplateClass`] u32 ID space + reserved ranges.
//! - [`descriptor`] — [`TemplateDescriptor`] metadata record.
//! - [`catalogue`] — [`catalogue`] + [`find`] lookups; the registered
//!   templates themselves.

pub mod catalogue;
pub mod class;
pub mod descriptor;

pub use catalogue::{catalogue, find, CatalogueError};
pub use class::{TemplateClass, APP_TEMPLATE_RANGE_END, APP_TEMPLATE_RANGE_START};
pub use descriptor::{DescriptorError, TemplateDescriptor};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "evaporchain-app-templates ships the deployable-
    /// template catalogue for all A5.* primitives. dApps/wallets
    /// enumerate `catalogue()`, look up specific classes via
    /// `find(class)`, and render deploy forms from each template's
    /// `default_params`. Catalogue is non-empty; every template
    /// belongs to the reserved A5.* class range; lookup is total
    /// (every catalogue entry is findable by its class)."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let cat = catalogue();
        assert!(!cat.is_empty(), "catalogue must not be empty");
        for t in cat.iter() {
            // Every template's class is in the reserved A5.* range.
            let cls_id = t.class.0;
            assert!(
                (APP_TEMPLATE_RANGE_START..=APP_TEMPLATE_RANGE_END).contains(&cls_id),
                "template class {:?} ({:#x}) out of A5.* range [{:#x}, {:#x}]",
                t.class,
                cls_id,
                APP_TEMPLATE_RANGE_START,
                APP_TEMPLATE_RANGE_END
            );
            // find(class) is total: every catalogued template is
            // findable by its class.
            assert!(find(t.class).is_ok(), "find({:?}) failed", t.class);
        }
    }
}
