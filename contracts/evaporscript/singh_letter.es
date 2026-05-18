// SinghLetter (ChildKey §A5.5) — Inverted-Decay Time-Lock.
//
// Parents seal text/voice/photo/video to a child, locked by
// age-of-recipient (not date). Chain holds the ciphertext commitment;
// the decryption key materializes when chain time proves the child has
// reached unlock age. Parent dies? Seal still opens on schedule.
//
// Inverted decay (doctrine):
//   Standard decay: energy DRAINS toward zero as elapsed time grows.
//   ChildKey (inverted): energy-to-unlock GROWS from 0 → threshold
//   over the recipient's lifetime. Same primitive, opposite sign.
//
//   Countdown: epochs_until_unlock = unlock_epoch − epoch_now  (≥ 0).
//   At epoch_now ≥ unlock_epoch: countdown reaches zero → gate opens.
//
// Unlock formula (immutable once sealed):
//   unlock_epoch = recipient_birth_epoch + unlock_age_years * epochs_per_year
//
// epochs_per_year is frozen at seal time so chain-time changes never
// retro-shift the unlock date. "Parent dies? Seal still opens on schedule."
//
// Letter lifecycle (letter_status):
//   0 = sealed  (waiting for child to come of age)
//   1 = opened  (recipient came of age; letter is readable)
//   Sealed → Opened only; no re-seal.
//
// Witness telemetry (same snapshot pattern as SinghResonance):
//   First two witness_countdown() calls write to snapshot1 / snapshot2.
//   Doctrinal proof probes: the only way to read the countdown result
//   from state without querying a local variable directly.
//
// Cultural lineage: time-capsule tradition; legal future-interests doctrine;
// Shamir 1979 threshold secret sharing.
//
// INVENTION_STACK.md §A5.5: Singh Letter (primitive), ChildKey (unlock-by-age
// key derivation), Singh Vault (blob layer). Build first.
// Press claim: "Parent dies? Seal still opens on schedule."

contract SinghLetter {
    state {
        // Initialisation gate.
        sealed: bool = false

        // Unlock parameters — frozen at seal_letter(). Immutable.
        recipient_birth_epoch: u64 = 0
        unlock_age_years: u64 = 0
        epochs_per_year: u64 = 0
        // Derived at seal time: birth_epoch + age_years * epy.
        // Stored so chain-time changes don't retroactively shift threshold.
        unlock_epoch: u64 = 0

        // Payload commitment — BLAKE3 hash of off-chain ciphertext blob.
        payload_hash: string = ""

        // Lifecycle: 0=sealed, 1=opened.
        letter_status: u64 = 0
        // Epoch when opened (0 until opened).
        opened_at_epoch: u64 = 0
        // Epoch when sealed (informational only; not used in unlock math).
        sealed_at_epoch: u64 = 0

        // Witness telemetry — written by first two witness_countdown() calls.
        witness_count: u64 = 0
        // snapshot_remaining: epochs left until unlock (0 once unlockable).
        snapshot1_remaining: u64 = 0
        // snapshot_unlockable: 1 if countdown reached zero, else 0.
        snapshot1_unlockable: u64 = 0
        snapshot2_remaining: u64 = 0
        snapshot2_unlockable: u64 = 0
    }

    // Seal the letter once. Owner-only.
    // birth_epoch: recipient's birth in chain epochs (asserted by DID layer).
    // age_years:   age (in years) at which the letter unlocks.
    // epy:         epochs per year (frozen at seal; never retro-shifts dates).
    // phash:       BLAKE3 hash of the off-chain ciphertext blob.
    fn seal_letter(birth_epoch: u64, age_years: u64, epy: u64, phash: string) {
        require(caller == owner, "only sender can seal")
        require(self.sealed == false, "letter already sealed")
        require(age_years > 0, "unlock age must be positive")
        require(epy > 0, "epochs_per_year must be positive")
        self.recipient_birth_epoch = birth_epoch
        self.unlock_age_years = age_years
        self.epochs_per_year = epy
        self.unlock_epoch = birth_epoch + age_years * epy
        self.payload_hash = phash
        self.sealed_at_epoch = epoch
        self.letter_status = 0
        self.sealed = true
        emit("letter.sealed")
    }

    // Open the letter. Anyone can call; the proof gate enforces timing.
    // Reverts unless epoch >= unlock_epoch (inverted-decay threshold reached).
    // Transition: sealed (0) → opened (1). Re-opening an opened letter reverts.
    fn open_letter() {
        require(self.sealed == true, "letter not yet sealed")
        require(self.letter_status == 0, "letter already opened")
        require(epoch >= self.unlock_epoch, "not yet unlockable — energy-to-unlock below threshold")
        self.letter_status = 1
        self.opened_at_epoch = epoch
        emit("letter.opened")
    }

    // Record a witness event. Anyone can call.
    // Computes the inverted-decay countdown (epochs_until_unlock) and
    // the unlockable flag, then stores both into snapshot1 (first call)
    // or snapshot2 (second call). Snapshots are the doctrinal proof probes.
    fn witness_countdown() {
        require(self.sealed == true, "letter not yet sealed")
        let remaining = 0
        if self.unlock_epoch > epoch {
            remaining = self.unlock_epoch - epoch
        }
        let unlockable = 0
        if self.unlock_epoch <= epoch {
            unlockable = 1
        }
        if self.witness_count == 0 {
            self.snapshot1_remaining = remaining
            self.snapshot1_unlockable = unlockable
        }
        if self.witness_count == 1 {
            self.snapshot2_remaining = remaining
            self.snapshot2_unlockable = unlockable
        }
        self.witness_count += 1
        emit("letter.witnessed")
    }

    // Proof gate: proves the letter is still sealed (not yet opened).
    // Used by systems that must verify the child has not yet come of age.
    fn require_sealed() {
        require(self.sealed == true, "letter not yet sealed")
        require(self.letter_status == 0, "letter already opened")
        emit("letter.sealed.confirmed")
    }

    // Proof gate: proves the letter has been opened (child came of age).
    // Used by systems that gate on the recipient having reached unlock age.
    fn require_opened() {
        require(self.sealed == true, "letter not yet sealed")
        require(self.letter_status == 1, "letter not yet opened")
        emit("letter.opened.confirmed")
    }

    on_grace() {
        emit("letter.approaching.unlock — countdown nearly zero")
    }

    on_refresh() {
        emit("letter.energy.refreshed")
    }

    on_evaporate() {
        emit("letter.evaporated — sealed forever, never opened")
    }
}
