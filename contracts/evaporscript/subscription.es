// Subscription — fifth pilot EvaporScript contract.
//
// Doctrine moment: every other chain needs an off-chain reaper to
// detect non-payment and cancel a subscription. EvaporChain doesn't.
// `pay()` refreshes the contract (boosts its energy via the runtime
// hooks); skipping payments lets the contract evaporate naturally;
// `on_evaporate` flips `lapsed = true`. No reaper, no
// who-watches-the-watcher.
//
// The deployer is the subscriber (`caller == owner` at deploy). After
// `set_terms`, both subscriber and provider may unilaterally cancel.
// Cancellation is one-shot and blocks future pay() calls.

contract Subscription {
    state {
        subscriber: address
        provider: address
        period_amount: u64 = 0
        period_length: u64 = 0
        sealed: bool = false

        paid_periods: u64 = 0
        cumulative_paid: u64 = 0
        last_payment_epoch: u64 = 0

        cancelled: bool = false
        cancelled_at_epoch: u64 = 0
        cancelled_by: address

        lapsed: bool = false
        lapsed_at_epoch: u64 = 0
    }

    // Arm the subscription exactly once. Only the subscriber (deployer)
    // can call. Records the provider and per-period terms.
    fn set_terms(provider_addr: address, amount: u64, period: u64) {
        require(caller == owner, "only subscriber can set terms")
        require(self.sealed == false, "terms already set")
        require(amount > 0, "amount must be positive")
        require(period > 0, "period must be positive")
        self.subscriber = owner
        self.provider = provider_addr
        self.period_amount = amount
        self.period_length = period
        self.sealed = true
        emit("subscription armed")
    }

    // Pay one period. Only the subscriber can pay. Bumps counters and
    // returns the amount paid so the coordinator knows how much to
    // transfer off-chain.
    //
    // SUB-1 (audit 2026-05-17): no enforcement that `epoch - last_payment_epoch
    // >= period_length`. Multiple calls in the same epoch all succeed and each
    // increments paid_periods / cumulative_paid. This is intentional — the
    // EvaporChain subscription model works by energy-decay, not periodic
    // gate-keeping: a subscriber who pays keeps the contract's energy alive;
    // over-paying simply boosts energy further at their own cost. The off-chain
    // coordinator is responsible for crediting only one payment per period.
    fn pay() -> u64 {
        require(self.sealed == true, "terms not yet set")
        require(self.cancelled == false, "subscription cancelled")
        require(caller == owner, "only subscriber can pay")
        self.paid_periods += 1
        self.cumulative_paid += self.period_amount
        self.last_payment_epoch = epoch
        emit("subscription payment")
        return self.period_amount
    }

    // Either subscriber or provider can cancel. One-shot: subsequent
    // cancels revert. The caller is recorded for audit.
    fn cancel() {
        require(self.sealed == true, "terms not yet set")
        require(self.cancelled == false, "already cancelled")
        require(
            caller == self.subscriber || caller == self.provider,
            "not authorized to cancel"
        )
        self.cancelled = true
        self.cancelled_at_epoch = epoch
        self.cancelled_by = caller
        emit("subscription cancelled")
    }

    fn provider_of() -> address {
        return self.provider
    }

    fn subscriber_of() -> address {
        return self.subscriber
    }

    fn amount_per_period() -> u64 {
        return self.period_amount
    }

    fn period_length() -> u64 {
        return self.period_length
    }

    fn periods_paid() -> u64 {
        return self.paid_periods
    }

    fn total_paid() -> u64 {
        return self.cumulative_paid
    }

    fn last_payment() -> u64 {
        return self.last_payment_epoch
    }

    // Active iff sealed AND not cancelled AND not lapsed.
    fn is_active() -> bool {
        if self.sealed == false {
            return false
        }
        if self.cancelled == true {
            return false
        }
        if self.lapsed == true {
            return false
        }
        return true
    }

    on_grace() {
        emit("subscription energy low — pay to keep alive")
    }

    on_refresh() {
        emit("subscription refreshed")
    }

    // If the contract evaporates while still uncancelled, mark the
    // subscription lapsed. Cancelled subscriptions ended cleanly and
    // do not relapse.
    on_evaporate() {
        if self.cancelled == false {
            self.lapsed = true
            self.lapsed_at_epoch = epoch
            emit("subscription evaporated — lapse ends coverage")
        }
    }
}
