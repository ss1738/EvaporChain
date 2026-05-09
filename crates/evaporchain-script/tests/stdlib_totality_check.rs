//! Totality regression for the seed-15 EvaporScript stdlib.
//!
//! Item B (V1) sister test to `stdlib_parse_check.rs`. Where parse-check
//! confirms the contracts compile, this confirms they would *also* be
//! accepted under total-programming mode — i.e. zero `while` loops
//! anywhere in their methods or lifecycle hooks.
//!
//! When total mode is wired behind a governance flag (`script_vm_mode =
//! "total"`, V1.5), this test is the proof that the entire stdlib survives
//! the flip without porting work.

use evaporchain_script::{parser, totality};

fn assert_total(name: &str, source: &str) {
    let c = parser::parse(source)
        .unwrap_or_else(|e| panic!("{name}: parse failed — {e:?}"));
    totality::check_total_contract(&c)
        .unwrap_or_else(|e| panic!("{name}: not total — {e:?}"));
}

#[test]
fn pilot_mortal_message_is_total() {
    assert_total(
        "MortalMessage",
        include_str!("../../../contracts/evaporscript/mortal_message.es"),
    );
}

#[test]
fn pilot_mortal_nft_is_total() {
    assert_total(
        "MortalNft",
        include_str!("../../../contracts/evaporscript/mortal_nft.es"),
    );
}

#[test]
fn pilot_energy_pool_is_total() {
    assert_total(
        "EnergyPool",
        include_str!("../../../contracts/evaporscript/energy_pool.es"),
    );
}

#[test]
fn stdlib_payment_split_is_total() {
    assert_total(
        "PaymentSplit",
        include_str!("../../../contracts/evaporscript/payment_split.es"),
    );
}

#[test]
fn stdlib_sealed_bid_auction_is_total() {
    assert_total(
        "SealedBidAuction",
        include_str!("../../../contracts/evaporscript/sealed_bid_auction.es"),
    );
}

#[test]
fn stdlib_vesting_schedule_is_total() {
    assert_total(
        "VestingSchedule",
        include_str!("../../../contracts/evaporscript/vesting_schedule.es"),
    );
}

#[test]
fn stdlib_time_lock_is_total() {
    assert_total(
        "TimeLock",
        include_str!("../../../contracts/evaporscript/time_lock.es"),
    );
}

#[test]
fn stdlib_attestation_is_total() {
    assert_total(
        "Attestation",
        include_str!("../../../contracts/evaporscript/attestation.es"),
    );
}

#[test]
fn stdlib_oracle_feed_is_total() {
    assert_total(
        "OracleFeed",
        include_str!("../../../contracts/evaporscript/oracle_feed.es"),
    );
}

#[test]
fn stdlib_subscription_is_total() {
    assert_total(
        "Subscription",
        include_str!("../../../contracts/evaporscript/subscription.es"),
    );
}

#[test]
fn stdlib_multisig_is_total() {
    assert_total(
        "Multisig",
        include_str!("../../../contracts/evaporscript/multisig.es"),
    );
}

#[test]
fn stdlib_lottery_is_total() {
    assert_total(
        "Lottery",
        include_str!("../../../contracts/evaporscript/lottery.es"),
    );
}

#[test]
fn stdlib_bounty_is_total() {
    assert_total(
        "Bounty",
        include_str!("../../../contracts/evaporscript/bounty.es"),
    );
}

#[test]
fn stdlib_dead_man_switch_is_total() {
    assert_total(
        "DeadManSwitch",
        include_str!("../../../contracts/evaporscript/dead_man_switch.es"),
    );
}

#[test]
fn stdlib_energy_marketplace_is_total() {
    assert_total(
        "EnergyMarketplace",
        include_str!("../../../contracts/evaporscript/energy_marketplace.es"),
    );
}
