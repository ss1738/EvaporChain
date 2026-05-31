// Single source of truth: `contracts/evaporscript/bounty.es`.
// Cargo pilot: `crates/evaporchain-script/tests/bounty_pilot.rs`.

export const BOUNTY_SOURCE = `contract Bounty {
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

    fn claim() -> u64 {
        require(self.accepted == true, "bounty not accepted")
        require(caller == self.winner, "only winner can claim")
        require(self.claimed == false, "bounty already claimed")
        self.claimed = true
        emit("bounty claimed")
        return self.reward_amount
    }

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

    fn submission_of(who: address) -> string {
        if self.has_submitted[who] == 0 {
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

    on_evaporate() {
        if self.accepted == false {
            self.refunded = true
            emit("bounty evaporated — refund to poster")
        }
    }
}`;
