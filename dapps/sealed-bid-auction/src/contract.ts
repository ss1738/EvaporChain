// Single source of truth: `contracts/evaporscript/sealed_bid_auction.es`.
// Cargo pilot: `crates/evaporchain-script/tests/sealed_bid_auction_pilot.rs`.

export const SEALED_BID_AUCTION_SOURCE = `contract SealedBidAuction {
    state {
        seller: address
        item: string = ""
        reserve_price: u64 = 0
        sealed: bool = false

        phase: u64 = 0

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

    fn set_metadata(item_label: string, reserve: u64) {
        require(caller == owner, "only seller can set metadata")
        require(self.sealed == false, "auction already configured")
        self.seller = owner
        self.item = item_label
        self.reserve_price = reserve
        self.sealed = true
        emit("auction configured")
    }

    fn set_phase(next: u64) {
        require(self.sealed == true, "auction not configured")
        require(caller == owner, "only seller can advance phase")
        require(next > self.phase, "phase only advances forward")
        require(next <= 3, "max phase is 3 (CLOSED)")
        self.phase = next
        emit("phase advanced")
    }

    fn commit(commit_hash: string) {
        require(self.sealed == true, "auction not configured")
        require(self.phase == 0, "not in COMMIT phase")
        require(self.committed[caller] == 0, "already committed")
        self.committed_hashes[caller] = commit_hash
        self.committed[caller] = 1
        self.commit_count += 1
        emit("commit recorded")
    }

    fn reveal(nominal_bid: u64, effective_bid: u64, commitment_hash: string) {
        require(self.sealed == true, "auction not configured")
        require(self.phase == 1, "not in REVEAL phase")
        require(self.committed[caller] > 0, "no commit on file")
        require(self.revealed[caller] == 0, "already revealed")
        require(self.committed_hashes[caller] == commitment_hash, "commitment hash mismatch")
        require(nominal_bid >= self.reserve_price, "nominal below reserve")
        require(effective_bid <= nominal_bid, "effective cannot exceed nominal")
        self.revealed[caller] = 1
        self.nominal[caller] = nominal_bid
        self.effective[caller] = effective_bid
        self.reveal_count += 1
        emit("bid revealed")
    }

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

    fn committed_hash_of(who: address) -> string {
        return self.committed_hashes[who]
    }

    on_grace() {
        emit("auction energy low — settle or void approaches")
    }

    on_refresh() {
        emit("auction refreshed")
    }

    on_evaporate() {
        if self.settled == false {
            emit("auction evaporated — void (bidders refund)")
        }
    }
}`;
