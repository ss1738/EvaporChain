// Subscription — tenth pilot. Stdlib contract #7: turns the recurring-
// payment / SaaS-subscription primitive into a decay-native one.
//
// Decay-thesis hook: every subscription on every chain today (Sablier
// streams, Superfluid, RecurringPayments contracts) requires an off-
// chain reaper to detect non-payment and cancel the subscription. The
// reaper is itself an attack surface: who runs it, who pays its gas,
// what happens if it stops? Subscription removes the reaper entirely:
// the contract's own energy decays. Each payment refreshes the contract
// (extends its life). Skip a payment and the contract evaporates by
// physics, on_evaporate fires, and the subscription cleans itself up.
// No off-chain reaper, no who-watches-the-watcher problem.
//
// Lifecycle:
//
//   1. Deploy → `set_terms(provider, amount, period)` defines the
//      subscription. Sealed-once. The deployer is the subscriber.
//   2. `pay()` records a payment. The chain runtime is expected to
//      refresh the contract's energy by `period`-equivalent epochs
//      on each successful pay (the call refreshes the contract). The
//      provider's withdrawal flow is coordinator-driven; this contract
//      is the on-chain ledger.
//   3. `cancel()` lets either party end the subscription early.
//   4. on_evaporate: subscription naturally ends — the lapse signal
//      tells the provider to stop service for this contract id.
//
// Auth model:
//   - `set_terms`:  caller == owner (subscriber).
//   - `pay`:        caller == owner (subscriber).
//   - `cancel`:     caller == owner OR caller == self.provider.

contract Subscription {
    state {
        subscriber: address
        provider: address

        // Per-period charge + period length (in epochs). Both fixed
        // at setup; changing either requires a new contract.
        period_amount: u64 = 0
        period_epochs: u64 = 0

        // Payment ledger. paid_periods is monotonic — counts every
        // payment ever made. cumulative_paid is the sum of amounts
        // (constant period_amount × paid_periods, but tracked
        // separately for audit clarity if period_amount semantics
        // ever evolve).
        paid_periods: u64 = 0
        cumulative_paid: u64 = 0
        last_payment_epoch: u64 = 0

        sealed: bool = false
        cancelled: bool = false
        cancelled_by: address
        cancelled_at_epoch: u64 = 0
        lapsed: bool = false
    }

    // Phase 1: define the subscription. Provider gets paid, subscriber
    // pays. amount × period_epochs are the price-per-period and
    // billing cadence respectively.
    fn set_terms(
        provider_addr: address,
        amount: u64,
        period: u64
    ) {
        require(caller == owner, "only subscriber can set terms")
        require(self.sealed == false, "terms already set")
        require(amount > 0, "period_amount must be positive")
        require(period > 0, "period_epochs must be positive")
        self.subscriber = owner
        self.provider = provider_addr
        self.period_amount = amount
        self.period_epochs = period
        self.sealed = true
        emit("subscription terms set")
    }

    // Subscriber pays a period. Each call refreshes the contract via
    // the chain runtime (call_contract refreshes targets). The result
    // is that paying = staying alive; not paying = evaporating.
    fn pay() -> u64 {
        require(self.sealed == true, "terms not set")
        require(self.cancelled == false, "subscription cancelled")
        require(caller == self.subscriber, "only subscriber can pay")
        self.paid_periods += 1
        self.cumulative_paid += self.period_amount
        self.last_payment_epoch = epoch
        emit("payment recorded")
        return self.period_amount
    }

    // Either party can cancel. The cancelled flag short-circuits
    // future pay() calls; on_evaporate still fires when the contract's
    // remaining energy runs out, but with `cancelled == true` rather
    // than `lapsed == true`.
    fn cancel() {
        require(self.sealed == true, "terms not set")
        require(self.cancelled == false, "already cancelled")
        require(
            caller == self.subscriber || caller == self.provider,
            "not authorized"
        )
        self.cancelled = true
        self.cancelled_by = caller
        self.cancelled_at_epoch = epoch
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
        return self.period_epochs
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

    fn is_active() -> bool {
        if self.sealed == false {
            return false
        }
        if self.cancelled == true {
            return false
        }
        return self.lapsed == false
    }

    on_grace() {
        if self.cancelled == false {
            emit("subscription energy low — pay before evaporation or service lapses")
        }
    }

    on_refresh() {
        emit("subscription refreshed")
    }

    // Doctrine moment: when energy runs out, the subscription lapses
    // naturally. The provider's coordinator subscribes to evaporation
    // events; receiving one for this contract id is the unambiguous
    // signal to stop service. No reaper needed.
    on_evaporate() {
        if self.cancelled == false {
            self.lapsed = true
            emit("subscription evaporated — service ends")
        }
    }
}
