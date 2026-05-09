// Lottery — twelfth pilot. Stdlib contract #9: turns the lottery /
// raffle primitive into a decay-native one.
//
// Decay-thesis hook: classic lottery contracts (PoolTogether, even
// the old smart-contract raffles) live indefinitely. If the operator
// disappears mid-draw, participants' funds and entries hang forever
// in a contract no one can resolve. Lottery binds the entire event to
// the contract's energy: entries open while the contract is alive,
// the draw must complete before evaporation, and an unresolved
// lottery is void by physics — entries refund signal fires from
// on_evaporate, no rescue contract needed.
//
// Pilot-grammar simplification: each address can `enter()` at most
// once. Multi-ticket variants would need array-of-tickets state which
// the pilot VM doesn't yet expose with iteration; this version is the
// "one-ticket-per-address weighted by stake" minimal form. Winner
// selection is off-chain VRF; the contract records the operator's
// pick and verifies the winner is a registered participant.
//
// Lifecycle:
//
//   1. Deploy → `set_event(prize, entry_stake)` configures the
//      lottery. Sealed-once.
//   2. Participants call `enter()` while the contract is alive.
//      entry_count tracks the running total.
//   3. Operator calls `set_winner(addr, vrf_proof)` once an off-chain
//      VRF resolves the draw. The contract verifies the winner has
//      entered.
//   4. on_evaporate: if no winner has been recorded, the lottery is
//      void. Coordinator emits refund-stakes signal.
//
// Auth model:
//   - `set_event`:     caller == owner.
//   - `enter`:         open (any address, before draw).
//   - `set_winner`:    caller == owner.
//   - `claim_prize`:   caller == self.winner.

contract Lottery {
    state {
        operator: address
        prize_amount: u64 = 0
        entry_stake: u64 = 0
        sealed: bool = false

        // Participants. entered[addr] flips true on first enter; one
        // entry per address. entry_count is the headline number used
        // by the off-chain VRF to size its random output.
        entered: map[address -> bool]
        entered_at_epoch: map[address -> u64]
        entry_count: u64 = 0

        // Draw state. winner != zero-address ⇒ draw completed.
        // vrf_commit captures the VRF proof for off-chain verification
        // (the contract treats the string as opaque).
        winner: address
        vrf_commit: string = ""
        drawn: bool = false
        drawn_at_epoch: u64 = 0
        prize_claimed: bool = false
        voided: bool = false
    }

    // Phase 1: configure. prize_amount is what the winner gets;
    // entry_stake is the per-participant entry fee. Both must be
    // positive — a free lottery would have unbounded entries.
    fn set_event(prize: u64, stake: u64) {
        require(caller == owner, "only operator can configure")
        require(self.sealed == false, "already configured")
        require(prize > 0, "prize must be positive")
        require(stake > 0, "stake must be positive")
        self.operator = owner
        self.prize_amount = prize
        self.entry_stake = stake
        self.sealed = true
        emit("lottery configured")
    }

    // Phase 2: anyone enters, paying entry_stake (recorded; actual
    // transfer is coordinator-driven). One entry per address.
    fn enter() {
        require(self.sealed == true, "not configured")
        require(self.drawn == false, "draw already happened")
        require(self.entered[caller] == false, "already entered")
        self.entered[caller] = true
        self.entered_at_epoch[caller] = epoch
        self.entry_count += 1
        emit("entry recorded")
    }

    // Phase 3: operator records the VRF-selected winner. The contract
    // verifies the address is a registered participant — preventing
    // the operator from fabricating a winner outside the entry set.
    // The vrf_proof string is opaque to the contract; off-chain
    // auditors verify it matches the VRF public key + entry seed.
    fn set_winner(winner_addr: address, vrf_proof: string) {
        require(caller == owner, "only operator can draw")
        require(self.sealed == true, "not configured")
        require(self.drawn == false, "already drawn")
        require(self.entry_count > 0, "no entries")
        require(self.entered[winner_addr] == true, "winner did not enter")
        self.winner = winner_addr
        self.vrf_commit = vrf_proof
        self.drawn = true
        self.drawn_at_epoch = epoch
        emit("winner recorded")
    }

    // Phase 4: winner pulls the prize. Coordinator handles actual
    // transfer; the contract just records that the prize has been
    // claimed so it can't be claimed again.
    fn claim_prize() -> u64 {
        require(self.drawn == true, "no draw yet")
        require(self.prize_claimed == false, "prize already claimed")
        require(caller == self.winner, "only winner can claim")
        self.prize_claimed = true
        emit("prize claimed")
        return self.prize_amount
    }

    fn entries_total() -> u64 {
        return self.entry_count
    }

    fn is_entered(addr: address) -> bool {
        return self.entered[addr]
    }

    fn winner_of() -> address {
        return self.winner
    }

    fn vrf_proof() -> string {
        return self.vrf_commit
    }

    fn is_drawn() -> bool {
        return self.drawn
    }

    fn is_voided() -> bool {
        return self.voided
    }

    fn prize_size() -> u64 {
        return self.prize_amount
    }

    fn stake_per_entry() -> u64 {
        return self.entry_stake
    }

    on_grace() {
        if self.drawn == false {
            emit("lottery energy low — operator must draw before evaporation or event voids")
        }
    }

    on_refresh() {
        emit("lottery refreshed")
    }

    // Doctrine moment: if the contract evaporates without a draw,
    // the lottery is void. Coordinator's refund-stakes flow keys off
    // this evaporation event. No rescue contract, no DAO vote, no
    // multi-step recovery — the protocol's natural decay enforces
    // resolution-or-refund.
    on_evaporate() {
        if self.drawn == false {
            self.voided = true
            emit("lottery evaporated without draw — entries void, refund stakes")
        }
    }
}
