//! `TemplateClass` — the stable u32 id space for app templates.
//!
//! The numeric range `0x0001_0000..=0x0001_FFFF` (65 536 ids) is
//! reserved for application templates registered by this crate.
//! Substrate parameter ids and oracle ids live elsewhere; this
//! reservation makes collisions structurally impossible.

use serde::{Deserialize, Serialize};

/// Inclusive low end of the app-template id range.
pub const APP_TEMPLATE_RANGE_START: u32 = 0x0001_0000;
/// Inclusive high end of the app-template id range.
pub const APP_TEMPLATE_RANGE_END: u32 = 0x0001_FFFF;

/// Stable numeric id for one app template. The dApp layer addresses
/// templates by class id, not by Rust type — adding a new primitive
/// doesn't recompile consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct TemplateClass(pub u32);

impl TemplateClass {
    /// True iff the id sits inside the reserved app-template range.
    pub fn is_in_app_range(&self) -> bool {
        (APP_TEMPLATE_RANGE_START..=APP_TEMPLATE_RANGE_END).contains(&self.0)
    }
}

// ─── The registered classes ─────────────────────────────────────────
//
// Class ids are assigned by lane:
//   0x0001_0000..=0x0001_00FF — NFT lane
//   0x0001_0100..=0x0001_01FF — Marketplace lane
//   0x0001_0200..=0x0001_02FF — Wallet UX lane
//   0x0001_0300..=0x0001_03FF — Consumer launch lane
//   0x0001_0400..=0x0001_04FF — Cultural launch wedge
//   0x0001_0500..=0x0001_05FF — Smart-contract paradigm
//   0x0001_0600..=0x0001_06FF — Governance lane

// NFT lane
pub const SINGH_SABI: TemplateClass = TemplateClass(0x0001_0001);
pub const SINGH_MIGRANT: TemplateClass = TemplateClass(0x0001_0002);
pub const SINGH_RESONANCE: TemplateClass = TemplateClass(0x0001_0003);
pub const SINGH_POSTHUMA: TemplateClass = TemplateClass(0x0001_0004);
pub const MAYFLY: TemplateClass = TemplateClass(0x0001_0005);
/// Mortal NFT (general) — the headline "decaying NFT" doctrine
/// claim in its full general-purpose form (transferable, named,
/// collection-aware, with metadata URI). Distinct from MAYFLY (the
/// doctrine-purest short-life mortal NFT) — Mortal NFT carries the
/// holder lifecycle, transfer count, and collection identity that a
/// marketplace dApp needs. The contract's own energy IS the NFT's
/// lifespan; on evaporation it becomes a chain-level Ghost
/// recoverable via the standard ghost-recovery flow. Reference
/// contract: `contracts/evaporscript/mortal_nft.es`. Verified cargo
/// pilot: `crates/evaporchain-script/tests/mortal_nft_pilot.rs`.
pub const MORTAL_NFT_GENERAL: TemplateClass = TemplateClass(0x0001_0006);

