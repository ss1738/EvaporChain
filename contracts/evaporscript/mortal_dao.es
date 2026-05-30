// MortalDAO — governance reference contract that composes ALL FOUR
// decay primitives shipped in 2026-05-29's PRs #470–#473:
//
//   decay-credential   → membership freshness (refresh to stay active)
//   decay-rate-limit   → per-member proposal cap (refresh to reset)
//   decay-reputation   → vote weight grows with participation
//   decay-quorum       → quorum threshold tracks a running peak of
//                        engagement, not a fixed fraction
//
// One contract = one DAO instance. One active proposal at a time
// (cycles after close_proposal). The contract's OWN energy is the
// DAO's lifespan: when it evaporates, the open proposal (if any) is
// closed unfinished. Refresh the contract energy to keep the DAO alive.
//
// All state is bounded by maps keyed on a registered member set, so
// the contract stays inside EvaporScript's V1 structural-totality gate
// (no `while`, branching via if/else only).

contract MortalDAO {
    state {
        // ── decay-credential: membership freshness ─────────────────
        // member -> (last_refresh_epoch + 1). The +1 shift means a
        // member added at epoch 0 has members[addr] == 1, never 0 —
        // so the `members[addr] == 0` sentinel reliably means
        // "not a member". Active iff (members[addr] - 1 + freshness_window) > epoch,
        // equivalently members[addr] + freshness_window > epoch + 1.
        members: map[address -> u64]
        member_count: u64 = 0
        // 500 epochs covers proposal_cap (3) × voting_window (50) = 150
        // with a comfortable buffer for human cadence between proposals.
        freshness_window: u64 = 500

        // ── decay-reputation: weight grows with participation ──────
        // member -> votes cast across all proposals. Voting weight
        // is (participations + 1) — a fresh member gets weight 1;
        // a steady voter accumulates influence over time.
        participations: map[address -> u64]

        // ── decay-rate-limit: per-member proposal cap ──────────────
        // member -> proposals opened since their last refresh.
        // Bounded by proposal_cap; refresh resets to 0.
        proposals_opened: map[address -> u64]
        proposal_cap: u64 = 3

        // ── active proposal slot (one decision at a time) ──────────
        active_proposal_id: u64 = 0
        proposal_text: string = ""
        proposal_proposer: address
        proposal_open: bool = false
        proposal_created_epoch: u64 = 0
        voting_window: u64 = 50
        for_votes: u64 = 0
        against_votes: u64 = 0
        weight_collected: u64 = 0
        // voter -> proposal_id last voted on. Equality with
        // active_proposal_id blocks double-voting on THIS proposal
        // without disturbing future proposals.
        voted_set: map[address -> u64]

        // ── decay-quorum: running peak of weight ever collected ────
        // Quorum gate: weight_collected * 2 >= observed_peak.
        // Early proposals set the bar low; once engagement spikes,
        // future proposals must clear half the peak.
        observed_peak: u64 = 0

        // ── history ────────────────────────────────────────────────
        next_proposal_id: u64 = 1
        decisions_carried: u64 = 0
        decisions_rejected: u64 = 0
    }

    // Genesis-only registry: owner adds founding members.
    fn add_member(who: address) {
        require(caller == owner, "only owner adds members")
        require(self.members[who] == 0, "already a member")
        // +1 shift so epoch=0 joiners are distinguishable from
        // never-joined (members[addr] == 0).
        self.members[who] = epoch + 1
        self.participations[who] = 0
        self.proposals_opened[who] = 0
        self.member_count += 1
        emit("member added")
    }

    // decay-credential — refresh resets your active timestamp AND
    // your proposal-cap counter in one call (engagement reset).
    fn refresh_membership() {
        require(self.members[caller] > 0, "not a member")
        self.members[caller] = epoch + 1
        self.proposals_opened[caller] = 0
        emit("membership refreshed")
    }

    // decay-rate-limit — open one of N proposals; refresh to reset cap.
    // Also gated by membership freshness (decay-credential).
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

    // decay-reputation — your weight = participations + 1. Each
    // vote also increments your participation counter, so steady
    // members get progressively heavier votes (capped only by
    // contract energy availability).
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

    // Close after voting_window elapses. decay-quorum gate:
    // collected weight must clear half the running peak.
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

    // ── Views ──────────────────────────────────────────────────────
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

    // Open-at-evaporation = closes the proposal unfinished and counts it
    // as rejected. Matches multisig.es's "unexecuted == expired" idiom.
    on_evaporate() {
        if self.proposal_open == true {
            self.proposal_open = false
            self.decisions_rejected += 1
            emit("DAO evaporated with open proposal — proposal expired")
        }
        emit("DAO evaporated")
    }
}
