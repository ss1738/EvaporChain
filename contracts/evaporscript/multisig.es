// Multisig — third pilot EvaporScript contract.
//
// One contract = one decision. The deployer registers the signer set
// and threshold pre-propose, then seals the configuration by calling
// `propose(action)`. Registered signers each sign at most once. When
// `signature_count >= threshold`, anyone can call `execute` to mark the
// decision as carried out (off-chain side-effects are the dApp's job
// — the chain stores the authorisation).
//
// Doctrine moment: one-decision-per-contract. Gnosis-Safe-style
// proposal-map architectures conflate the signer set with the
// proposal stream. EvaporChain inverts that: the contract IS the
// proposal. Multiple decisions = multiple contracts, deployed
// independently, evaporating independently. An unexecuted multisig at
// evaporation is `expired = true` — its decision lapsed; no follow-up
// vote can resurrect it.

contract Multisig {
    state {
        signer_set: map[address -> u64]
        signed_set: map[address -> u64]
        signer_count: u64 = 0
        threshold: u64 = 0
        sealed: bool = false

        action: string = ""

        signature_count: u64 = 0
        executed: bool = false
        expired: bool = false
    }

    // Register a signer. Owner-only, pre-propose. Duplicates rejected.
    fn add_signer(who: address) {
        require(caller == owner, "only owner can add signers")
        require(self.sealed == false, "proposal already sealed")
        require(self.signer_set[who] == 0, "signer already registered")
        self.signer_set[who] = 1
        self.signer_count += 1
        emit("signer added")
    }

    // Set the approval threshold. Owner-only, pre-propose. Cannot
    // exceed registered signer count (a threshold no quorum can ever
    // satisfy would brick the contract).
    fn set_threshold(t: u64) {
        require(caller == owner, "only owner can set threshold")
        require(self.sealed == false, "proposal already sealed")
        require(t > 0, "threshold must be positive")
        require(t <= self.signer_count, "threshold exceeds signer count")
        self.threshold = t
        emit("threshold set")
    }

    // Seal the configuration with a proposal action. After this call,
    // add_signer + set_threshold revert. The action is opaque to the
    // contract — interpretation is the dApp's responsibility.
    fn propose(proposal_action: string) {
        require(caller == owner, "only owner can propose")
        require(self.sealed == false, "proposal already sealed")
        require(self.threshold > 0, "threshold not set")
        require(self.signer_count > 0, "no signers registered")
        self.action = proposal_action
        self.sealed = true
        emit("proposal sealed")
    }

    // Registered signer adds their signature. One per signer. Reverts
    // post-execute — the decision is closed.
    fn sign() {
        require(self.sealed == true, "no proposal to sign")
        require(self.executed == false, "proposal already executed")
        require(self.signer_set[caller] > 0, "not a signer")
        require(self.signed_set[caller] == 0, "already signed")
        self.signed_set[caller] = 1
        self.signature_count += 1
        emit("signature recorded")
    }

    // Anyone can trigger execution once the threshold is reached. The
    // contract records the authorisation; the dApp performs the
    // off-chain side effect.
    fn execute() {
        require(self.sealed == true, "no proposal to execute")
        require(self.executed == false, "proposal already executed")
        require(self.signature_count >= self.threshold, "threshold not yet reached")
        self.executed = true
        emit("proposal executed")
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

    fn has_signed(who: address) -> bool {
        return self.signed_set[who] > 0
    }

    fn is_signer(who: address) -> bool {
        return self.signer_set[who] > 0
    }

    fn proposal_action() -> string {
        return self.action
    }

    fn is_executed() -> bool {
        return self.executed
    }

    // Pending iff sealed AND not executed AND not expired.
    fn is_pending() -> bool {
        if self.sealed == false {
            return false
        }
        if self.executed == true {
            return false
        }
        if self.expired == true {
            return false
        }
        return true
    }

    on_grace() {
        emit("multisig energy low — execute or expire approaches")
    }

    on_refresh() {
        emit("multisig refreshed")
    }

    // Unexecuted at evaporation = expired. An executed multisig
    // discharged its purpose; expired stays false there.
    on_evaporate() {
        if self.executed == false {
            self.expired = true
            emit("multisig expired without execution")
        }
    }
}
