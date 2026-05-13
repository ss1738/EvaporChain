// Lottery — tenth pilot EvaporScript contract.
//
// Single-draw lottery with operator-supplied VRF proof. The operator
// deploys, configures `prize` + `stake`, then opens enrolment. Any
// address can enter exactly once. The operator submits the VRF proof
// and the winning address; the winner pulls the prize.
//
// Doctrine moment: an unresolved draw at evaporation is void by
// physics — entries refund themselves because the prize-funding
// contract is no longer on-chain to honour any claim. No coordinator
// poll, no rescue contract, no recovery flow. The protocol's natural
// decay enforces resolution-or-refund. Setting `voided = true` is
// purely a signal for the dApp coordinator to refund participants.

contract Lottery {
    state {
        operator: address
        prize: u64 = 0
        stake: u64 = 0
        sealed: bool = false

        entered: map[address -> u64]
        entry_count: u64 = 0

        drawn: bool = false
        winner: address
        vrf_blob: string = ""
        claimed: bool = false

        voided: bool = false
    }

    // Configure prize + stake. Operator-only, one-shot.
    fn set_event(prize_amount: u64, stake_amount: u64) {
        require(caller == owner, "only operator can configure")
        require(self.sealed == false, "lottery already configured")
        require(prize_amount > 0, "prize must be positive")
        require(stake_amount > 0, "stake must be positive")
        self.operator = owner
        self.prize = prize_amount
        self.stake = stake_amount
        self.sealed = true
        emit("lottery configured")
    }

    // Open enrolment. One entry per address; post-draw entries reject.
    fn enter() {
        require(self.sealed == true, "lottery not configured")
        require(self.drawn == false, "draw already happened")
        let already = self.entered[caller]
        require(already == 0, "already entered")
        self.entered[caller] = 1
        self.entry_count += 1
        emit("entry recorded")
    }

    // Operator picks the winner with a VRF proof. Requires the winner
    // to have entered. One-shot.
    fn set_winner(winner_addr: address, proof: string) {
        require(self.sealed == true, "lottery not configured")
        require(caller == owner, "only operator can draw")
        require(self.drawn == false, "lottery already drawn")
        require(self.entry_count > 0, "no entries to draw from")
        let present = self.entered[winner_addr]
        require(present > 0, "winner did not enter")
        self.winner = winner_addr
        self.vrf_blob = proof
        self.drawn = true
        emit("winner drawn")
    }

    // Winner pulls the prize. One-shot per draw.
    fn claim_prize() -> u64 {
        require(self.drawn == true, "no draw yet")
        require(caller == self.winner, "only winner can claim")
        require(self.claimed == false, "prize already claimed")
        self.claimed = true
        emit("prize claimed")
        return self.prize
    }

    fn entries_total() -> u64 {
        return self.entry_count
    }

    fn is_entered(who: address) -> bool {
        let v = self.entered[who]
        return v > 0
    }

    fn winner_of() -> address {
        return self.winner
    }

    fn vrf_proof() -> string {
        return self.vrf_blob
    }

    fn is_drawn() -> bool {
        return self.drawn
    }

    fn is_voided() -> bool {
        return self.voided
    }

    fn prize_size() -> u64 {
        return self.prize
    }

    fn stake_per_entry() -> u64 {
        return self.stake
    }

    on_grace() {
        emit("lottery energy low — resolve or void approaches")
    }

    on_refresh() {
        emit("lottery refreshed")
    }

    // Unresolved at evaporation = void. Entries refund off-chain by
    // the coordinator reading `voided = true`.
    on_evaporate() {
        if self.drawn == false {
            self.voided = true
            emit("lottery evaporated — entries refunded")
        }
    }
}
