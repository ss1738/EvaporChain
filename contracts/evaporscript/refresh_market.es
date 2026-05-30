// RefreshMarket — reference contract for one namespace's AMM rent market.
//
// Per `research/INVENTION_STACK.md` §4.1 row 7 (and the catalogue
// entry at `evaporchain-app-templates` REFRESH_MARKET_NAMESPACE,
// 0x0001_0106): each namespace publishes a per-epoch rent rate that
// is quadratic in utilisation. Empty namespaces pay near-zero rent;
// fully-saturated namespaces pay near `base_rent`; over-the-cap is
// structurally rejected (claims revert when `used_now >= capacity`).
//
// Rate formula:
//   rate = base_rent * (used_now + 1)^2 / capacity^2
//
// One contract = one namespace. The deployer is the operator (owner)
// who configures `capacity / base_rent / eviction_window` once via
// `arm()`. After arming, anyone can `claim_slot()` while there's
// capacity; holders `refresh_slot()` to reset their eviction clock;
// stale holders can be evicted by anyone via `evict(who)`.
//
// Payment is OUT OF SCOPE for the reference contract — the
// substrate crate `evaporchain-refresh-market` handles the
// actual token flows. This contract publishes the rate + tracks
// occupancy + enforces eviction.
//
// Like every reference contract in the catalogue, this contract's
// OWN energy is the namespace's lifespan. If the operator lets
// the contract evaporate, the namespace closes — no further claims.

contract RefreshMarket {
    state {
        // ── operator-configured policy ─────────────────────────────
        capacity: u64 = 0
        base_rent: u64 = 0
        // Epochs of grace before a non-refreshing holder is evictable.
        eviction_window: u64 = 10
        sealed: bool = false

        // ── per-namespace occupancy ────────────────────────────────
        used_now: u64 = 0
        // holder -> 1 if currently holds a slot, 0 otherwise.
        holders: map[address -> u64]
        // holder -> last-refresh-epoch + 1 (the +1 sentinel lets us
        // distinguish "claimed at epoch 0" from "never claimed";
        // see mortal_dao.es for the same idiom).
        last_refresh: map[address -> u64]

        // ── lifetime counters ──────────────────────────────────────
        total_claims: u64 = 0
        total_evictions: u64 = 0
        total_releases: u64 = 0
    }

    // Owner-only, one-shot setup. Configures the AMM curve + the
    // eviction grace period.
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

    // Anyone can claim a slot while there's capacity. Caller may
    // only hold one slot at a time per contract — duplicate claims
    // revert (release first, or refresh if you just want to extend).
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

    // Holder resets their eviction clock. The natural pairing with
    // an off-chain rent payment — pay the substrate-crate rent for
    // the next window, then call this to record the refresh on-chain.
    fn refresh_slot() {
        require(self.holders[caller] == 1, "caller does not hold a slot")
        self.last_refresh[caller] = epoch + 1
        emit("slot refreshed")
    }

    // Holder voluntarily releases their slot.
    fn release_slot() {
        require(self.holders[caller] == 1, "caller does not hold a slot")
        self.holders[caller] = 0
        self.last_refresh[caller] = 0
        self.used_now -= 1
        self.total_releases += 1
        emit("slot released")
    }

    // Anyone can evict a stale holder — incentivising third parties
    // to reclaim capacity. The eviction gate is `epoch >= last_refresh
    // + eviction_window` (recall last_refresh is shifted by +1, so
    // this corresponds to "epochs-since-refresh > eviction_window"
    // in unshifted terms).
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

    // The doctrine — quadratic AMM rent. The formula is:
    //   rate = base_rent * (used + 1)^2 / capacity^2
    // Expanded to multiplications only (no `**` operator in V1
    // EvaporScript). Returns 0 if the market isn't armed (capacity 0
    // would otherwise divide by zero; we check sealed first so the
    // capacity-square denominator is always positive).
    fn current_rate() -> u64 {
        if self.sealed == false {
            return 0
        }
        return self.base_rent * (self.used_now + 1) * (self.used_now + 1) / (self.capacity * self.capacity)
    }

    // What the rate WOULD be after `delta` more slots get claimed —
    // useful for off-chain UIs that want to surface "claim this and
    // your rate becomes X" projections.
    fn rate_at_used(used: u64) -> u64 {
        if self.sealed == false {
            return 0
        }
        return self.base_rent * (used + 1) * (used + 1) / (self.capacity * self.capacity)
    }

    // ── Views ──────────────────────────────────────────────────────
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

    // Returns 0 if `who` isn't a holder; otherwise the +1-shifted
    // last-refresh epoch (subtract 1 for the actual epoch).
    fn last_refresh_now(who: address) -> u64 {
        return self.last_refresh[who]
    }

    // True iff `who` holds a slot AND is past their eviction window.
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
