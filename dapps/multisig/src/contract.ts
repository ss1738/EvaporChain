// Single source of truth: `contracts/evaporscript/multisig.es`.
// Cargo pilot: `crates/evaporchain-script/tests/multisig_pilot.rs`.

export const MULTISIG_SOURCE = `contract Multisig {
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

    fn add_signer(who: address) {
        require(caller == owner, "only owner can add signers")
        require(self.sealed == false, "proposal already sealed")
        require(self.signer_set[who] == 0, "signer already registered")
        self.signer_set[who] = 1
        self.signer_count += 1
        emit("signer added")
    }

    fn set_threshold(t: u64) {
        require(caller == owner, "only owner can set threshold")
        require(self.sealed == false, "proposal already sealed")
        require(t > 0, "threshold must be positive")
        require(t <= self.signer_count, "threshold exceeds signer count")
        self.threshold = t
        emit("threshold set")
    }

    fn propose(proposal_action: string) {
        require(caller == owner, "only owner can propose")
        require(self.sealed == false, "proposal already sealed")
        require(self.threshold > 0, "threshold not set")
        require(self.signer_count > 0, "no signers registered")
        self.action = proposal_action
        self.sealed = true
        emit("proposal sealed")
    }

    fn sign() {
        require(self.sealed == true, "no proposal to sign")
        require(self.executed == false, "proposal already executed")
        require(self.signer_set[caller] > 0, "not a signer")
        require(self.signed_set[caller] == 0, "already signed")
        self.signed_set[caller] = 1
        self.signature_count += 1
        emit("signature recorded")
    }

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

    on_evaporate() {
        if self.executed == false {
            self.expired = true
            emit("multisig expired without execution")
        }
    }
}`;
