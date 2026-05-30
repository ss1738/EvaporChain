//! The registered template catalogue.
//!
//! `catalogue()` returns a sorted, deduplicated list of all
//! application-layer templates. `find(class)` returns the descriptor
//! for one class. Both are pure functions; validators agree
//! byte-for-byte on the catalogue.

use serde_json::json;
use thiserror::Error;

use crate::class::*;
use crate::descriptor::TemplateDescriptor;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CatalogueError {
    #[error("no template registered for class {0:#010x}")]
    NotFound(u32),
}

/// Build the registered catalogue. Sorted by class id and
/// deduplicated. Pure function; validators agree.
pub fn catalogue() -> Vec<TemplateDescriptor> {
    // ── NFT lane ───────────────────────────────────────────────────
    let entries: Vec<TemplateDescriptor> = vec![
        TemplateDescriptor::new(
            SINGH_SABI,
            "Singh-Sabi (Patina Token)",
            "NFT",
            json!({"initial_energy": 1000, "floor_pct": 15, "half_life": 365}),
            "Wabi-sabi anchored NFT — ages toward 'ruined-beautiful' (~15% asymptote, never zero).",
            "evaporchain-singh-sabi",
        )
        .expect("Singh-Sabi descriptor is constant"),
        TemplateDescriptor::new(
            SINGH_MIGRANT,
            "Singh-Migrant (Wanderwrit)",
            "NFT",
            json!({
                "initial_energy": 1000,
                "base_half_life": 365,
                "resting_threshold_epochs": 30
            }),
            "NFT that dies if held still — kula-ring. Tiered decay 1×/2×/4× past resting threshold.",
            "evaporchain-singh-migrant",
        )
        .expect("Singh-Migrant descriptor is constant"),
        TemplateDescriptor::new(
            SINGH_RESONANCE,
            "Singh-Resonance (Vital-Sign NFT)",
            "NFT",
            json!({
                "initial_energy": 1000,
                "base_half_life": 100,
                "saturation": 1000,
                "max_scale_bp": 800
            }),
            "Engagement-coupled λ — loved art slows, ignored art accelerates. Anti-Black-Mirror max-scale ceiling.",
            "evaporchain-singh-resonance",
        )
        .expect("Singh-Resonance descriptor is constant"),
        TemplateDescriptor::new(
            MAYFLY,
            "Mayfly (Ephemeral NFT)",
            "NFT",
            json!({"initial_energy": 1000, "half_life": 10}),
            "Ephemeral NFT — the doctrine-purest NFT, with a half-life measured in epochs not years. No expire method, no terminal state; the contract's own energy IS the NFT's lifespan. Refresh works but the on_refresh hook explicitly notes the holder is defying the mayfly's nature. Reference contract: contracts/evaporscript/mayfly.es.",
            "evaporchain-mayfly",
        )
        .expect("Mayfly descriptor is constant"),
        TemplateDescriptor::new(
            SINGH_POSTHUMA,
            "Singh-Posthuma (Sealed Testament)",
            "NFT",
            json!({"half_life": 365, "initial_visible_energy": 1000, "m_threshold": 3, "n_committee": 5}),
            "Confessional NFT revealed on certified death. Decay suspended while issuer alive. New Yorker-grade.",
            "evaporchain-singh-posthuma",
        )
        .expect("Singh-Posthuma descriptor is constant"),
        TemplateDescriptor::new(
            MAYFLY,
            "Mayfly (Provably-Mortal NFT)",
            "NFT",
            json!({"initial_energy": 1000, "half_life": 100}),
            "NFT with on-chain death certificate at mint time. The Gallery That Forgets primitive.",
            "evaporchain-gallery-forgets",
        )
        .expect("Mayfly descriptor is constant"),
        // ── Marketplace lane ────────────────────────────────────────
        TemplateDescriptor::new(
            SDDC_AUCTION,
            "SDDC (Decay-Dutch Continuous Auction)",
            "Marketplace",
            json!({"ceiling": 1000, "floor": 100, "lot_lambda": 50, "duration_epochs": 1000}),
            "Two-axis Dutch auction — joint clearing on (price, λ-tolerance). Foundational mechanism.",
            "evaporchain-sddc",
        )
        .expect("SDDC descriptor is constant"),
        TemplateDescriptor::new(
            SFSV_VAULT,
            "SFSV (Future-Self Vault)",
            "Marketplace",
            json!({"future_self": "0x00", "predicate_type": 0, "release_param": 10000, "deposit_amount": 1000}),
            "Sell your future-self's claim — third parties bid for delayed-payout vaults via SDDC.",
            "evaporchain-sfsv",
        )
        .expect("SFSV descriptor is constant"),
        TemplateDescriptor::new(
            SHLM_BOUNTY,
            "SHLM (Skill Half-Life Bounty)",
            "Marketplace",
            json!({"salary_cap": 100000, "min_freshness": 500, "min_level": 100}),
            "Skill credentials decay at skill-specific λ (Python ~18mo, COBOL ~8yr). Recruiter market.",
            "evaporchain-shlm",
        )
        .expect("SHLM descriptor is constant"),
        TemplateDescriptor::new(
            SCL_LEASE,
            "SCL (Capability Lease)",
            "Marketplace",
            json!({"verb": "0x...", "object": "0x...", "duration_epochs": 1000}),
            "Permission market with structural revocation — capabilities can't outlive their purpose.",
            "evaporchain-scl",
        )
        .expect("SCL descriptor is constant"),
        TemplateDescriptor::new(
            SAP_AQ,
            "SAP (Attention Quantum)",
            "Marketplace",
            json!({"initial_value": 1000, "half_life": 45, "max_aq_per_window": 10, "window_epochs": 60}),
            "Attention with a half-life. 5-min-old AQ worth more than 25-min one. Anti-Sybil rate cap.",
            "evaporchain-sap",
        )
        .expect("SAP descriptor is constant"),
        TemplateDescriptor::new(
            REFRESH_MARKET_NAMESPACE,
            "Refresh-Market (Namespace Rent)",
            "Marketplace",
            json!({"id_hex": "00", "capacity": 1000, "base_rent": 100, "eviction_window": 10}),
            "Per-namespace AMM rent. Quadratic in utilisation: empty pays floor; full pays ~base. The chain's primary economic activity. Reference contract: contracts/evaporscript/refresh_market.es.",
            "evaporchain-refresh-market",
        )
        .expect("RefreshMarket descriptor is constant"),
        TemplateDescriptor::new(
            DECAY_ACCESS_PASS,
            "Decay Access Pass (Credential)",
            "Marketplace",
            json!({"energy": 1000, "half_life": 100, "validity_floor": 250}),
            "On-chain decaying credential — validity rides the contract's energy lifecycle; valid only while strength >= floor, evaporates unless the issuer refreshes. Issuer-gated issue/revoke. Reference contract: contracts/evaporscript/decay_access_pass.es.",
            "evaporchain-decay-credential",
        )
        .expect("DecayAccessPass descriptor is constant"),
        // ── Wallet UX lane ──────────────────────────────────────────
        TemplateDescriptor::new(
            SINGH_TRIAGE_CONTRACT,
            "Singh-Triage (Wallet Inbox)",
            "Wallet UX",
            json!({"horizon_today": 1, "horizon_tomorrow": 2, "horizon_week": 7}),
            "Wallet opens on inbox, not balance. Bucketed Today/Tomorrow/Week/Healthy/Decayed.",
            "evaporchain-singh-triage",
        )
        .expect("Triage descriptor is constant"),
        TemplateDescriptor::new(
            SINGH_HEARTBEAT_PULSE,
            "Singh-Heartbeat (Wallet Pulse)",
            "Wallet UX",
            json!({"healthy_bpm": 60, "alarmed_bpm": 120, "amber_threshold_bp": 75, "red_threshold_bp": 40}),
            "Wallet has a pulse — BPM + arrhythmia + 24-tick sparkline.",
            "evaporchain-singh-heartbeat",
        )
        .expect("Heartbeat descriptor is constant"),
        TemplateDescriptor::new(
            SINGH_LINEAGE_POLICY,
            "Singh-Lineage (Inheritance Policy)",
            "Wallet UX",
            json!({"ladder": [{"days": 90, "share_bp": 2500}, {"days": 180, "share_bp": 5000}, {"days": 365, "share_bp": 10000}]}),
            "Crypto solves inheritance — graduated dormancy thresholds.",
            "evaporchain-singh-lineage",
        )
        .expect("Lineage descriptor is constant"),
        // ── Consumer launch lane ───────────────────────────────────
        TemplateDescriptor::new(
            CHILDKEY_LETTER,
            "ChildKey (Singh Letter)",
            "Consumer",
            json!({"unlock_age_years": 18, "epochs_per_year": 365, "m_threshold": 3, "n_committee": 5}),
            "Sealed letters unlocked by recipient's age. Inverted decay. Today Show segment.",
            "evaporchain-childkey",
        )
        .expect("ChildKey descriptor is constant"),
        TemplateDescriptor::new(
            MNEMOCHAIN_CARD,
            "MnemoChain (FSRS Card)",
            "Consumer",
            json!({"initial_energy": 1000, "initial_stability": 10}),
            "Anki on-chain with FSRS forgetting curves. Portable cognitive credentials.",
            "evaporchain-mnemochain",
        )
        .expect("MnemoChain descriptor is constant"),
        TemplateDescriptor::new(
            WITNESSFIT_STREAK,
            "WitnessFit (Singh-Streak)",
            "Consumer",
            json!({"half_life": 7, "boost_bp": 5000}),
            "Wearable streaks where the streak ITSELF decays. Graceful fade, not cliff. Reference contract: contracts/evaporscript/witnessfit.es.",
            "evaporchain-witnessfit",
        )
        .expect("WitnessFit descriptor is constant"),
        // ── Cultural launch wedge ──────────────────────────────────
        TemplateDescriptor::new(
            GALLERY_FORGETS,
            "The Gallery That Forgets",
            "Cultural",
            json!({"opened_at_epoch": 0}),
            "Thermodynamic-closing gallery + Mayflies + AI-decay-art seed. 'The first thing humans have made that is provably going to die.'",
            "evaporchain-gallery-forgets",
        )
        .expect("Gallery descriptor is constant"),
        // ── Smart-contract paradigm primitives ─────────────────────
        TemplateDescriptor::new(
            SGB_TYPE_SYSTEM,
            "Singh-Girard (Linear-Logic !/?)",
            "Paradigm",
            json!({"fragment": "SLL"}),
            "Linear-logic exponentials at L1. Ephemerality is a type, not a runtime convention.",
            "evaporchain-sgb",
        )
        .expect("SGB descriptor is constant"),
        TemplateDescriptor::new(
            SBAV_RUNTIME,
            "Singh-Bennett (Reversible VM)",
            "Paradigm",
            json!({"reg_count": 8}),
            "Reversible compute — only DECAY exports entropy. Landauer literal.",
            "evaporchain-sbav",
        )
        .expect("SBAV descriptor is constant"),
        TemplateDescriptor::new(
            SSM_GAME_SEMANTICS,
            "Singh Strategy Machines",
            "Paradigm",
            json!({"fragment": "innocent_well_bracketed"}),
            "Game-semantic contracts — Hyland-Ong arenas + AJM innocent strategies. Decay = visibility condition.",
            "evaporchain-ssm",
        )
        .expect("SSM descriptor is constant"),
        TemplateDescriptor::new(
            BELL_ORACLE,
            "Bell-Oracle (Quantum-Certified Beacon)",
            "Paradigm",
            json!({
                "energy": 1000,
                "half_life": 100,
                "threshold_milli": 2000,
                "max_age_epochs": 10
            }),
            "On-chain consumer of EvaporChain's per-block CHSH S-value beacon. Structurally rejects readings at or below the local-realism floor (2000 milli = S=2.0) so the stored state is certifiably quantum-derived. Downstream contracts gate quantum-randomness-requiring actions on is_certified_now(). Reference contract: contracts/evaporscript/bell_oracle.es.",
            "evaporchain-bell-oracle",
        )
        .expect("BellOracle descriptor is constant"),
        // ── Governance lane ────────────────────────────────────────
        TemplateDescriptor::new(
            MORTAL_DAO,
            "Mortal-DAO (Decay-Native Governance)",
            "Governance",
            json!({
                "energy": 1000,
                "half_life": 100,
                "freshness_window": 500,
                "proposal_cap": 3,
                "voting_window": 50
            }),
            "Single-instance DAO that composes all four decay primitives in one contract: members refresh to stay active (decay-credential); per-member proposal cap resets on refresh (decay-rate-limit); vote weight grows with participation (decay-reputation); quorum threshold tracks a running peak of engagement (decay-quorum). Reference contract: contracts/evaporscript/mortal_dao.es.",
            "evaporchain-mortal-dao",
        )
        .expect("MortalDAO descriptor is constant"),
    ];

    // Sort by class id; dedupe (defensive).
    let mut sorted = entries;
    sorted.sort_by_key(|d| d.class.0);
    sorted.dedup_by_key(|d| d.class.0);
    sorted
}