// Marketplace lane
pub const SDDC_AUCTION: TemplateClass = TemplateClass(0x0001_0101);
pub const SFSV_VAULT: TemplateClass = TemplateClass(0x0001_0102);
pub const SHLM_BOUNTY: TemplateClass = TemplateClass(0x0001_0103);
pub const SCL_LEASE: TemplateClass = TemplateClass(0x0001_0104);
pub const SAP_AQ: TemplateClass = TemplateClass(0x0001_0105);
/// Refresh-Market — AMM-priced rent per state object. Per
/// `research/INVENTION_STACK.md` §4.1 row 7: the chain's primary
/// economic activity. Each namespace declares max-slots capacity +
/// base coefficient; per-epoch rent = `base × (used + 1)² / capacity²`.
pub const REFRESH_MARKET_NAMESPACE: TemplateClass = TemplateClass(0x0001_0106);
/// Decay Access Pass — on-chain decaying credential. The pass's strength
/// is the contract's own energy (decayed via `energy_at_epoch`); valid
/// only while strength stays at or above a floor, so it evaporates unless
/// the issuer refreshes. Deployable reference contract at
/// `contracts/evaporscript/decay_access_pass.es`; substrate lib is
/// `evaporchain-decay-credential`.
pub const DECAY_ACCESS_PASS: TemplateClass = TemplateClass(0x0001_0107);
/// Dead-Man Switch — secret-release contract where the chain's own
/// epoch advancement IS the trigger. Holder refreshes within
/// `refresh_window` epochs; if they miss, anyone may call
/// `release_dead` to publish the committed payload. No keeper service
/// needed — the require() guard `epoch >= last_refresh + window`
/// opens to the world the moment the chain catches up. Doctrinally
/// the marketplace-lane peer of SFSV (future-self vault) +
/// SDDC (decay-Dutch auction): commit-and-reveal escrow whose
/// liquidation closer is the chain runtime. Reference contract:
/// `contracts/evaporscript/deadman_switch.es`. Verified cargo pilot:
/// `crates/evaporchain-script/tests/deadman_switch_pilot.rs` (9/9).
pub const DEADMAN_SWITCH: TemplateClass = TemplateClass(0x0001_0108);
/// Subscription — recurring payment whose chain-as-keeper doctrine
/// applies to a payment cadence. `pay()` IS the keep-alive: each
/// call refreshes the contract's energy via the runtime hook;
/// missing payments lets the contract evaporate; `on_evaporate`
/// flips `lapsed = true`. No off-chain reaper needed to detect
/// non-payment and cancel — the same chain-as-keeper claim as
/// DEADMAN_SWITCH, in a different surface (recurring payment instead
/// of secret release). Either subscriber or provider may cancel;
/// cancellation is one-shot, blocks future pay() calls, and is
/// distinct from lapsed-by-evaporation. Reference contract:
/// `contracts/evaporscript/subscription.es`. Verified cargo pilot:
/// `crates/evaporchain-script/tests/subscription_pilot.rs`.
pub const SUBSCRIPTION_SERVICE: TemplateClass = TemplateClass(0x0001_0109);
/// Open-Call Bounty — task bounty whose un-accepted-on-evaporation
/// path refunds the poster, no off-chain liquidator required. Poster
/// (deployer) sets task + reward; any address may submit a solution;
/// poster accepts a winner from the submission ledger; winner claims
/// the reward exactly once. Cancellation is blocked the moment any
/// hunter submits (no rug-pull on done work). On evaporation without
/// an accepted winner, `refunded = true` and the off-chain coordinator
/// returns funds — the chain-as-keeper escrow doctrine in the
/// task-bounty surface. Distinct from `SHLM_BOUNTY` (Skill Half-Life
/// Bounty), which is the skill-credential decay market. Reference
/// contract: `contracts/evaporscript/bounty.es`. Verified cargo
/// pilot: `crates/evaporchain-script/tests/bounty_pilot.rs`.
pub const OPEN_BOUNTY: TemplateClass = TemplateClass(0x0001_010A);
/// Time Lock — locks `amount` for `beneficiary` until `unlock_epoch`;
/// grantor may revoke before unlock; beneficiary may claim once
/// unlock reached; on_evaporate flips `forfeit_signaled = true` if
/// never claimed and never revoked, so the off-chain coordinator
/// returns the locked amount to the grantor. The runtime is the
/// deadline enforcer — no off-chain reaper polls the unlock epoch.
/// Doctrinally the fourth chain-as-keeper Marketplace primitive
/// alongside DeadMan Switch + Subscription + Open Bounty; surface is
/// time-locked vault (escrow over an epoch boundary). Reference
/// contract: `contracts/evaporscript/time_lock.es`. Verified cargo
/// pilot: `crates/evaporchain-script/tests/time_lock_pilot.rs`.
pub const TIME_LOCK: TemplateClass = TemplateClass(0x0001_010B);
/// Vesting Schedule — classic linear vest with cliff. Grantor locks
/// `grant` for `beneficiary` over `duration` epochs with a `cliff`;
/// vested amount rises linearly from 0 to total_grant between cliff
/// and duration. Doctrine claim: the post-vest claim window is
/// bounded by the contract's own energy — if the beneficiary stops
/// claiming and the contract evaporates, on_evaporate stamps
/// `vested_at_evaporate` and flips `forfeit_signaled` so the
/// off-chain coordinator returns the unclaimed remainder to the
/// grantor. VEST-1 (audit 2026-05-17): all five vest-math sites use
/// division-first arithmetic to avoid u64 overflow at large grants.
/// Reference contract: `contracts/evaporscript/vesting_schedule.es`.
/// Verified cargo pilot:
/// `crates/evaporchain-script/tests/vesting_schedule_pilot.rs`.
pub const VESTING_SCHEDULE_CLASS: TemplateClass = TemplateClass(0x0001_010C);
/// Payment Split — pull-payment revenue splitter with basis-point
/// shares. Deployer adds N recipients with bps shares that must sum
/// to exactly 10_000 (100.00%), then seals. Any address deposits;
/// recipients pull cumulative share on demand via the SPLIT-1
/// division-first formula (audit 2026-05-17 hardening; avoids u64
/// overflow at total_deposited > u64::MAX/bps). Per-recipient
/// `claimed[]` tracking makes the math idempotent — re-claim with no
/// new deposit reverts. Doctrine claim: at evaporation, unclaimed
/// amounts forfeit; on_evaporate stamps `unclaimed_at_evaporate` +
/// flips `forfeit_signaled` so the coordinator returns the residue
/// to the deployer. No off-chain recovery sweep — the runtime is the
/// closer. Reference contract: `contracts/evaporscript/payment_split.es`.
/// Verified cargo pilot:
/// `crates/evaporchain-script/tests/payment_split_pilot.rs`.
pub const PAYMENT_SPLIT_CLASS: TemplateClass = TemplateClass(0x0001_010D);

