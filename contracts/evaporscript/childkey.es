// ChildKey — the reference contract behind CHILDKEY_LETTER
// (0x0001_0301), the Consumer lane's first concrete primitive
// (now backed alongside witnessfit).
//
// Catalogue description: "Sealed letters unlocked by recipient's age.
// Inverted decay. Today Show segment."
//
// Doctrine moment: most chain primitives decay AWAY (energy halves
// over time). ChildKey is one of the rare INVERTED-decay primitives:
// it accumulates accessibility instead of shedding it. From mint
// until the unlock_epoch, the letter is sealed. After the unlock
// epoch, it can be opened by the recipient (or by a quorum of the
// committee earlier, in case the writer dies or the recipient is
// incapacitated).
//
// One contract = one sealed letter. Trust model:
//   - The OWNER (writer) registers the committee + arms the contract.
//   - After arming, committee size + threshold + recipient + unlock
//     epoch + content hash are all IMMUTABLE.
//   - Reading the content_hash is gated on `unlocked == true`, which
//     can flip in TWO ways:
//       (a) anyone calls `finalize_natural_unlock()` once `epoch >=
//           unlock_epoch` — the planned path.
//       (b) `threshold` committee members each call `vote_emergency()`,
//           then anyone calls `finalize_emergency_unlock()` — the
//           safety valve for unexpected events.
//
// The on-chain stored "content" is a HASH; the cleartext lives
// off-chain (encrypted by a key derived from the recipient's identity
// + a committee secret-share). The chain provides the gating; the
// off-chain dApp handles the cryptography.

contract ChildKey {
    state {
        // ── one-shot config (set by add_committee_member + arm) ────
        recipient: address
        unlock_epoch: u64 = 0
        content_hash: string = ""
        committee: map[address -> u64]
        committee_size: u64 = 0
        threshold: u64 = 0
        sealed: bool = false

        // ── emergency-vote tracking ────────────────────────────────
        voted: map[address -> u64]
        vote_count: u64 = 0

        // ── terminal flag ──────────────────────────────────────────
        unlocked: bool = false
    }

    // Owner-only, pre-arm: register a committee member. Multisig-style
    // (one call per member). After arm() the committee is frozen.
    fn add_committee_member(member: address) {
        require(caller == owner, "only writer adds committee")
        require(self.sealed == false, "already sealed — committee frozen")
        require(self.committee[member] == 0, "already a committee member")
        self.committee[member] = 1
        self.committee_size += 1
        emit("committee member added")
    }

    // Owner-only, one-shot: arm the contract with the recipient,
    // unlock epoch, content hash, and emergency-vote threshold.
    // After this call, the contract is immutable until unlock.
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

    // Committee-only: cast an emergency-unlock vote. Each member
    // votes at most once; vote_count tracks the running total.
    fn vote_emergency() {
        require(self.sealed == true, "not armed")
        require(self.unlocked == false, "already unlocked")
        require(self.committee[caller] == 1, "not a committee member")
        require(self.voted[caller] == 0, "already voted")
        self.voted[caller] = 1
        self.vote_count += 1
        emit("emergency vote cast")
    }

    // Anyone: finalize the emergency unlock once the threshold is
    // met. The trigger is separated from the vote so the gas cost
    // sits with whoever wants to read first, not the marginal voter.
    fn finalize_emergency_unlock() {
        require(self.sealed == true, "not armed")
        require(self.unlocked == false, "already unlocked")
        require(self.vote_count >= self.threshold, "emergency threshold not met")
        self.unlocked = true
        emit("emergency unlock finalized")
    }

    // Anyone: finalize the planned unlock once the natural epoch
    // has been reached. No-op after `unlocked` flips true.
    fn finalize_natural_unlock() {
        require(self.sealed == true, "not armed")
        require(self.unlocked == false, "already unlocked")
        require(epoch >= self.unlock_epoch, "natural unlock time not reached")
        self.unlocked = true
        emit("natural unlock finalized")
    }

    // Read the content hash. Recipient-and-committee-gated post-unlock
    // (the cleartext is off-chain; the hash on-chain is the anchor).
    fn read_content() -> string {
        require(self.unlocked == true, "not yet unlocked")
        require(
            caller == self.recipient || self.committee[caller] == 1,
            "only recipient or committee may read"
        )
        return self.content_hash
    }

    // ── Views ──────────────────────────────────────────────────────
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

    // Epochs until natural unlock; 0 if already past or not armed.
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