/// Look up the descriptor for one class. Errors with `NotFound` if
/// the class isn't registered.
pub fn find(class: crate::class::TemplateClass) -> Result<TemplateDescriptor, CatalogueError> {
    catalogue()
        .into_iter()
        .find(|d| d.class == class)
        .ok_or(CatalogueError::NotFound(class.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_sorted_by_class_id() {
        let cat = catalogue();
        for w in cat.windows(2) {
            assert!(
                w[0].class.0 < w[1].class.0,
                "catalogue not sorted: {:#010x} >= {:#010x}",
                w[0].class.0,
                w[1].class.0
            );
        }
    }

    #[test]
    fn catalogue_has_no_duplicate_class_ids() {
        let cat = catalogue();
        let mut ids: Vec<u32> = cat.iter().map(|d| d.class.0).collect();
        let len_before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), len_before, "duplicate class ids found");
    }

    #[test]
    fn catalogue_is_deterministic_across_calls() {
        // Validators must agree byte-for-byte. Two calls return the
        // same JSON.
        let a = serde_json::to_vec(&catalogue()).unwrap();
        let b = serde_json::to_vec(&catalogue()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn catalogue_covers_all_seven_lanes() {
        let cat = catalogue();
        let lanes: std::collections::BTreeSet<&str> = cat.iter().map(|d| d.lane.as_str()).collect();
        // The 7 lanes the doctrine ships (Governance added 2026-05-30
        // with MortalDAO — see class.rs comment block).
        for l in [
            "NFT",
            "Marketplace",
            "Wallet UX",
            "Consumer",
            "Cultural",
            "Paradigm",
            "Governance",
        ] {
            assert!(lanes.contains(l), "lane {l} missing from catalogue");
        }
    }

    #[test]
    fn catalogue_lists_25_templates() {
        // Anti-regression: dropping a primitive accidentally would
        // shrink the catalogue. 20 was the original Singh-named set;
        // 21 added RefreshMarket (2026-05-09); 22 added the Decay
        // Access Pass credential (2026-05-29); 23 added Mortal-DAO
        // and opened the Governance lane (2026-05-30); 24 added the
        // Bell-Oracle in the Paradigm lane; 25 adds the Mayfly
        // descriptor — class id existed in class.rs since the original
        // NFT lane definition, but no catalogue entry until today
        // (2026-05-30 night). NFT lane is now 5/5 backed.
        let cat = catalogue();
        assert_eq!(cat.len(), 25);
    }

    #[test]
    fn find_returns_known_descriptor() {
        let d = find(SINGH_SABI).unwrap();
        assert_eq!(d.class, SINGH_SABI);
        assert!(d.brand.contains("Sabi"));
    }

    #[test]
    fn find_returns_not_found_for_unknown_class() {
        // Out of range
        let err = find(TemplateClass(0x9999_9999)).unwrap_err();
        assert!(matches!(err, CatalogueError::NotFound(_)));
        // In range but unregistered (gaps in the 0x0001_06xx range)
        let err = find(TemplateClass(0x0001_0600)).unwrap_err();
        assert!(matches!(err, CatalogueError::NotFound(_)));
    }

    #[test]
    fn every_descriptor_points_at_a_real_crate() {
        // Anti-regression: catalogue's `crate_name` must match the
        // workspace crate the primitive lives in. We don't load the
        // crate; we just check the conventional naming.
        for d in catalogue() {
            assert!(
                d.crate_name.starts_with("evaporchain-"),
                "descriptor for {:?} has bad crate_name '{}'",
                d.class,
                d.crate_name
            );
        }
    }

    #[test]
    fn the_dapp_layer_can_enumerate_what_it_can_deploy() {
        // The whole point: a wallet/dApp UI can fetch this list and
        // render a "what kind of contract do you want to deploy?"
        // form. Verify the shape is renderable.
        let cat = catalogue();
        for d in cat {
            assert!(!d.brand.is_empty());
            assert!(!d.notes.is_empty());
            // default_params must be a JSON object so the form can
            // iterate keys; any well-formed JSON is acceptable but
            // most descriptors use objects (some use empty `{}`).
            assert!(d.default_params.is_object());
        }
    }
}
