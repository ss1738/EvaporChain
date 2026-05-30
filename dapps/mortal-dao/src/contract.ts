// Single source of truth: `contracts/evaporscript/mortal_dao.es`.
// This is the inline copy the dApp ships with — keep it byte-identical
// to the .es file. The Rust pilot test
// `evaporchain-script` → `mod mortal_dao_pilot` (in lib.rs) is the
// regression barrier proving this exact source compiles + runs through
// the VM and is totality-clean. Don't edit the body here without
// running it.
//
// Deploy flow:
//   1. POST /api/tx/deploy-script  with source_code = MORTAL_DAO_SOURCE
//      + energy (DAO's own lifespan) + half_life (how fast the DAO ages).
//   2. POST /api/tx/call-script    method = "add_member"  (owner-only,
//      register each founding member).
//   3. From then on: refresh_membership / open_proposal / vote_for /
//      vote_against / close_proposal — see client.ts for typed builders.

export const MORTAL_DAO_SOURCE = `contract MortalDAO {
    state {
        members: map[address -> u64]
        member_count: u64 = 0
        freshness_window: u64 = 100

        participations: map[address -> u64]

        proposals_opened: map[address -> u64]
        proposal_cap: u64 = 3

        active_proposal_id: u64 = 0
        proposal_text: string = ""
        proposal_proposer: address
        proposal_open: bool = false
        proposal_created_epoch: u64 = 0
        voting_window: u64 = 50
        for_votes: u64 = 0
        against_votes: u64 = 0
        weight_collected: u64 = 0
        voted_set: map[address -> u64]

        observed_peak: u64 = 0

        next_proposal_id: u64 = 1
        decisions_carried: u64 = 0
        decisions_rejected: u64 = 0
    }

    fn add_member(who: address) {
        require(caller == owner, "only owner adds members")
        require(self.members[who] == 0, "already a member")
        self.members[who] = epoch
        self.participations[who] = 0
        self.proposals_opened[who] = 0
        self.member_count += 1
        emit("member added")
    }

    fn refresh_membership() {
        require(self.members[caller] > 0, "not a member")
        self.members[caller] = epoch
        self.proposals_opened[caller] = 0
        emit("membership refreshed")
    }

    fn open_proposal(text: string) {
        require(self.members[caller] > 0, "not a member")
        require(self.proposal_open == false, "another proposal already open")
        require(
            self.members[caller] + self.freshness_window > epoch,
            "membership stale — refresh first"
        )
        require(
            self.proposals_opened[caller] < self.proposal_cap,
            "proposer cap reached — refresh to reset"
        )
        self.proposal_text = text
        self.proposal_proposer = caller
        self.proposal_open = true
        self.proposal_created_epoch = epoch
        self.active_proposal_id = self.next_proposal_id
        self.next_proposal_id += 1
        self.for_votes = 0
        self.against_votes = 0
        self.weight_collected = 0
        self.proposals_opened[caller] += 1
        emit("proposal opened")
    }

    fn vote_for() {
        require(self.proposal_open == true, "no open proposal")
        require(self.members[caller] > 0, "not a member")
        require(
            self.members[caller] + self.freshness_window > epoch,
            "membership stale"
        )
        require(
            self.voted_set[caller] != self.active_proposal_id,
            "already voted on this proposal"
        )
        self.for_votes += self.participations[caller] + 1
        self.weight_collected += self.participations[caller] + 1
        self.voted_set[caller] = self.active_proposal_id
        self.participations[caller] += 1
        emit("vote for")
    }

    fn vote_against() {
        require(self.proposal_open == true, "no open proposal")
        require(self.members[caller] > 0, "not a member")
        require(
            self.members[caller] + self.freshness_window > epoch,
            "membership stale"
        )
        require(
            self.voted_set[caller] != self.active_proposal_id,
            "already voted on this proposal"
        )
        self.against_votes += self.participations[caller] + 1
        self.weight_collected += self.participations[caller] + 1
        self.voted_set[caller] = self.active_proposal_id
        self.participations[caller] += 1
        emit("vote against")
    }

    fn close_proposal() -> bool {
        require(self.proposal_open == true, "no open proposal")
        require(
            epoch >= self.proposal_created_epoch + self.voting_window,
            "voting window not closed yet"
        )
        if self.weight_collected > self.observed_peak {
            self.observed_peak = self.weight_collected
        }
        require(
            self.weight_collected * 2 >= self.observed_peak,
            "quorum not reached against running peak"
        )
        self.proposal_open = false
        if self.for_votes > self.against_votes {
            self.decisions_carried += 1
            emit("proposal carried")
            return true
        }
        self.decisions_rejected += 1
        emit("proposal rejected")
        return false
    }

    fn member_count_now() -> u64 {
        return self.member_count
    }

    fn is_member(who: address) -> bool {
        return self.members[who] > 0
    }

    fn is_active(who: address) -> bool {
        if self.members[who] == 0 {
            return false
        }
        if self.members[who] + self.freshness_window <= epoch {
            return false
        }
        return true
    }

    fn weight_of(who: address) -> u64 {
        return self.participations[who] + 1
    }

    fn proposal_open_now() -> bool {
        return self.proposal_open
    }

    fn for_count() -> u64 {
        return self.for_votes
    }

    fn against_count() -> u64 {
        return self.against_votes
    }

    fn weight_collected_now() -> u64 {
        return self.weight_collected
    }

    fn peak() -> u64 {
        return self.observed_peak
    }

    fn carried_total() -> u64 {
        return self.decisions_carried
    }

    fn rejected_total() -> u64 {
        return self.decisions_rejected
    }

    fn next_id() -> u64 {
        return self.next_proposal_id
    }

    on_grace() {
        emit("DAO energy low — refresh to keep alive")
    }

    on_refresh() {
        emit("DAO refreshed")
    }

    on_evaporate() {
        if self.proposal_open == true {
            self.proposal_open = false
            self.decisions_rejected += 1
            emit("DAO evaporated with open proposal — proposal expired")
        }
        emit("DAO evaporated")
    }
}
`;
