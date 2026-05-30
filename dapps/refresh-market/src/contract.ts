// Single source of truth: `contracts/evaporscript/refresh_market.es`.
// Inline copy the dApp ships with — keep it byte-identical to the
// .es file. The Rust pilot test `evaporchain-script` →
// `mod refresh_market_pilot` is the regression barrier proving this
// exact source compiles, runs through the VM, and is totality-clean.

export const REFRESH_MARKET_SOURCE = `contract RefreshMarket {
    state {
        capacity: u64 = 0
        base_rent: u64 = 0
        eviction_window: u64 = 10
        sealed: bool = false

        used_now: u64 = 0
        holders: map[address -> u64]
        last_refresh: map[address -> u64]

        total_claims: u64 = 0
        total_evictions: u64 = 0
        total_releases: u64 = 0
    }

    fn arm(cap: u64, base: u64, eviction: u64) {
        require(caller == owner, "only operator arms")
        require(self.sealed == false, "already armed")
        require(cap > 0, "capacity must be positive")
        require(eviction > 0, "eviction window must be positive")
        self.capacity = cap
        self.base_rent = base
        self.eviction_window = eviction
        self.sealed = true
        emit("refresh market armed")
    }

    fn claim_slot() {
        require(self.sealed == true, "market not armed")
        require(self.holders[caller] == 0, "caller already holds a slot")
        require(self.used_now < self.capacity, "namespace at capacity")
        self.holders[caller] = 1
        self.last_refresh[caller] = epoch + 1
        self.used_now += 1
        self.total_claims += 1
        emit("slot claimed")
    }

    fn refresh_slot() {
        require(self.holders[caller] == 1, "caller does not hold a slot")
        self.last_refresh[caller] = epoch + 1
        emit("slot refreshed")
    }

    fn release_slot() {
        require(self.holders[caller] == 1, "caller does not hold a slot")
        self.holders[caller] = 0
        self.last_refresh[caller] = 0
        self.used_now -= 1
        self.total_releases += 1
        emit("slot released")
    }

    fn evict(who: address) {
        require(self.sealed == true, "market not armed")
        require(self.holders[who] == 1, "target does not hold a slot")
        require(
            epoch >= self.last_refresh[who] + self.eviction_window,
            "holder still within eviction window"
        )
        self.holders[who] = 0
        self.last_refresh[who] = 0
        self.used_now -= 1
        self.total_evictions += 1
        emit("slot evicted")
    }

    fn current_rate() -> u64 {
        if self.sealed == false {
            return 0
        }
        return self.base_rent * (self.used_now + 1) * (self.used_now + 1) / (self.capacity * self.capacity)
    }

    fn rate_at_used(used: u64) -> u64 {
        if self.sealed == false {
            return 0
        }
        return self.base_rent * (used + 1) * (used + 1) / (self.capacity * self.capacity)
    }

    fn capacity_now() -> u64 {
        return self.capacity
    }

    fn base_now() -> u64 {
        return self.base_rent
    }

    fn used() -> u64 {
        return self.used_now
    }

    fn slots_remaining() -> u64 {
        if self.sealed == false {
            return 0
        }
        return self.capacity - self.used_now
    }

    fn is_holder(who: address) -> bool {
        return self.holders[who] == 1
    }

    fn last_refresh_now(who: address) -> u64 {
        return self.last_refresh[who]
    }

    fn is_evictable(who: address) -> bool {
        if self.holders[who] == 0 {
            return false
        }
        if epoch >= self.last_refresh[who] + self.eviction_window {
            return true
        }
        return false
    }

    fn claims_total() -> u64 {
        return self.total_claims
    }

    fn evictions_total() -> u64 {
        return self.total_evictions
    }

    fn releases_total() -> u64 {
        return self.total_releases
    }

    fn is_armed() -> bool {
        return self.sealed
    }

    on_grace() {
        emit("refresh market energy low — refresh to keep accepting claims")
    }

    on_refresh() {
        emit("refresh market refreshed")
    }

    on_evaporate() {
        emit("refresh market evaporated — namespace closed, no new claims")
    }
}
`;
