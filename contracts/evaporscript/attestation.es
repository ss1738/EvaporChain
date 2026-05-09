// Attestation — eighth pilot. Stdlib contract #5: turns the
// attestation primitive into a decay-native one.
//
// Decay-thesis hook: existing attestation systems (EAS, Sign Protocol)
// record claims as eternal, immutable facts. "I said this in 2021" is
// just as queryable in 2031, even if I've revoked or changed my mind.
// Attestation makes the *strength* of the claim decay with the
// contract's energy: a fresh attestation is full-strength; a stale one
// is weaker; a fully decayed one carries no weight at all. To keep an
// attestation strong, the attestor must actively refresh it — silence
// is decay.
//
// This maps directly to real-world reputation: a recommendation from
// last week beats one from a decade ago. The chain enforces this
// physically via energy decay.
//
// Lifecycle:
//
//   1. Deploy → `attest(subject, claim_text)` seals the attestation.
//      One-shot. Subject can be an address, an object id (hex), or
//      any string identifier the dApp chooses to encode.
//   2. Third parties call `endorse()` to add their co-signature; each
//      endorsement is itself decay-tracked via the contract's lifetime.
//   3. `revoke()` lets the original attestor mark the claim as
//      withdrawn; readers should treat revoked attestations as
//      historical only.
//   4. on_evaporate: claim is permanently null. Ghost record persists
//      for audit but carries no live weight.
//
// Auth model:
//   - `attest`:    caller == owner (the attestor).
//   - `endorse`:   open (any address can co-sign).
//   - `revoke`:    caller == owner.

contract Attestation {
    state {
        attestor: address
        subject: string = ""
        claim: string = ""
        attested_at_epoch: u64 = 0
        sealed: bool = false
        revoked: bool = false
        revoked_at_epoch: u64 = 0

        // Endorsement ledger. endorsement_count is the headline
        // metric; per-address endorsement_epoch records when each
        // co-signer chimed in (so the dApp can compute an age-
        // weighted strength score off-chain if it wants nuance
        // beyond raw counts).
        endorsements: map[address -> bool]
        endorsement_epoch: map[address -> u64]
        endorsement_count: u64 = 0
    }

    // Phase 1: seal the attestation. Subject + claim text are immutable
    // after this call. The attested_at_epoch is the canonical
    // timestamp readers anchor decay-from.
    fn attest(subject_id: string, claim_text: string) {
        require(caller == owner, "only attestor can attest")
        require(self.sealed == false, "attestation already sealed")
        self.attestor = owner
        self.subject = subject_id
        self.claim = claim_text
        self.attested_at_epoch = epoch
        self.sealed = true
        emit("attestation sealed")
    }

    // Anyone can endorse. Each address can endorse exactly once;
    // re-endorsements are no-ops to keep the count honest. Endorsements
    // do not require the attestation to be unrevoked — co-signers may
    // stand by a revoked claim or formally distance themselves later
    // via their own endorsement contracts.
    fn endorse() {
        require(self.sealed == true, "attestation not yet sealed")
        require(self.endorsements[caller] == false, "already endorsed")
        self.endorsements[caller] = true
        self.endorsement_epoch[caller] = epoch
        self.endorsement_count += 1
        emit("endorsement recorded")
    }

    // Attestor revokes. Attestation isn't deleted — readers see the
    // claim text *and* the revoked flag, can decide for themselves
    // whether to give it any weight. revoked_at_epoch lets readers
    // distinguish "was true, attestor changed their mind" from
    // "attestor never stood behind it for long."
    fn revoke() {
        require(caller == owner, "only attestor can revoke")
        require(self.sealed == true, "nothing to revoke")
        require(self.revoked == false, "already revoked")
        self.revoked = true
        self.revoked_at_epoch = epoch
        emit("attestation revoked")
    }

    fn subject_of() -> string {
        return self.subject
    }

    fn claim_text() -> string {
        return self.claim
    }

    fn attestor_of() -> address {
        return self.attestor
    }

    fn attested_at() -> u64 {
        return self.attested_at_epoch
    }

    fn is_revoked() -> bool {
        return self.revoked
    }

    fn endorsements_total() -> u64 {
        return self.endorsement_count
    }

    fn has_endorsement(addr: address) -> bool {
        return self.endorsements[addr]
    }

    // Age in epochs since the original attestation was sealed.
    // Off-chain readers feed this (plus endorsement ages) into their
    // own scoring functions. The chain doesn't dictate a strength
    // formula — different dApps want different decay curves.
    fn age() -> u64 {
        if self.sealed == false {
            return 0
        }
        return epoch - self.attested_at_epoch
    }

    on_grace() {
        if self.revoked == false {
            emit("attestation energy low — refresh to keep claim live")
        }
    }

    on_refresh() {
        emit("attestation refreshed")
    }

    // Doctrine moment: when the attestation evaporates, the claim is
    // null on-chain. The ghost record preserves the audit trail (who
    // said what when), but no live query can resolve it.
    on_evaporate() {
        emit("attestation evaporated — claim no longer live")
    }
}