// Wallet UX lane (these are *contract-deployable* knobs the wallet
// can attach; the wallet UI itself is off-chain frontend code)
pub const SINGH_TRIAGE_CONTRACT: TemplateClass = TemplateClass(0x0001_0201);
pub const SINGH_HEARTBEAT_PULSE: TemplateClass = TemplateClass(0x0001_0202);
pub const SINGH_LINEAGE_POLICY: TemplateClass = TemplateClass(0x0001_0203);
/// Multisig proposal — one contract, one decision. Doctrine inversion
/// of Gnosis-Safe-style proposal-map architectures: the contract IS
/// the proposal, multiple decisions = multiple contracts evaporating
/// independently. Owner registers signers + threshold pre-propose,
/// seals with `propose(action)`; signers sign once each; anyone may
/// `execute()` once `signature_count >= threshold`. Unexecuted at
/// evaporation = `expired = true` (the decision lapsed; no follow-up
/// vote resurrects it). Reference contract:
/// `contracts/evaporscript/multisig.es`. Verified cargo pilot:
/// `crates/evaporchain-script/tests/multisig_pilot.rs`.
pub const MULTISIG_PROPOSAL: TemplateClass = TemplateClass(0x0001_0204);

// Consumer launch lane
pub const CHILDKEY_LETTER: TemplateClass = TemplateClass(0x0001_0301);
pub const MNEMOCHAIN_CARD: TemplateClass = TemplateClass(0x0001_0302);
pub const WITNESSFIT_STREAK: TemplateClass = TemplateClass(0x0001_0303);
/// Mortal Message — the canonical EvaporScript pilot contract.
/// Self-destructing message where the contract's own energy IS the
/// message lifespan; sender + recipient may `read()` while alive,
/// `on_refresh` boosts the boost_count, `on_evaporate` ends the
/// message. Per project CLAUDE.md "Two unifying invariants" #2, this
/// is the *reference pilot* every other EvaporScript contract is
/// shaped from. The 24-entry registry historically excluded it because
/// it was framed as a stdlib pilot rather than a deployable consumer
/// primitive; this entry surfaces it as a first-class catalogue slot.
/// Reference contract: `contracts/evaporscript/mortal_message.es`.
/// Verified cargo pilot: `crates/evaporchain-script/tests/mortal_message_pilot.rs`.
pub const MORTAL_MESSAGE_PILOT: TemplateClass = TemplateClass(0x0001_0304);

// Cultural launch wedge
pub const GALLERY_FORGETS: TemplateClass = TemplateClass(0x0001_0401);

// Smart-contract paradigm primitives — paradigm-grade type systems.
// Listed for completeness; deploying them as templates means
// declaring a contract using SGB / SBAV / SSM as its type system,
// not deploying a stand-alone instance.
pub const SGB_TYPE_SYSTEM: TemplateClass = TemplateClass(0x0001_0501);
pub const SBAV_RUNTIME: TemplateClass = TemplateClass(0x0001_0502);
pub const SSM_GAME_SEMANTICS: TemplateClass = TemplateClass(0x0001_0503);
/// Bell-Oracle — on-chain consumer of the per-block CHSH S-value
/// beacon. Structurally rejects readings at or below the
/// local-realism floor (2000 milli-units = S = 2.0) so the stored
/// state is *certifiably* quantum-grade. Downstream contracts gate
/// quantum-randomness-requiring actions on `is_certified_now()`.
/// Reference contract: `contracts/evaporscript/bell_oracle.es`.
pub const BELL_ORACLE: TemplateClass = TemplateClass(0x0001_0504);

