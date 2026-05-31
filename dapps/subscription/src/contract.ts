// Single source of truth: `contracts/evaporscript/subscription.es`.
// Byte-stable inline copy. Mirrors the 5-pilot stdlib contract that
// also lives in `crates/evaporchain-script/tests/subscription_pilot.rs`.

export const SUBSCRIPTION_SOURCE = `contract Subscription {
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

    on_evaporate() {
        if self.cancelled == false {
            self.lapsed = true
            self.lapsed_at_epoch = epoch
            emit("subscription evaporated — lapse ends coverage")
        }
    }
}`;
