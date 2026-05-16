// SHLM — Singh Skill Half-Life Market (EvaporScript contract).
//
// Per INVENTION_STACK.md §A5.2: one contract instance = one skill class.
// Deploy a fresh instance per skill (e.g. one for Python, one for COBOL).
//
// Core mechanic: credentials decay at the class's half_life rate. A Python
// credential attested 400 epochs ago (≈1 half-life for Python) retains ~50%
// of its original level. A COBOL credential of the same age retains far more
// because COBOL's half_life is much longer. Employers post bounties with a
// minimum freshness_age (how many epochs ago the latest attestation must
// be) — not an absolute epoch, but a relative staleness cap.
//
// Freshness model (simplified for EvaporScript totality):
//   elapsed = epoch - cred.attested_at
//   Credential is "eligible" for a bounty when:
//     1. elapsed <= bounty.max_elapsed        (staleness gate)
//     2. cred.level >= bounty.min_level       (skill-level gate)
//   The off-chain coordinator computes the exact half-life-decayed score
//   via energy_at_epoch(level, half_life, elapsed) and runs the SDDC two-
//   axis clearing. This contract records the coordinator's final decision.
//
// Roles:
//   owner (= admin, = attestor) — issues and refreshes credentials.
//                                 In production, this is the exam-board
//                                 or employer-of-record address.
//   employers                   — post salary bounties.
//   holders                     — credential holders seeking opportunities.
//
// One contract per skill class keeps state bounded and cross-class logic
// clean. Aggregation across classes is the coordinator's responsibility.

