// ErasureAttestation — EvaporScript contract for Proof-of-Erasure-as-a-
// Service (the AI machine-unlearning / verifiable-deletion frontier).
// See research/GDPR_ERASURE_ARCHITECTURE.md (model A lineage).
//
// WHY THIS EXISTS (research-grounded): the unsolved, UNSTANDARDISED
// problem in the right-to-be-forgotten / AI-unlearning space is *proving*
// deletion happened — "no method is fast, complete, AND verifiable"
// (EDPS/NeurIPS/IAPP). Regulators compel it (GDPR Art.17; California
// AB 1008 for data embedded in models). NIST SP 800-88 answers the
// general case with a "Certificate of Media Disposition": data ref,
// sanitization METHOD, VERIFICATION result, who/when, retained as
// tamper-evident proof. This contract is that certificate, on-chain
// and tamper-evident — one attestation = one mortal instance.
//
// HONEST SCOPE (the founding constraint, verified — Dead Drop §9):
// the chain does NOT byte-erase and does NOT perform machine
// unlearning. It holds NO personal data — only a 32-byte commitment.
// It is the *verifiable attestation / proof layer*: an immutable,
// tamper-evident record that a deletion obligation was met (or that
// its window terminally closed). The actual sanitization / unlearning
// happens off-chain (crypto-shred, SISA retrain, HSM key destruction);
// this is the audit-grade certificate a DPO/regulator verifies.
//
// Deploy via DeployScript with energy/half_life sized to the
// obligation/retention window; the instance's own decay is the
// tamper-evident clock. All state bounded (two addresses, four
// scalars, two flags) — inside EvaporScript's totality contract.

contract ErasureAttestation {
    state {
        // ── commitment (NIST: media/data identifier — a hash, NOT data) ──
        data_ref: address
        subject_ref: address
        // ── obligation basis: 1=GDPR-Art17 2=CCPA/AB1008 3=NIST-program ─
        obligation_basis: u64 = 0
        // ── sanitization method (NIST 800-88): 1=crypto-shred 2=clear
        //    3=purge 4=destroy 5=ML-unlearn(SISA/approx) ──────────────────
        method: u64 = 0
        sealed_at: u64 = 0
        attested_at: u64 = 0
        // ── lifecycle flags ───────────────────────────────────────────
        sealed: bool = false
        attested: bool = false
    }

    // ── 1. open the attestation exactly once (controller = owner) ────────
    // ct/data is encrypted & sanitized OFF-chain; only its commitment +
    // the obligation/method metadata are recorded here.
    fn seal(data_commitment: address, subject: address, basis: u64, sanitize_method: u64) {
        require(caller == owner, "only the controller can open this attestation")
        require(self.sealed == false, "attestation already opened")
        require(basis > 0, "obligation basis code must be positive")
        require(sanitize_method > 0, "sanitization method code must be positive")

        self.data_ref = data_commitment
        self.subject_ref = subject
        self.obligation_basis = basis
        self.method = sanitize_method
        self.sealed_at = epoch
        self.sealed = true
        emit("erasure attestation opened")
    }

    // ── 2. attest erasure: the controller has performed + verified the
    //    off-chain sanitization (NIST: "verify data unavailability and
    //    provide certification"). Records the immutable, tamper-evident
    //    proof event a regulator/DPO reads. Owner-only, once.
    fn attest_erasure(verification_code: u64) {
        require(self.sealed == true, "attestation not yet opened")
        require(self.attested == false, "erasure already attested")
        require(caller == owner, "only the controller can attest erasure")
        require(verification_code > 0, "verification result code must be positive")

        self.attested = true
        self.attested_at = epoch
        emit("PROOF-OF-ERASURE: obligation honoured, off-chain sanitization verified")
    }

    // ── read-only audit queries (the certificate of disposition) ─────────

    // 0 not-opened · 1 open (in obligation window, not yet attested) ·
    // 2 attested (erasure proven)
    fn status() -> u64 {
        if self.sealed == false {
            return 0
        }
        if self.attested == true {
            return 2
        }
        return 1
    }

    fn obligation_basis_code() -> u64 {
        return self.obligation_basis
    }

    fn method_code() -> u64 {
        return self.method
    }

    fn subject() -> address {
        return self.subject_ref
    }

    fn data_commitment() -> address {
        return self.data_ref
    }

    fn attested_epoch() -> u64 {
        return self.attested_at
    }

    fn sealed_epoch() -> u64 {
        return self.sealed_at
    }

    // ── lifecycle hooks ──────────────────────────────────────────────────

    // Obligation window nearly elapsed without an attestation —
    // controller is at risk of a missed-deletion compliance event.
    on_grace() {
        emit("obligation window ending — attest erasure or risk non-compliance")
    }

    on_refresh() {
        emit("obligation window lawfully extended")
    }

    // Terminal: the obligation window closed (by physics). If no
    // attestation was recorded, this is the immutable, tamper-evident
    // record that the deadline lapsed un-attested (a regulator-grade
    // negative proof). If attested, the proof already stands.
    on_evaporate() {
        if self.attested == false {
            emit("erasure-attestation: obligation window CLOSED with no attestation")
        }
    }
}
