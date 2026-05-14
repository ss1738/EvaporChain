// SealedBidAuction — ninth pilot EvaporScript contract.
//
// Classic commit / reveal / settle auction with a twist: `effective`
// (decay-adjusted) bid strength is the comparator, not the nominal
// bid. Bidders commit a hash in phase 0, reveal nominal + effective
// in phase 1, then the seller records the winner in phase 2 (which
// also closes the auction by advancing to phase 3).
//
// Doctrine moment: bid weight decays during reveal — early reveal
// wins ties on equal effective strength (the dApp coordinator
// computes effective from epoch-distance to commit). The contract's
// energy budget is the entire auction window; an evaporated auction
// without settlement is void by physics, and `on_evaporate` records
// that for the dApp coordinator.
//
// Phase machine:
//   0 = COMMIT     — bidders submit commit hashes
//   1 = REVEAL     — bidders open commits with nominal + effective
//   2 = SETTLE     — seller picks the winner
//   3 = CLOSED     — terminal; record_winner advances to this on
//                    success

contract SealedBidAuction {
    state {
        seller: address
        item: string = ""
        reserve_price: u64 = 0
        sealed: bool = false

        phase: u64 = 0

        // NX4 (re-audit 2026-05-14): pre-fix the `committed` map only
        // held a presence flag (1/0) and the bid hash supplied at
        // commit time was discarded. `reveal` never re-derived the
        // hash from (nominal, effective, blinding) and compared, so
        // the "seal" of "sealed-bid" was decorative — a bidder could
        // observe other reveals in-order on-chain and pick their own
        // nominal/effective to beat the highest seen.
        //
        // Fix: keep the presence map (workspace gotcha — map defaults
        // are U64(0) regardless of declared type; non-numeric maps
        // need a parallel u64 presence flag to test "key exists"
        // safely) AND store the actual commit hash in a parallel
        // string-keyed map. Reveal recomputes
        //   to_string(hash(blinding + ":" + to_string(nominal) +
        //                  ":" + to_string(effective)))
        // and rejects on mismatch.
        committed: map[address -> u64]
        committed_hash: map[address -> string]
        revealed: map[address -> u64]
        nominal: map[address -> u64]
        effective: map[address -> u64]

        commit_count: u64 = 0
        reveal_count: u64 = 0

        winner: address
        winning_effective: u64 = 0
        settled: bool = false
    }

    // Set the auction metadata one-shot. Seller-only.
    fn set_metadata(item_label: string, reserve: u64) {
        require(caller == owner, "only seller can set metadata")
        require(self.sealed == false, "auction already configured")
        self.seller = owner
        self.item = item_label
        self.reserve_price = reserve
        self.sealed = true
        emit("auction configured")
    }

    // Advance the phase machine. Strict monotone increment (skipping
    // is allowed; rewinding is not). Seller-only.
    fn set_phase(next: u64) {
        require(self.sealed == true, "auction not configured")
        require(caller == owner, "only seller can advance phase")
        require(next > self.phase, "phase only advances forward")
        require(next <= 3, "max phase is 3 (CLOSED)")
        self.phase = next
        emit("phase advanced")
    }

    // Commit a bid hash in phase 0. Open call; one commit per address.
    //
    // `commit_hash` is the bidder's pre-computed
    //   to_string(hash(blinding + ":" + to_string(nominal) +
    //                  ":" + to_string(effective)))
    // computed off-chain. `reveal` will recompute the same value from
    // the revealed inputs and require a match — that's what makes
    // this sealed. See NX4 docstring on the state block.
    fn commit(commit_hash: string) {
        require(self.sealed == true, "auction not configured")
        require(self.phase == 0, "not in COMMIT phase")
        require(self.committed[caller] == 0, "already committed")
        self.committed[caller] = 1
        self.committed_hash[caller] = commit_hash
        self.commit_count += 1
        emit("commit recorded")
    }

    // Reveal nominal + effective bid in phase 1. Must have committed.
    // Nominal must clear the reserve; effective must not exceed
    // nominal (the dApp coordinator decays nominal → effective by
    // reveal-time distance from commit, so effective <= nominal always).
    //
    // NX4 (re-audit 2026-05-14): `blinding` is the bidder's private
    // entropy that turned the (nominal, effective) pair into a
    // commitment opaque to other bidders. Pre-fix `reveal` ignored
    // this entirely; post-fix the contract recomputes
    //   expected = to_string(hash(blinding + ":" +
    //                             to_string(nominal_bid) + ":" +
    //                             to_string(effective_bid)))
    // and requires it match the stored `committed_hash[caller]`.
    // Without this binding, a bidder could observe other reveals
    // in-order on-chain during phase 1 and pick their own values
    // to beat the highest seen.
    fn reveal(nominal_bid: u64, effective_bid: u64, blinding: string) {
        require(self.sealed == true, "auction not configured")
        require(self.phase == 1, "not in REVEAL phase")
        require(self.committed[caller] > 0, "no commit on file")
        require(self.revealed[caller] == 0, "already revealed")
        // NX4: bind the reveal to the prior commit.
        let preimage = blinding + ":" + to_string(nominal_bid) + ":" + to_string(effective_bid)
        let expected = to_string(hash(preimage))
        require(self.committed_hash[caller] == expected, "reveal does not match commit")
        require(nominal_bid >= self.reserve_price, "nominal below reserve")
        require(effective_bid <= nominal_bid, "effective cannot exceed nominal")
        self.revealed[caller] = 1
        self.nominal[caller] = nominal_bid
        self.effective[caller] = effective_bid
        self.reveal_count += 1
        emit("bid revealed")
    }

    // Seller picks the winner in phase 2 and the contract advances
    // to phase 3 (CLOSED). Verifies the winner actually revealed and
    // that the seller's claimed effective matches the on-chain reveal.
    fn record_winner(winner_addr: address, winning_effective_bid: u64) {
        require(self.sealed == true, "auction not configured")
        require(caller == owner, "only seller can settle")
        require(self.phase == 2, "not in SETTLE phase")
        require(self.settled == false, "auction already settled")
        require(self.revealed[winner_addr] > 0, "winner did not reveal")
        let on_chain = self.effective[winner_addr]
        require(on_chain == winning_effective_bid, "effective bid mismatch")
        self.winner = winner_addr
        self.winning_effective = winning_effective_bid
        self.settled = true
        self.phase = 3
        emit("auction settled")
    }

    fn current_phase() -> u64 {
        return self.phase
    }

    fn nominal_bid_of(who: address) -> u64 {
        return self.nominal[who]
    }

    fn effective_bid_of(who: address) -> u64 {
        return self.effective[who]
    }

    fn winner_of() -> address {
        return self.winner
    }

    fn is_settled() -> bool {
        return self.settled
    }

    fn commits_received() -> u64 {
        return self.commit_count
    }

    fn reveals_received() -> u64 {
        return self.reveal_count
    }

    on_grace() {
        emit("auction energy low — settle or void approaches")
    }

    on_refresh() {
        emit("auction refreshed")
    }

    // Evaporation without settlement = void. Bidders refund off-chain
    // when the coordinator sees `settled == false` at evap.
    on_evaporate() {
        if self.settled == false {
            emit("auction evaporated — void (bidders refund)")
        }
    }
}