contract SHLM {
    state {
        // ── skill class ───────────────────────────────────────────────
        skill_name: string = ""
        half_life: u64 = 0       // decay rate in epochs
        admin: address
        sealed: bool = false
        credential_count: u64 = 0

        // ── credential registry (per holder) ──────────────────────────
        // attested_at = 0 means no credential for that holder.
        cred_level: map[address -> u64]
        cred_attested_at: map[address -> u64]
        // presence flag avoids false-zero ambiguity for attested_at=0.
        cred_exists: map[address -> u64]

        // ── bounty registry (per employer) ────────────────────────────
        // max_elapsed: bounty requires credential attested within this
        // many epochs (e.g. 300 = "no more than 300 epochs old").
        bounty_max_elapsed: map[address -> u64]
        bounty_min_level: map[address -> u64]
        bounty_salary: map[address -> u64]
        bounty_active: map[address -> u64]   // 1=active 0=inactive
        bounty_count: u64 = 0

        // ── match log (per employer → matched holder) ─────────────────
        match_holder: map[address -> address]
        match_salary: map[address -> u64]
        match_at: map[address -> u64]
        match_exists: map[address -> u64]    // 1=matched 0=open
    }

    // ── class registration ───────────────────────────────────────────────

    // Configure the skill class exactly once. Only the deployer can call.
    // `half_life_epochs` is the half-life of this skill in consensus epochs.
    // Example calibration: Python ≈ 540, COBOL ≈ 2920, prompt-eng ≈ 120.
    fn register_class(name: string, half_life_epochs: u64) {
        require(caller == owner, "only admin can register class")
        require(self.sealed == false, "class already registered")
        require(half_life_epochs > 0, "half_life must be positive")

        self.skill_name = name
        self.half_life = half_life_epochs
        self.admin = owner
        self.sealed = true
        emit("skill class registered")
    }

    // ── credential issuance ──────────────────────────────────────────────

    // Issue a credential to `holder` at the current epoch. Admin-only.
    // Overwrites any existing credential (re-attestation path: refreshing
    // a credential is the same code-path as initial issuance when the
    // holder already has one).
    fn issue_credential(holder: address, level: u64) {
        require(self.sealed == true, "class not registered")
        require(caller == self.admin, "only admin can issue credentials")
        require(level > 0, "credential level must be positive")

        let is_new = 0
        if self.cred_exists[holder] == 0 {
            is_new = 1
        }
        self.cred_level[holder] = level
        self.cred_attested_at[holder] = epoch
        self.cred_exists[holder] = 1
        if is_new == 1 {
            self.credential_count += 1
        }
        emit("credential issued")
    }

    // Refresh an existing credential (bump attested_at, optionally update
    // level). Holder must already have a credential. Admin-only — the
    // assessment is off-chain; only the outcome lands here.
    fn refresh_credential(holder: address, new_level: u64) {
        require(self.sealed == true, "class not registered")
        require(caller == self.admin, "only admin can refresh credentials")
        require(self.cred_exists[holder] == 1, "no credential for this holder")
        require(new_level > 0, "level must be positive")

        self.cred_level[holder] = new_level
        self.cred_attested_at[holder] = epoch
        emit("credential refreshed")
    }

    // ── bounty market ────────────────────────────────────────────────────

    // Employer posts a bounty for fresh-skill holders.
    //   max_staleness — credential must have been attested within this
    //                   many epochs (e.g. 300 = "no older than 300 epochs").
    //   min_level     — original level at attestation must be >= this.
    //   salary_offer  — the offered reward / compensation amount (u64,
    //                   interpreted as energy units by the execution layer).
    fn post_bounty(max_staleness: u64, min_level: u64, salary_offer: u64) {
        require(self.sealed == true, "class not registered")
        require(max_staleness > 0, "staleness cap must be positive")
        require(min_level > 0, "min_level must be positive")
        require(salary_offer > 0, "salary offer must be positive")
        require(self.bounty_active[caller] == 0, "bounty already active — withdraw first")

        self.bounty_max_elapsed[caller] = max_staleness
        self.bounty_min_level[caller] = min_level
        self.bounty_salary[caller] = salary_offer
        self.bounty_active[caller] = 1
        self.bounty_count += 1
        emit("bounty posted")
    }

    // Employer withdraws an active bounty.
    fn withdraw_bounty() {
        require(self.sealed == true, "class not registered")
        require(self.bounty_active[caller] == 1, "no active bounty")

        self.bounty_active[caller] = 0
        self.bounty_max_elapsed[caller] = 0
        self.bounty_min_level[caller] = 0
        self.bounty_salary[caller] = 0
        emit("bounty withdrawn")
    }

    // ── matching ─────────────────────────────────────────────────────────

    // Record a coordinator-confirmed match between an employer and a holder.
    // The off-chain SDDC coordinator has:
    //   1. Verified holder.cred_level >= bounty.min_level
    //   2. Verified (epoch - cred_attested_at) <= bounty.max_elapsed
    //   3. Run two-axis Dutch clearing across all eligible bids
    //   4. Confirmed payment routing
    //
    // Any address may call (the coordinator is a service account); the
    // contract validates that the employer has an active bounty and the
    // holder has a valid credential satisfying the bounty conditions.
    fn record_match(employer: address, holder: address, agreed_salary: u64) {
        require(self.sealed == true, "class not registered")
        require(self.bounty_active[employer] == 1, "employer has no active bounty")
        require(self.cred_exists[holder] == 1, "holder has no credential")
        require(self.match_exists[employer] == 0, "bounty already matched")
        require(agreed_salary > 0, "salary must be positive")

        // On-chain eligibility check: both gates that the coordinator also ran.
        let elapsed = 0
        if epoch > self.cred_attested_at[holder] {
            elapsed = epoch - self.cred_attested_at[holder]
        }
        let max_elapsed = self.bounty_max_elapsed[employer]
        require(elapsed <= max_elapsed, "credential too stale for this bounty")

        let h_level = self.cred_level[holder]
        let min_level = self.bounty_min_level[employer]
        require(h_level >= min_level, "holder level below bounty requirement")

        self.match_holder[employer] = holder
        self.match_salary[employer] = agreed_salary
        self.match_at[employer] = epoch
        self.match_exists[employer] = 1
        self.bounty_active[employer] = 0
        emit("match recorded")
    }

    // ── read-only queries ────────────────────────────────────────────────

    fn skill() -> string {
        return self.skill_name
    }

    fn half_life_rate() -> u64 {
        return self.half_life
    }

    fn total_credentials() -> u64 {
        return self.credential_count
    }

    // Epochs since the holder's last attestation (staleness).
    fn credential_age(holder: address) -> u64 {
        if self.cred_exists[holder] == 0 {
            return 0
        }
        if epoch <= self.cred_attested_at[holder] {
            return 0
        }
        return epoch - self.cred_attested_at[holder]
    }

    fn credential_level(holder: address) -> u64 {
        return self.cred_level[holder]
    }

    fn has_credential(holder: address) -> u64 {
        return self.cred_exists[holder]
    }

    fn bounty_salary_offer(employer: address) -> u64 {
        return self.bounty_salary[employer]
    }

    fn bounty_is_active(employer: address) -> u64 {
        return self.bounty_active[employer]
    }

    fn match_winner(employer: address) -> address {
        return self.match_holder[employer]
    }

    fn match_agreed_salary(employer: address) -> u64 {
        return self.match_salary[employer]
    }

    // Returns 1 if holder satisfies the employer's active bounty conditions.
    // Pure check — no state change. Coordinator calls this before settling.
    fn is_eligible(holder: address, employer: address) -> u64 {
        if self.cred_exists[holder] == 0 {
            return 0
        }
        if self.bounty_active[employer] == 0 {
            return 0
        }
        let elapsed = 0
        if epoch > self.cred_attested_at[holder] {
            elapsed = epoch - self.cred_attested_at[holder]
        }
        if elapsed > self.bounty_max_elapsed[employer] {
            return 0
        }
        if self.cred_level[holder] < self.bounty_min_level[employer] {
            return 0
        }
        return 1
    }

    // ── lifecycle hooks ──────────────────────────────────────────────────

    // Energy entering grace: open bounties should be settled or withdrawn.
    on_grace() {
        emit("SHLM energy low — settle open bounties or refresh")
    }

    on_refresh() {
        emit("SHLM energy refreshed")
    }

    // Evaporation terminates the skill class registry on-chain.
    // Existing matched credentials persist in match_holder; open bounties
    // are implicitly withdrawn. Coordinator records the evaporation and
    // signals employers to re-post on a new class instance.
    on_evaporate() {
        emit("skill class evaporated — open bounties void")
    }
}
