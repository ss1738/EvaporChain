// Bounty — thirteenth pilot. Stdlib contract #10: turns the bounty /
// task-reward primitive into a decay-native one.
//
// Decay-thesis hook: standard bounty contracts (Gitcoin's old bounties
// network, Optimism RetroPGF interim escrows) can hold reward funds
// indefinitely waiting for a claimant. If the original task becomes
// irrelevant, or the poster disappears, the funds sit forever in a
// contract no one wants. Bounty binds the offer to the contract's
// energy lifetime: post the task, accept submissions, pick a winner —
// or the bounty refunds by physics. Forgotten and abandoned bounties
// don't accumulate; they evaporate.
//
// Lifecycle:
//
//   1. Deploy → `set_bounty(task_hash, reward)` configures. Sealed.
//   2. Hunters call `submit(solution_hash)` to register their work.
//      Each hunter can submit at most once.
//   3. Poster calls `accept(hunter_addr)` to designate a winner.
//   4. Winner calls `claim()` to record the pull (coordinator handles
//      transfer).
//   5. on_evaporate: if no winner has been accepted, the bounty
//      refunds to the poster (coordinator key off the void signal).
//
// Auth model:
//   - `set_bounty`:  caller == owner (poster).
//   - `submit`:      open (any address can register a solution).
//   - `accept`:      caller == owner.
//   - `claim`:       caller == self.winner.
//   - `cancel`:      caller == owner, while no submissions yet.

contract Bounty {
    state {
        poster: address
        task_hash: string = ""
        reward_amount: u64 = 0
        sealed: bool = false

        // Submissions ledger. Hunters submit solution_hashes; the
        // contract is opaque to the content (the poster verifies off-
        // chain).
        submissions: map[address -> string]
        submitted_at_epoch: map[address -> u64]
        submission_count: u64 = 0

        // Acceptance + claim state.
        winner: address
        accepted: bool = false
        accepted_at_epoch: u64 = 0
        reward_claimed: bool = false
        cancelled: bool = false
        refunded: bool = false
    }

    // Phase 1: post the bounty. task_hash is the canonical encoding of
    // the work spec (IPFS CID, etc.); the poster holds the spec doc.
    fn set_bounty(task: string, reward: u64) {
        require(caller == owner, "only poster can configure")
        require(self.sealed == false, "already configured")
        require(reward > 0, "reward must be positive")
        self.poster = owner
        self.task_hash = task
        self.reward_amount = reward
        self.sealed = true
        emit("bounty posted")
    }

    // Phase 2: hunter submits a solution. One submission per address;
    // re-submissions overwrite the previous attempt to avoid spam-
    // dilution of the on-chain record.
    fn submit(solution_hash: string) {
        require(self.sealed == true, "not configured")
        require(self.cancelled == false, "bounty cancelled")
        require(self.accepted == false, "bounty already accepted")
        let prev = self.submissions[caller]
        if prev == "" {
            self.submission_count += 1
        }
        self.submissions[caller] = solution_hash
        self.submitted_at_epoch[caller] = epoch
        emit("submission recorded")
    }

    // Phase 3: poster accepts a submission. Locks the winner. Once
    // accepted, the bounty is committed — only the named winner can
    // claim, and the bounty cannot be re-routed even by the poster.
    fn accept(winner_addr: address) {
        require(caller == owner, "only poster can accept")
        require(self.sealed == true, "not configured")
        require(self.accepted == false, "already accepted")
        require(
            self.submissions[winner_addr] != "",
            "winner has no submission on file"
        )
        self.winner = winner_addr
        self.accepted = true
        self.accepted_at_epoch = epoch
        emit("submission accepted")
    }

    // Phase 4: winner claims. Coordinator transfers actual funds; the
    // contract just records that the claim has happened.
    fn claim() -> u64 {
        require(self.accepted == true, "no acceptance yet")
        require(self.reward_claimed == false, "already claimed")
        require(caller == self.winner, "only winner can claim")
        self.reward_claimed = true
        emit("reward claimed")
        return self.reward_amount
    }

    // Poster can cancel only if no submissions have arrived yet —
    // once a hunter has invested work, the poster cannot rug-pull.
    // After submissions exist, the only way out is to accept one or
    // let the bounty evaporate (which refunds the poster anyway, but
    // signals to hunters that no acceptance was made).
    fn cancel() {
        require(caller == owner, "only poster can cancel")
        require(self.sealed == true, "not configured")
        require(self.cancelled == false, "already cancelled")
        require(self.accepted == false, "already accepted")
        require(self.submission_count == 0, "submissions exist; cannot cancel")
        self.cancelled = true
        emit("bounty cancelled")
    }

    fn task_of() -> string {
        return self.task_hash
    }

    fn reward() -> u64 {
        return self.reward_amount
    }

    fn submissions_total() -> u64 {
        return self.submission_count
    }

    fn submission_of(hunter: address) -> string {
        return self.submissions[hunter]
    }

    fn winner_of() -> address {
        return self.winner
    }

    fn is_accepted() -> bool {
        return self.accepted
    }

    fn is_claimed() -> bool {
        return self.reward_claimed
    }

    on_grace() {
        if self.accepted == false {
            if self.cancelled == false {
                emit("bounty energy low — poster should accept a submission or bounty refunds")
            }
        }
    }

    on_refresh() {
        emit("bounty refreshed")
    }

    // Doctrine moment: an unaccepted bounty refunds to the poster
    // when the contract evaporates. Hunters' submissions are
    // historical record; no acceptance means no payout. Poster's
    // funds don't sit forever.
    on_evaporate() {
        if self.accepted == false {
            self.refunded = true
            emit("bounty evaporated without acceptance — funds refund to poster")
        }
    }
}
