// Single source of truth: `contracts/evaporscript/childkey.es`.
// Byte-stable inline copy. Pilot at `mod childkey_pilot` is the
// regression barrier proving this exact source parses, compiles,
// and is totality-clean.

export const CHILDKEY_SOURCE = `contract ChildKey {
    state {
        recipient: address
        unlock_epoch: u64 = 0
        content_hash: string = ""
        committee: map[address -> u64]
        committee_size: u64 = 0
        threshold: u64 = 0
        sealed: bool = false

        voted: map[address -> u64]
        vote_count: u64 = 0

        unlocked: bool = false
    }

    fn add_committee_member(member: address) {
        require(caller == owner, "only writer adds committee")
        require(self.sealed == false, "already sealed — committee frozen")
        require(self.committee[member] == 0, "already a committee member")
        self.committee[member] = 1
        self.committee_size += 1
        emit("committee member added")
    }

    fn arm(rec: address, unlock_at: u64, content: string, t: u64) {
        require(caller == owner, "only writer arms")
        require(self.sealed == false, "already armed")
        require(t > 0, "threshold must be positive")
        require(t <= self.committee_size, "threshold exceeds committee size")
        require(unlock_at > epoch, "unlock_epoch must be in the future")
        self.recipient = rec
        self.unlock_epoch = unlock_at
        self.content_hash = content
        self.threshold = t
        self.sealed = true
        emit("childkey armed")
    }

    fn vote_emergency() {
        require(self.sealed == true, "not armed")
        require(self.unlocked == false, "already unlocked")
        require(self.committee[caller] == 1, "not a committee member")
        require(self.voted[caller] == 0, "already voted")
        self.voted[caller] = 1
        self.vote_count += 1
        emit("emergency vote cast")
    }

    fn finalize_emergency_unlock() {
        require(self.sealed == true, "not armed")
        require(self.unlocked == false, "already unlocked")
        require(self.vote_count >= self.threshold, "emergency threshold not met")
        self.unlocked = true
        emit("emergency unlock finalized")
    }

    fn finalize_natural_unlock() {
        require(self.sealed == true, "not armed")
        require(self.unlocked == false, "already unlocked")
        require(epoch >= self.unlock_epoch, "natural unlock time not reached")
        self.unlocked = true
        emit("natural unlock finalized")
    }

    fn read_content() -> string {
        require(self.unlocked == true, "not yet unlocked")
        require(
            caller == self.recipient || self.committee[caller] == 1,
            "only recipient or committee may read"
        )
        return self.content_hash
    }

    fn is_committee_member(who: address) -> bool {
        return self.committee[who] == 1
    }

    fn has_voted(who: address) -> bool {
        return self.voted[who] == 1
    }

    fn vote_progress() -> u64 {
        return self.vote_count
    }

    fn threshold_required() -> u64 {
        return self.threshold
    }

    fn committee_count() -> u64 {
        return self.committee_size
    }

    fn is_armed() -> bool {
        return self.sealed
    }

    fn is_unlocked() -> bool {
        return self.unlocked
    }

    fn unlock_at() -> u64 {
        return self.unlock_epoch
    }

    fn is_recipient(who: address) -> bool {
        if self.sealed == false {
            return false
        }
        return who == self.recipient
    }

    fn epochs_until_unlock() -> u64 {
        if self.sealed == false {
            return 0
        }
        if epoch >= self.unlock_epoch {
            return 0
        }
        return self.unlock_epoch - epoch
    }

    on_grace() {
        emit("childkey energy low — refresh to keep alive until unlock")
    }

    on_refresh() {
        emit("childkey refreshed")
    }

    on_evaporate() {
        if self.unlocked == false {
            emit("childkey evaporated unread — letter lost")
        } else {
            emit("childkey evaporated post-read — content already disclosed")
        }
    }
}
`;
