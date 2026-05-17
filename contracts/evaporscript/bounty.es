// Bounty — eleventh pilot EvaporScript contract.
//
// Open-call task with a fixed reward. The deployer (poster) posts a
// task spec + reward. Any address can submit a solution; the poster
// picks a winner and the winner claims exactly once.
//
// Doctrine moment: an unaccepted bounty refunds to the poster when
// the contract evaporates. Hunters' submissions are kept as
// historical record, but they produce no payout without explicit
// acceptance — the poster's funds don't sit forever on a stalled
// bounty. No off-chain liquidator needed; the runtime is the closer.
//
// `has_submitted` is a parallel u64 presence map (1 = submitted, 0 =
// not). EvaporScript maps return 0 for missing keys regardless of the
// value type, so a parallel numeric presence map is more reliable
// than peeking at the string content.

contract Bounty {
    state {
        poster: address
        task: string = ""
        reward_amount: u64 = 0
        sealed: bool = false

        submissions: map[address -> string]
        has_submitted: map[address -> u64]
        submission_count: u64 = 0

        accepted: bool = false
        winner: address
        claimed: bool = false
        cancelled: bool = false

        refunded: bool = false
    }

    // Post the bounty exactly once. Poster (deployer / owner) only.
    fn set_bounty(task_spec: string, reward: u64) {
        require(caller == owner, "only poster can set bounty")
        require(self.sealed == false, "bounty already set")
        require(reward > 0, "reward must be positive")
        self.poster = owner
        self.task = task_spec
        self.reward_amount = reward
        self.sealed = true
        emit("bounty posted")
    }

    // Hunters submit solutions. Open call. Resubmissions overwrite the
    // stored solution but do NOT bump the per-hunter or global counter.
    fn submit(solution: string) {
        require(self.sealed == true, "bounty not yet set")
        require(self.accepted == false, "bounty already accepted")
        require(self.cancelled == false, "bounty cancelled")
        let already = self.has_submitted[caller]
        if already == 0 {
            self.submission_count += 1
            self.has_submitted[caller] = 1
        }
        self.submissions[caller] = solution
        emit("solution submitted")
    }

    // Poster accepts a winning submission. Must reference an address
    // that has previously submitted. One-shot.
    fn accept(winner_addr: address) {
        require(self.sealed == true, "bounty not yet set")
        require(caller == owner, "only poster can accept")
        require(self.accepted == false, "bounty already accepted")
        require(self.cancelled == false, "bounty cancelled")
        let present = self.has_submitted[winner_addr]
        require(present > 0, "no submission on file for that address")
        self.winner = winner_addr
        self.accepted = true
        emit("submission accepted")
    }

    // Winner pulls the reward. One-shot per bounty.
    fn claim() -> u64 {
        require(self.accepted == true, "bounty not accepted")
        require(caller == self.winner, "only winner can claim")
        require(self.claimed == false, "bounty already claimed")
        self.claimed = true
        emit("bounty claimed")
        return self.reward_amount
    }

    // Poster cancels before any hunter has submitted. Once a hunter has
    // put work into a submission cancellation is blocked — no rug-pull.
    fn cancel() {
        require(self.sealed == true, "bounty not yet set")
        require(caller == owner, "only poster can cancel")
        require(self.accepted == false, "bounty already accepted")
        require(self.cancelled == false, "already cancelled")
        require(self.submission_count == 0, "submissions exist — cannot cancel")
        self.cancelled = true
        emit("bounty cancelled")
    }

    fn task_of() -> string {
        return self.task
    }

    fn reward() -> u64 {
        return self.reward_amount
    }

    fn submissions_total() -> u64 {
        return self.submission_count
    }

    // BOUNTY-1 (audit 2026-05-17): EvaporScript map defaults are U64(0)
    // regardless of declared value type (see grammar gotchas). Reading
    // self.submissions[who] for an unsubmitted `who` returns U64(0) and
    // would surface as a runtime type-mismatch when the caller expects
    // string. Gate on the parallel `has_submitted` presence map.
    fn submission_of(who: address) -> string {
        let present = self.has_submitted[who]
        if present == 0 {
            return ""
        }
        return self.submissions[who]
    }

    fn winner_of() -> address {
        return self.winner
    }

    fn is_accepted() -> bool {
        return self.accepted
    }

    fn is_claimed() -> bool {
        return self.claimed
    }

    on_grace() {
        emit("bounty energy low — accept or it will refund")
    }

    on_refresh() {
        emit("bounty refreshed")
    }

    // If the bounty evaporates without an accepted winner, signal
    // refund so the coordinator can return the reward to the poster.
    on_evaporate() {
        if self.accepted == false {
            self.refunded = true
            emit("bounty evaporated — refund to poster")
        }
    }
}
