// EnergyMarketplace — fifteenth pilot. Stdlib contract #12 (final
// of the seed-12 set). The most reflexive contract in the stdlib:
// energy itself is the commodity being traded.
//
// Decay-thesis hook: every other chain treats compute / storage / gas
// as a one-shot purchase priced at tx-submission time. EvaporChain
// has a richer notion: every object has an *energy budget* that
// decays, and refresh extends it. EnergyMarketplace makes that budget
// itself tradable. Sellers list "I'll grant X energy units to your
// object for Y EVAP". Buyers shop, buy, and the chain runtime
// applies the granted energy as a refresh on the buyer's named
// target. This is the closest the chain gets to a meta-market —
// trading the substance of the chain's own physics.
//
// And the marketplace itself decays: listings expire when the
// marketplace contract evaporates. There is no eternal order book.
// Liquidity is mortal.
//
// Pilot-grammar simplification: one listing per seller (not multiple
// concurrent listings — which would require an array/list of
// listings). Sellers can update their listing in place. Each `buy`
// matches one listing; partial fills and order-book mechanics are
// outside the seed pilot.
//
// Lifecycle:
//
//   1. Deploy → `set_market(market_name)` configures. Sealed-once.
//      Open thereafter to any seller / buyer.
//   2. Sellers call `list(energy_units, price_per_unit)` to advertise.
//      Each address has at most one active listing.
//   3. Buyers call `buy(seller_addr, units)` to fill some or all of
//      the seller's listing. Coordinator handles the actual energy-
//      transfer + EVAP-payment.
//   4. Sellers call `cancel()` to retract their listing.
//   5. on_evaporate: marketplace closes, all listings void.
//
// Auth model:
//   - `set_market`:  caller == owner (market operator).
//   - `list`:        open (any seller).
//   - `buy`:         open (any buyer).
//   - `cancel`:      caller's own listing only.

contract EnergyMarketplace {
    state {
        market_operator: address
        market_name: string = ""
        sealed: bool = false

        // Listings. Each seller has at most one active listing.
        // available_units[seller] = how much energy the seller still
        // has on offer. price_per_unit[seller] = EVAP per unit.
        // listed[seller] = listing-active flag.
        available_units: map[address -> u64]
        price_per_unit: map[address -> u64]
        listed: map[address -> bool]
        listed_at_epoch: map[address -> u64]
        listing_count: u64 = 0

        // Aggregate market telemetry. cumulative_units_sold is monotonic
        // (lifetime fills); cumulative_evap_volume is the running EVAP
        // throughput. trade_count is the number of buy() calls
        // processed.
        cumulative_units_sold: u64 = 0
        cumulative_evap_volume: u64 = 0
        trade_count: u64 = 0

        closed: bool = false
    }

    // Phase 1: configure. market_name is informational. Operator is
    // the deployer; in v1 they have no special privileges beyond
    // labelling — open marketplace, anyone can list / buy.
    fn set_market(name: string) {
        require(caller == owner, "only operator can configure")
        require(self.sealed == false, "already configured")
        self.market_operator = owner
        self.market_name = name
        self.sealed = true
        emit("marketplace configured")
    }

    // Seller lists energy. Replaces any existing listing from the
    // same seller (in-place update). For a fresh seller, bumps
    // listing_count. For an updated seller, no count change.
    fn list(energy_units: u64, price: u64) {
        require(self.sealed == true, "not configured")
        require(self.closed == false, "marketplace closed")
        require(energy_units > 0, "energy_units must be positive")
        require(price > 0, "price must be positive")
        if self.listed[caller] == false {
            self.listing_count += 1
        }
        self.available_units[caller] = energy_units
        self.price_per_unit[caller] = price
        self.listed[caller] = true
        self.listed_at_epoch[caller] = epoch
        emit("listing posted")
    }

    // Buyer fills part-or-all of a specific seller's listing. The
    // coordinator picks up the trade event and applies the energy
    // refresh to whichever object the buyer specifies in their tx
    // (the on-chain ledger only records the trade; the cross-contract
    // refresh is a runtime operation).
    fn buy(seller_addr: address, units: u64) -> u64 {
        require(self.sealed == true, "not configured")
        require(self.closed == false, "marketplace closed")
        require(units > 0, "units must be positive")
        require(self.listed[seller_addr] == true, "no active listing for seller")
        let avail = self.available_units[seller_addr]
        require(avail >= units, "insufficient units in listing")
        let price = self.price_per_unit[seller_addr]
        let total_cost = units * price
        self.available_units[seller_addr] = avail - units
        self.cumulative_units_sold += units
        self.cumulative_evap_volume += total_cost
        self.trade_count += 1
        if self.available_units[seller_addr] == 0 {
            self.listed[seller_addr] = false
            self.listing_count -= 1
        }
        emit("trade executed")
        return total_cost
    }

    // Seller retracts their listing. No partial-cancel — the seller
    // simply pulls everything off the book and can re-list with new
    // terms.
    fn cancel() {
        require(self.sealed == true, "not configured")
        require(self.listed[caller] == true, "no active listing")
        self.listed[caller] = false
        self.available_units[caller] = 0
        self.listing_count -= 1
        emit("listing cancelled")
    }

    fn market_label() -> string {
        return self.market_name
    }

    fn active_listings() -> u64 {
        return self.listing_count
    }

    fn units_sold_total() -> u64 {
        return self.cumulative_units_sold
    }

    fn evap_volume_total() -> u64 {
        return self.cumulative_evap_volume
    }

    fn trades_total() -> u64 {
        return self.trade_count
    }

    fn listing_units_of(seller: address) -> u64 {
        return self.available_units[seller]
    }

    fn listing_price_of(seller: address) -> u64 {
        return self.price_per_unit[seller]
    }

    fn has_listing(seller: address) -> bool {
        return self.listed[seller]
    }

    fn is_open() -> bool {
        if self.sealed == false {
            return false
        }
        return self.closed == false
    }

    on_grace() {
        emit("marketplace energy low — refresh or all listings void")
    }

    on_refresh() {
        emit("marketplace refreshed")
    }

    // Doctrine moment: when the marketplace evaporates, all listings
    // are simultaneously void. Sellers retain their unsold energy
    // (they never actually transferred custody to the contract); the
    // contract simply stops being a venue. Buyers' in-flight trades
    // settle via the coordinator's view of the final ledger state.
    // No eternal order book.
    on_evaporate() {
        self.closed = true
        emit("marketplace evaporated — all listings void, liquidity dispersed")
    }
}
