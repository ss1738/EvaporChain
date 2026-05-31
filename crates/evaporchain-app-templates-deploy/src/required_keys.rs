//! Required-keys table for each template class.
//!
//! Each catalogue entry has a `default_params` JSON shape. At deploy
//! time, we require the *required* subset of those keys to be
//! present (the others can fall back to defaults). This table lives
//! here, not in the catalogue, because it's the deploy-time contract
//! — the catalogue's `default_params` is for UI pre-population, not
//! validation.
//!
//! Returning an empty slice for an unrecognised class is intentional
//! — forward-compat for a V2 primitive landing before this table is
//! updated. The `validate_against_descriptor` caller is responsible
//! for combining "class is in catalogue" + "required keys present"
//! checks.

use evaporchain_app_templates::{
    class::{
        BELL_ORACLE, CHILDKEY_LETTER, DEADMAN_SWITCH, DECAY_ACCESS_PASS, GALLERY_FORGETS, MAYFLY,
        MNEMOCHAIN_CARD, MORTAL_DAO, REFRESH_MARKET_NAMESPACE, SAP_AQ, SBAV_RUNTIME, SCL_LEASE,
        SDDC_AUCTION, SFSV_VAULT, SGB_TYPE_SYSTEM, SHLM_BOUNTY, SINGH_HEARTBEAT_PULSE,
        SINGH_LINEAGE_POLICY, SINGH_MIGRANT, SINGH_POSTHUMA, SINGH_RESONANCE, SINGH_SABI,
        SINGH_TRIAGE_CONTRACT, SSM_GAME_SEMANTICS, SUBSCRIPTION_SERVICE, WITNESSFIT_STREAK,
    },
    TemplateClass,
};

/// Required parameter keys for `class`. Returns an empty slice if
/// the class isn't in the table (forward-compat).
pub fn required_keys_for(class: TemplateClass) -> &'static [&'static str] {
    match class {
        // ── NFT lane ───────────────────────────────────────────────
        c if c == SINGH_SABI => &["initial_energy", "floor_pct", "half_life"],
        c if c == SINGH_MIGRANT => &[
            "initial_energy",
            "base_half_life",
            "resting_threshold_epochs",
        ],
        c if c == SINGH_RESONANCE => &[
            "initial_energy",
            "base_half_life",
            "saturation",
            "max_scale_bp",
        ],
        c if c == SINGH_POSTHUMA => &[
            "half_life",
            "initial_visible_energy",
            "m_threshold",
            "n_committee",
        ],
        c if c == MAYFLY => &["initial_energy", "half_life"],
        // ── Marketplace lane ──────────────────────────────────────
        c if c == SDDC_AUCTION => &["ceiling", "floor", "lot_lambda", "duration_epochs"],
        c if c == SFSV_VAULT => &[
            "future_self",
            "predicate_type",
            "release_param",
            "deposit_amount",
        ],
        c if c == SHLM_BOUNTY => &["salary_cap", "min_freshness", "min_level"],
        c if c == SCL_LEASE => &["verb", "object", "duration_epochs"],
        c if c == SAP_AQ => &[
            "initial_value",
            "half_life",
            "max_aq_per_window",
            "window_epochs",
        ],
        c if c == REFRESH_MARKET_NAMESPACE => &["id_hex", "capacity", "base_rent"],
        c if c == DECAY_ACCESS_PASS => &["energy", "half_life", "validity_floor"],
        c if c == DEADMAN_SWITCH => &["initial_energy", "refresh_window"],
        c if c == SUBSCRIPTION_SERVICE => &[
            "initial_energy",
            "half_life",
            "period_amount",
            "period_length",
        ],
        // ── Wallet UX lane ────────────────────────────────────────
        c if c == SINGH_TRIAGE_CONTRACT => &["horizon_today", "horizon_tomorrow", "horizon_week"],
        c if c == SINGH_HEARTBEAT_PULSE => &[
            "healthy_bpm",
            "alarmed_bpm",
            "amber_threshold_bp",
            "red_threshold_bp",
        ],
        c if c == SINGH_LINEAGE_POLICY => &["ladder"],
        // ── Consumer lane ─────────────────────────────────────────
        c if c == CHILDKEY_LETTER => &[
            "unlock_age_years",
            "epochs_per_year",
            "m_threshold",
            "n_committee",
        ],
        c if c == MNEMOCHAIN_CARD => &["initial_energy", "initial_stability"],
        c if c == WITNESSFIT_STREAK => &["half_life", "boost_bp"],
        // ── Cultural lane ─────────────────────────────────────────
        c if c == GALLERY_FORGETS => &["opened_at_epoch"],
        // ── Paradigm lane (no runtime params; validators check the
        //     fragment string) ──────────────────────────────────────
        c if c == SGB_TYPE_SYSTEM => &["fragment"],
        c if c == SBAV_RUNTIME => &["reg_count"],
        c if c == SSM_GAME_SEMANTICS => &["fragment"],
        c if c == BELL_ORACLE => &["energy", "half_life", "threshold_milli", "max_age_epochs"],
        // ── Governance lane ───────────────────────────────────────
        c if c == MORTAL_DAO => &[
            "energy",
            "half_life",
            "freshness_window",
            "proposal_cap",
            "voting_window",
        ],
        // Unrecognised — empty (forward-compat).
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_template_has_required_keys() {
        // Every class in the catalogue should have at least one
        // required key (paradigm crates have `fragment` even though
        // they accept any value). Anti-regression: a new template
        // landing without a required-keys row would have an empty
        // table — the catalogue test below catches it.
        let cat = evaporchain_app_templates::catalogue();
        for d in &cat {
            let keys = required_keys_for(d.class);
            assert!(
                !keys.is_empty(),
                "template {:#010x} ({}) has no required keys",
                d.class.0,
                d.brand
            );
        }
    }

    #[test]
    fn unregistered_class_returns_empty() {
        let unknown = TemplateClass(0x9999_9999);
        assert!(required_keys_for(unknown).is_empty());
        // In-range but unregistered (forward-compat slot).
        let v2_slot = TemplateClass(0x0001_0700);
        assert!(required_keys_for(v2_slot).is_empty());
    }

    #[test]
    fn nft_lane_required_keys_are_concrete() {
        // Spot-check: each NFT primitive's required keys cover the
        // doctrine-meaningful inputs.
        assert!(required_keys_for(SINGH_SABI).contains(&"floor_pct"));
        assert!(required_keys_for(SINGH_MIGRANT).contains(&"resting_threshold_epochs"));
        assert!(required_keys_for(SINGH_RESONANCE).contains(&"max_scale_bp"));
        assert!(required_keys_for(SINGH_POSTHUMA).contains(&"m_threshold"));
        assert!(required_keys_for(MAYFLY).contains(&"half_life"));
    }
}