// Governance lane — on-chain coordination primitives that compose
// the decay substrate (credential / rate-limit / reputation / quorum)
// into runnable contracts.
/// Mortal-DAO — single-instance governance contract whose lifecycle
/// rides the contract's own energy. Members refresh to stay active
/// (decay-credential); per-member proposal cap resets on refresh
/// (decay-rate-limit); vote weight = participations + 1
/// (decay-reputation); quorum gate tracks a running peak of engagement
/// (decay-quorum). Reference contract:
/// `contracts/evaporscript/mortal_dao.es`.
pub const MORTAL_DAO: TemplateClass = TemplateClass(0x0001_0601);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_classes_in_app_range() {
        for c in [
            SINGH_SABI,
            SINGH_MIGRANT,
            SINGH_RESONANCE,
            SINGH_POSTHUMA,
            MAYFLY,
            MORTAL_NFT_GENERAL,
            SDDC_AUCTION,
            SFSV_VAULT,
            SHLM_BOUNTY,
            SCL_LEASE,
            SAP_AQ,
            REFRESH_MARKET_NAMESPACE,
            DECAY_ACCESS_PASS,
            DEADMAN_SWITCH,
            SUBSCRIPTION_SERVICE,
            OPEN_BOUNTY,
            TIME_LOCK,
            VESTING_SCHEDULE_CLASS,
            PAYMENT_SPLIT_CLASS,
            SINGH_TRIAGE_CONTRACT,
            SINGH_HEARTBEAT_PULSE,
            SINGH_LINEAGE_POLICY,
            MULTISIG_PROPOSAL,
            CHILDKEY_LETTER,
            MNEMOCHAIN_CARD,
            WITNESSFIT_STREAK,
            MORTAL_MESSAGE_PILOT,
            GALLERY_FORGETS,
            SGB_TYPE_SYSTEM,
            SBAV_RUNTIME,
            SSM_GAME_SEMANTICS,
            BELL_ORACLE,
            MORTAL_DAO,
        ] {
            assert!(c.is_in_app_range(), "{:#010x} not in app range", c.0);
        }
    }

    #[test]
    fn ids_outside_range_are_rejected() {
        let tc = TemplateClass(0x0000_FFFF);
        assert!(!tc.is_in_app_range());
        let tc = TemplateClass(0x0002_0000);
        assert!(!tc.is_in_app_range());
    }

    #[test]
    fn lane_partitioning_holds() {
        // NFT lane: 0x0001_0001..0x0001_00FF
        assert!((0x0001_0001..=0x0001_00FF).contains(&SINGH_SABI.0));
        assert!((0x0001_0001..=0x0001_00FF).contains(&MAYFLY.0));
        assert!((0x0001_0001..=0x0001_00FF).contains(&MORTAL_NFT_GENERAL.0));
        // Marketplace lane: 0x0001_0100..0x0001_01FF
        assert!((0x0001_0100..=0x0001_01FF).contains(&SDDC_AUCTION.0));
        assert!((0x0001_0100..=0x0001_01FF).contains(&SAP_AQ.0));
        assert!((0x0001_0100..=0x0001_01FF).contains(&DEADMAN_SWITCH.0));
        assert!((0x0001_0100..=0x0001_01FF).contains(&SUBSCRIPTION_SERVICE.0));
        assert!((0x0001_0100..=0x0001_01FF).contains(&OPEN_BOUNTY.0));
        assert!((0x0001_0100..=0x0001_01FF).contains(&TIME_LOCK.0));
        assert!((0x0001_0100..=0x0001_01FF).contains(&VESTING_SCHEDULE_CLASS.0));
        assert!((0x0001_0100..=0x0001_01FF).contains(&PAYMENT_SPLIT_CLASS.0));
        // Wallet UX: 0x0001_0200..
        assert!((0x0001_0200..=0x0001_02FF).contains(&SINGH_LINEAGE_POLICY.0));
        assert!((0x0001_0200..=0x0001_02FF).contains(&MULTISIG_PROPOSAL.0));
        // Consumer: 0x0001_0300..
        assert!((0x0001_0300..=0x0001_03FF).contains(&CHILDKEY_LETTER.0));
        assert!((0x0001_0300..=0x0001_03FF).contains(&MORTAL_MESSAGE_PILOT.0));
        // Cultural: 0x0001_0400..
        assert!((0x0001_0400..=0x0001_04FF).contains(&GALLERY_FORGETS.0));
        // Paradigm: 0x0001_0500..
        assert!((0x0001_0500..=0x0001_05FF).contains(&SGB_TYPE_SYSTEM.0));
        assert!((0x0001_0500..=0x0001_05FF).contains(&BELL_ORACLE.0));
        // Governance: 0x0001_0600..
        assert!((0x0001_0600..=0x0001_06FF).contains(&MORTAL_DAO.0));
    }

    #[test]
    fn round_trip_serde() {
        let c = SINGH_SABI;
        let s = serde_json::to_string(&c).unwrap();
        let back: TemplateClass = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }
}
