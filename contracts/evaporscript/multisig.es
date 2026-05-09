// Multisig — eleventh pilot. Stdlib contract #8: turns the m-of-n
// multisig primitive into a decay-native one.
//
// Decay-thesis hook: classic multisigs (Gnosis Safe, etc.) sit forever
// with the same signer set, regardless of whether the signers are
// still active, still controlled by the same humans, or still even
// alive. A multisig from 2018 is fully usable in 2028 even if half the
// signers have lost their keys. Multisig binds each signer to the
// contract's energy: once deployed, the proposal must be approved by
// `threshold` of the signers BEFORE the contract evaporates. If the
// signers are absent or unreachable, the proposal fails by physics —
// no coordinator polling, no recovery flow, no hostage problem.
//
// Single-proposal-per-contract design: each multisig instance encodes
// exactly one decision. New decision = new contract. This makes old
// decisions evaporate cleanly instead of accumulating in a shared
// proposal map.
//
// Lifecycle:
//
//   1. Deploy → `add_signer(addr)` repeated for each signer, then
//      `set_threshold(k)`, then `propose(action_hash)`. Sealed.
//   2. Each signer calls `sign()` to add their approval. signed[caller]
//      flips true; signature_count bumps.
//   3. Anyone calls `execute()` once signature_count >= threshold.
//      The contract records execution; the actual action (call to
//      another contract, transfer, etc.) is coordinator-driven from
//      the action_hash + execution event.
//   4. on_evaporate: if not executed, proposal expires.
//
// Auth model:
//   - `add_signer` / `set_threshold` / `propose`:  caller == owner.
//   - `sign`:    caller in signers + proposal sealed + not executed.
//   - `execute`: open (anyone, once threshold reached).

contract Multisig {
    state {
        // Setup phase. signers ledger; threshold of signatures
        // required to execute. threshold <= signer_count enforced
        // at set_threshold time.
        signers: map[address -> bool]
        signer_count: u64 = 0
        threshold: u64 = 0

        // Proposal state. action_hash is opaque to the contract — the
        // coordinator interprets it as a target call, transfer, etc.
        action_hash: string = ""
        proposal_sealed: bool = false
        proposed_at_epoch: u64 = 0

        // Signature ledger. signed[addr] flips true exactly once per
        // signer; signature_count is the headline tally.
        signed: map[address -> bool]
        signed_at_epoch: map[address -> u64]
        signature_count: u64 = 0

        // Execution. Once executed, no more signatures count and the
        // proposal becomes immutable history.
        executed: bool = false
        executed_at_epoch: u64 = 0
        executed_by: address
        expired: bool = false
    }

    // Phase 1a: register a signer. Deployer-gated. Signers can be added
    // until propose() is called.
    fn add_signer(signer_addr: address) {
        require(caller == owner, "only deployer can add signer")
        require(self.proposal_sealed == false, "proposal already sealed")
        require(self.signers[signer_addr] == false, "signer already added")
        self.signers[signer_addr] = true
        self.signer_count += 1
        emit("signer added")
    }

    // Phase 1b: set the threshold. Must be > 0 and <= signer_count.
    fn set_threshold(k: u64) {
        require(caller == owner, "only deployer can set threshold")
        require(self.proposal_sealed == false, "proposal already sealed")
        require(k > 0, "threshold must be positive")
        require(k <= self.signer_count, "threshold exceeds signer count")
        self.threshold = k
        emit("threshold set")
    }

    // Phase 2: lock the signer set + threshold and bind to a specific
    // action. action_hash is the blake3 of the canonical encoding of
    // the action the multisig is approving — the coordinator verifies
    // that whatever they execute matches.
    fn propose(action: string) {
        require(caller == owner, "only deployer can propose")
        require(self.proposal_sealed == false, "already proposed")
        require(self.signer_count > 0, "no signers")
        require(self.threshold > 0, "threshold not set")
        self.action_hash = action
        self.proposal_sealed = true
        self.proposed_at_epoch = epoch
        emit("proposal sealed")
    }

    // Phase 3: signer approves. Each signer signs at most once.
    fn sign() {
        require(self.proposal_sealed == true, "no proposal yet")
        require(self.executed == false, "already executed")
        require(self.signers[caller] == true, "caller is not a signer")
        require(self.signed[caller] == false, "caller already signed")
        self.signed[caller] = true
        self.signed_at_epoch[caller] = epoch
        self.signature_count += 1
        emit("signature recorded")
    }

    // Phase 4: anyone can call execute() once threshold is met. The
    // coordinator picks up the execution event and runs the actual
    // action off-chain (or via a chained call_contract); this method
    // just records that the multisig has authorised it.
    fn execute() {
        require(self.proposal_sealed == true, "no proposal yet")
        require(self.executed == false, "already executed")
        require(
            self.signature_count >= self.threshold,
            "threshold not yet reached"
        )
        self.executed = true
        self.executed_at_epoch = epoch
        self.executed_by = caller
        emit("multisig executed")
    }

    fn signers_total() -> u64 {
        return self.signer_count
    }

    fn threshold_required() -> u64 {
        return self.threshold
    }

    fn signatures_collected() -> u64 {
        return self.signature_count
    }

    fn has_signed(addr: address) -> bool {
        return self.signed[addr]
    }

    fn is_signer(addr: address) -> bool {
        return self.signers[addr]
    }

    fn proposal_action() -> string {
        return self.action_hash
    }

    fn is_executed() -> bool {
        return self.executed
    }

    fn is_pending() -> bool {
        if self.proposal_sealed == false {
            return false
        }
        if self.executed == true {
            return false
        }
        return self.expired == false
    }

    on_grace() {
        if self.executed == false {
            if self.proposal_sealed == true {
                emit("multisig energy low — collect remaining signatures or proposal expires")
            }
        }
    }

    on_refresh() {
        emit("multisig refreshed")
    }

    // Doctrine moment: an unexecuted proposal expires when the
    // contract evaporates. Hostage problem solved by physics: lost-
    // key signers cannot block forever, because the proposal itself
    // is mortal.
    on_evaporate() {
        if self.executed == false {
            self.expired = true
            emit("multisig evaporated — proposal expired without execution")
        }
    }
}
