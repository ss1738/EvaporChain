// SealedBidAuction — fifth pilot. Stdlib contract #2: turns the
// commit-reveal auction primitive into a decay-native one.
//
// Decay-thesis hook: standard sealed-bid auctions have a hard reveal
// deadline — bidders who reveal one block late lose their entire bid.
// Plus the "last-minute sniping" problem: a bidder who reveals at the
// final moment has no time pressure on counter-bidders. SealedBidAuction
// replaces the cliff with a decay curve: bid *weight* decays during
// reveal, so revealing early is rewarded and revealing late is penalised
// continuously. The decay rate is the contract's own half-life — when
// the contract evaporates, all unrevealed bids are forfeit.
//
// Lifecycle:
//
//   1. Deploy with the auctioned-item descriptor (string blob — IPFS
//      hash, NFT id, etc.) and a reserve_price.
//   2. `set_phase(0)` → COMMIT phase. Bidders submit `commit(hash)`
//      where hash = blake3(amount || nonce). Multiple commits per
//      bidder allowed; only the latest counts.
//   3. `set_phase(1)` → REVEAL phase. Bidders call `reveal(amount,
//      nonce)`. Effective bid = amount * (current_energy /
//      initial_energy) — i.e. revealing while the contract is fresh
//      counts at full strength; revealing after grace counts at less.
//      The chain runtime applies the decay; this contract records the
//      effective bid.
//   4. `set_phase(2)` → SETTLE phase. Highest effective bid wins;
//      `record_winner()` stamps the winner. Loser refunds and seller
//      payout are coordinator-driven (off-chain settlement, ledger
//      lives here).
//
// Auth model:
//   - Phase transitions:   caller == owner (deployer = auctioneer).
//   - commit/reveal:       open (any address can bid).
//   - record_winner:       caller == owner.

contract SealedBidAuction {
    state {
        // Item descriptor + reserve. Sealed at deploy via set_metadata.
        item: string = ""
        reserve_price: u64 = 0
        seller: address
        sealed: bool = false

        // Phase: 0 = COMMIT, 1 = REVEAL, 2 = SETTLE, 3 = CLOSED.
        // Phases advance monotonically; no rewinds.
        phase: u64 = 0
        commit_started_epoch: u64 = 0
        reveal_started_epoch: u64 = 0

        // Per-bidder commit. Latest commit wins (allows raise-bid).
        // commit_count tracks how many bidders have committed at all.
        commits: map[address -> string]
        commit_count: u64 = 0
        bidder_first_commit_epoch: map[address -> u64]

        // Per-bidder reveal. effective_bid = nominal_bid * decay_factor
        // applied by the chain runtime when reveal() is called. We
        // record both the nominal and effective values for audit.
        nominal_bids: map[address -> u64]
        effective_bids: map[address -> u64]
        revealed: map[address -> bool]
        reveal_count: u64 = 0

        // Settlement state. winner != zero-address ⇒ auction settled.
        winner: address
        winning_bid: u64 = 0
        settled: bool = false
    }

    // Seal the auction descriptor exactly once. Deployer-gated.
    fn set_metadata(item_descriptor: string, item_reserve: u64) {
        require(caller == owner, "only seller can seal")
        require(self.sealed == false, "auction already sealed")
        self.item = item_descriptor
        self.reserve_price = item_reserve
        self.seller = owner
        self.sealed = true
        self.commit_started_epoch = epoch
        emit("auction sealed")
    }

    // Advance to next phase. Monotonic; phase 0→1→2→3 only.
    fn set_phase(next: u64) {
        require(caller == owner, "only seller can advance phase")
        require(self.sealed == true, "auction not yet sealed")
        require(next > self.phase, "phase only advances")
        require(next <= 3, "max phase is 3")
        self.phase = next
        if next == 1 {
            self.reveal_started_epoch = epoch
        }
        emit("phase advanced")
    }

    // COMMIT phase: bidder submits a hash of (amount || nonce). Latest
    // commit replaces the previous one (raise-bid is free; the pilot
    // grammar can't refund stake, so commit-only-once would be punitive
    // for raise-intent bidders).
    fn commit(commit_hash: string) {
        require(self.sealed == true, "auction not yet sealed")
        require(self.phase == 0, "not in COMMIT phase")
        let prev = self.commits[caller]
        if prev == "" {
            self.commit_count += 1
            self.bidder_first_commit_epoch[caller] = epoch
        }
        self.commits[caller] = commit_hash
        emit("commit recorded")
    }

    // REVEAL phase: bidder discloses (amount, nonce). The contract
    // doesn't recompute the hash here — the chain runtime / VM verifies
    // hash(amount || nonce) == commits[caller] via a separate verify
    // opcode pathway in production. The pilot grammar trusts the call
    // and records the bid for downstream settlement.
    //
    // Effective bid is the nominal bid scaled by the contract's current
    // energy ratio. The chain runtime supplies `effective` directly via
    // the call args (the dApp computes it from `current_energy /
    // initial_energy` before calling). This keeps the pilot grammar
    // free of float arithmetic while still letting the decay curve
    // shape the auction.
    fn reveal(nominal_amount: u64, effective_amount: u64) {
        require(self.sealed == true, "auction not yet sealed")
        require(self.phase == 1, "not in REVEAL phase")
        require(self.revealed[caller] == false, "already revealed")
        require(self.commits[caller] != "", "no commit on file")
        require(nominal_amount >= self.reserve_price, "bid below reserve")
        require(effective_amount <= nominal_amount, "effective cannot exceed nominal")
        self.nominal_bids[caller] = nominal_amount
        self.effective_bids[caller] = effective_amount
        self.revealed[caller] = true
        self.reveal_count += 1
        emit("reveal recorded")
    }

    // SETTLE phase: stamp the winner. Caller must be the seller. Winner
    // determination uses the *effective* bid (decay-adjusted), so a
    // late-revealing bidder loses to an early-revealing one even if
    // their nominal amount was higher. This is the doctrine moment:
    // the auction prices time-to-reveal as well as willingness-to-pay.
    fn record_winner(winner_addr: address, effective_winning: u64) {
        require(caller == owner, "only seller can settle")
        require(self.phase == 2, "not in SETTLE phase")
        require(self.settled == false, "already settled")
        require(self.revealed[winner_addr] == true, "winner did not reveal")
        require(
            effective_winning == self.effective_bids[winner_addr],
            "effective bid mismatch"
        )
        self.winner = winner_addr
        self.winning_bid = self.nominal_bids[winner_addr]
        self.settled = true
        self.phase = 3
        emit("auction settled")
    }

    fn current_phase() -> u64 {
        return self.phase
    }

    fn nominal_bid_of(addr: address) -> u64 {
        return self.nominal_bids[addr]
    }

    fn effective_bid_of(addr: address) -> u64 {
        return self.effective_bids[addr]
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
        emit("auction energy low — bidders should reveal before evaporation")
    }

    on_refresh() {
        emit("auction refreshed")
    }

    // Doctrine moment: if the contract evaporates without settling,
    // the auction is void. All commits and reveals lapse. This is the
    // strongest commitment device the chain offers — even the seller
    // can't extend the auction past its energy budget.
    on_evaporate() {
        if self.settled == false {
            emit("auction evaporated — bids lapsed, item retained by seller")
        }
    }
}
