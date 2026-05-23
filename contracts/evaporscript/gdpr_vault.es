// GdprVault — EvaporScript contract for GDPR-Erasure-as-a-Service
// (model A: crypto-shred). See research/GDPR_ERASURE_ARCHITECTURE.md.
//
// FOUNDING CONSTRAINT (verified 2026-05-17, Dead Drop §9 probe): the
// chain does NOT byte-erase — post-evaporation get_script still
// returns .state. So this contract holds NO personal data, ever:
// only a 32-byte ciphertext commitment + consent/retention lifecycle.
// "Erasure" = off-chain destruction of the decryption key
// (crypto-shred). This contract's job is to be the tamper-evident
// retention clock and the provable key-shred trigger — nothing else.
//
// One retained record = one contract instance (the proven
// SFSV/mortal_message/evaporcash_note single-instance pattern). The
// contract's OWN energy is the retention clock: deploy energy/
// half_life are sized so it reaches terminal `evaporated` at the
// retention deadline. The contract reads only its own `energy`
// builtin — there is no in-script decay primitive (verified;
// EVAPORSCRIPT.md). All state is bounded (two addresses, two
// scalars, two flags) so this stays inside EvaporScript's
// structural totality contract.
//
// Deploy via DeployScript with:
//   - `energy`     = retention budget (sized to the retention period)
//   - `half_life`  = decay rate (controller picks; sets the deadline)
// Then call `seal(ct_commit, subject_ref, lawful_basis)` exactly once.
//
// The off-chain key-custody/HSM subscribes to the emitted
// "erasure-due" event and destroys the key. The immutable on-chain
// record (finalised seal + terminal evaporation at/after the
// retention epoch, OR an explicit withdraw_consent) is the audit
// artifact a DPO/regulator needs.

contract GdprVault {
    state {
        // ── commitment (NOT the data — a hash of the ciphertext) ──────
        ct_commit: address
        subject_ref: address
        // ── lawful basis code (controller's taxonomy; e.g. 1=consent
        //    2=contract 3=legal-obligation 6=legitimate-interest) ──────
        lawful_basis: u64 = 0
        sealed_at: u64 = 0
        // ── lifecycle flags ───────────────────────────────────────────
        sealed: bool = false
        expiry_forced: bool = false
    }

    // ── seal (bind the retained record exactly once) ─────────────────────

    // Only the deployer (controller = owner) can seal. `ct_commit` is a
    // 32-byte hash of the off-chain ciphertext — never the data itself.
    // `lawful_basis` is the controller's Art. 6 basis code (recorded
    // immutably for the audit trail).
    fn seal(ct_commitment: address, subject: address, basis: u64) {
        require(caller == owner, "only the controller can seal this vault")
        require(self.sealed == false, "vault already sealed")
        require(basis > 0, "lawful basis code must be positive")

        self.ct_commit = ct_commitment
        self.subject_ref = subject
        self.lawful_basis = basis
        self.sealed_at = epoch
        self.sealed = true
        emit("gdpr vault sealed")
    }

    // ── early erasure (Art. 17 right-to-erasure / Art. 7(3) consent
    //    withdrawal) — fire the key-shred trigger NOW, before the
    //    natural retention deadline. Subject or controller may call.
    fn withdraw_consent() {
        require(self.sealed == true, "vault not yet sealed")
        require(self.expiry_forced == false, "erasure already requested")
        require(
            caller == self.subject_ref || caller == owner,
            "only the data subject or controller can request erasure"
        )

        self.expiry_forced = true
        emit("erasure-due (consent withdrawn): shred key for this ct_commit")
    }

    // ── lawful retention extension — logged immutably. The chain
    //    applies the actual energy refresh; this records it for audit.
    fn extend_retention() {
        require(self.sealed == true, "vault not yet sealed")
        require(caller == owner, "only the controller can extend retention")
        require(self.expiry_forced == false, "erasure already requested")

        emit("retention lawfully extended")
    }

    // ── read-only audit queries ──────────────────────────────────────────

    fn status() -> u64 {
        if self.sealed == false {
            return 0
        }
        if self.expiry_forced == true {
            return 1
        }
        return 2
    }

    fn lawful_basis_code() -> u64 {
        return self.lawful_basis
    }

    fn subject() -> address {
        return self.subject_ref
    }

    fn ct_commitment() -> address {
        return self.ct_commit
    }

    // Epoch the vault was sealed. The retention DEADLINE is driven by
    // the contract's own energy (deploy energy/half_life) — observable
    // via the terminal `evaporated` state, not computable in-script
    // (no decay primitive). This returns the start, for the audit log.
    fn sealed_epoch() -> u64 {
        return self.sealed_at
    }

    // ── lifecycle hooks ──────────────────────────────────────────────────

    // Retention period nearly elapsed — key-custody should prepare.
    on_grace() {
        emit("retention ending — prepare key-shred")
    }

    // Lawful retention extension applied (energy refreshed).
    on_refresh() {
        emit("retention lawfully extended")
    }

    // Terminal evaporation: the retention period has run out by
    // physics. This is the provable, tamper-evident key-shred trigger.
    // The off-chain key-custody service destroys the decryption key on
    // this event; the ciphertext is then permanently inaccessible.
    on_evaporate() {
        emit("erasure-due: shred key for this ct_commit")
    }
}
