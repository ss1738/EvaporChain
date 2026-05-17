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
//
// Commit-reveal binding (SBA-1 fix):
//   `commit_hash` is blake3(chain_id || auction_id || bidder || price || nonce)
//   produced by the Rust substrate (NX4 / decay-bound-auction crate).
//   At `reveal()` the caller must supply the SAME hash string;
//   the contract verifies the stored hash matches before accepting
//   the reveal. The Rust layer additionally verifies the pre-image
//   (price, nonce) against the hash so both layers enforce binding.

contract SealedBidAuction {
    state {
        seller: address
        item: string = ""
        reserve_price: u64 = 0
        sealed: bool = false

        phase: u64 = 0

        // committed_hashes stores the hash each bidder submitted at commit
        // time. Parallel presence map `committed` guards the U64(0) default
        // on unmapped keys (EvaporScript map default = U64(0)).
        committed_hashes: map[address -> string]
        committed: map[address -> u64]
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
    // commit_hash = blake3(chain_id || auction_id || caller || price || nonce)
    // as produced by the Rust NX4 binding layer.
    fn commit(commit_hash: string) {
        require(self.sealed == true, "auction not configured")
        require(self.phase == 0, "not in COMMIT phase")
        require(self.committed[caller] == 0, "already committed")
        // SBA-1: store the hash so reveal can verify binding.
        self.committed_hashes[caller] = commit_hash
        self.committed[caller] = 1
        self.commit_count += 1
        emit("commit recorded")
    }

    // Reveal nominal + effective bid in phase 1. Must have committed.
    // Nominal must clear the reserve; effective must not exceed
    // nominal (the dApp coordinator decays nominal → effective by
    // reveal-time distance from commit, so effective <= nominal always).
    // commitment_hash must match the hash submitted at commit time.
    fn reveal(nominal_bid: u64, effective_bid: u64, commitment_hash: string) {
        require(self.sealed == true, "auction not configured")
        require(self.phase == 1, "not in REVEAL phase")
        require(self.committed[caller] > 0, "no commit on file")
        require(self.revealed[caller] == 0, "already revealed")
        // SBA-1: verify the caller knows the exact hash they committed.
        // This binds the reveal to the commit; the Rust substrate also
        // verifies the blake3 pre-image (price, nonce).
        require(self.committed_hashes[caller] == commitment_hash, "commitment hash mismatch")
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

    // Return the stored commit hash for a bidder (for coordinator verification).
    fn committed_hash_of(who: address) -> string {
        return self.committed_hashes[who]
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
