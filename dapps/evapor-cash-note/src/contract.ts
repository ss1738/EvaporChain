// Single source of truth: `contracts/evaporscript/evaporcash_note.es`.
// Cargo pilot: `crates/evaporchain-script/tests/evaporcash_note_pilot.rs`.

export const EVAPORCASH_NOTE_SOURCE = `contract EvaporCashNote {
    state {
        // ── bearer ────────────────────────────────────────────────────
        holder: address
        // ── accounting (issue-time snapshot; the LIVE value is the
        //    contract's own \`energy\` builtin, which decays by physics) ──
        face: u64 = 0
        issued_at: u64 = 0
        // ── lifecycle flags ───────────────────────────────────────────
        sealed: bool = false
        spent: bool = false
    }

    // ── issuance ─────────────────────────────────────────────────────────

    // Bind the bearer exactly once. Only the deployer (creator = owner)
    // can issue. \`face_value\` is a snapshot of the energy the deployer
    // funded the note with — the spendable value thereafter is the
    // contract's own decaying \`energy\`, never this number.
    fn issue(to: address, face_value: u64) {
        require(caller == owner, "only issuer can issue this note")
        require(self.sealed == false, "note already issued")
        require(face_value > 0, "face value must be positive")

        self.holder = to
        self.face = face_value
        self.issued_at = epoch
        self.sealed = true
        emit("note issued")
    }

    // ── circulation ──────────────────────────────────────────────────────

    // Spend the note. Only the current holder can spend; a note can be
    // spent exactly once. This retires the on-chain note and emits the
    // event the off-chain coordinator watches to reissue a fresh note
    // to \`to\` carrying the live value (same emit-then-off-chain-credit
    // shape as future_self_vault.es::try_payout). Circulating BEFORE
    // the energy decays out is how the holder preserves value.
    fn spend(to: address) {
        require(self.sealed == true, "note not yet issued")
        require(self.spent == false, "note already spent")
        require(caller == self.holder, "only the holder can spend")

        self.holder = to
        self.spent = true
        emit("note spent — coordinator reissues live value to new holder")
    }

    // ── read-only queries ────────────────────────────────────────────────

    fn current_holder() -> address {
        return self.holder
    }

    fn is_spent() -> bool {
        return self.spent
    }

    // Issue-time face value (accounting only — NOT the spendable value).
    fn face_value() -> u64 {
        return self.face
    }

    // The spendable value RIGHT NOW. This is the whole point: it is the
    // contract's own \`energy\` builtin, decayed by the evaporation engine
    // (invariant #1 — read the builtin, never re-derive the formula).
    // A hoarded note's live_value bleeds toward zero by physics.
    fn live_value() -> u64 {
        return energy
    }

    fn issued_epoch() -> u64 {
        return self.issued_at
    }

    // ── lifecycle hooks ──────────────────────────────────────────────────

    // Note value is running low: spend (circulate) it before it
    // evaporates, or top it up (refresh).
    on_grace() {
        emit("note value low — spend or top up before it evaporates")
    }

    // Energy was refreshed (a top-up). The note's live value is
    // restored; the demurrage clock effectively resets.
    on_refresh() {
        emit("note value restored (top-up)")
    }

    // The note evaporated. If it was never spent, its value is lost to
    // hoarding — this is the demurrage taken to its physical limit and
    // the demo's whole punchline. Off-chain coordinator reads
    // \`spent == false\` at evaporation and records the forfeiture.
    on_evaporate() {
        if self.spent == false {
            emit("note evaporated — value lost to hoarding")
        }
    }
}`;
