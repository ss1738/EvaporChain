// SinghLineage (EvaporWallet-Lineage) — §A5.4 Graduated Inheritance.
//
// Wallet second screen: family tree showing primary key + designated
// successors with dormancy thresholds. "If I'm silent 90 days, my
// daughter's key gains 25% authority; 180 days, 50%; 365 days, full."
// Per-asset posthumous designation. Real-time visual of your own
// digital mortality.
//
// Graduated dormancy model (integer-only; FULL_AUTHORITY_BP = 10000):
//
//   dormancy = epoch_now − last_seen_epoch
//   tier_share = highest DormancyTier whose threshold ≤ dormancy
//   effective_bp = tier_share * successor_weight_bp / 10000
//
//   Tiers (set at initialise, doctrine defaults):
//     tier 0: threshold = tier1_epochs, share = 2500 bp  (25%)
//     tier 1: threshold = tier2_epochs, share = 5000 bp  (50%)
//     tier 2: threshold = tier3_epochs, share = 10000 bp (100%)
//
//   Invariant: total_weight_bp across all successors ≤ 10000.
//   At full dormancy, total authority = total_weight_bp × ladder_max / 10000
//   ≤ 10000 (cannot exceed full authority — no double-spend).
//
// What's structurally different from Apple Legacy Contact / Google IAM:
//   Those are binary (on/off). Singh-Lineage is graduated: authority
//   accrues continuously as silence lengthens. Touch resets dormancy.
//
// Witness telemetry (same snapshot pattern as SinghResonance/SinghLetter):
//   witness_authority(addr_u8) writes snapshot1 (first call) or
//   snapshot2 (second call): (addr, authority_bp, dormancy_at_call).
//
// Cultural lineage: Apple Legacy Contact (closest precedent — reactive,
// not graduated); Google Inactive Account Manager.
//
// INVENTION_STACK.md §A5.4: Singh-Lineage (EvaporWallet-Lineage).
// Press claim: "crypto solves inheritance." Highest mainstream-press
// potency of the wallet set — FT, NYT personal finance, Atlantic.

contract SinghLineage {
    state {
        // Initialisation gate.
        sealed: bool = false

        // Issuer's last alive signal. Set at initialise; updated by touch().
        last_seen_epoch: u64 = 0

        // Dormancy ladder — 3 tiers, set once at initialise.
        // Doctrine: [(tier1_epochs, 25%), (tier2_epochs, 50%), (tier3_epochs, 100%)].
        tier_threshold: map[u64 -> u64]  // tier_index → epochs_dormant threshold
        tier_share_bp: map[u64 -> u64]   // tier_index → authority share (out of 10000)
        tier_count: u64 = 0

        // Successors: addr_u8 → weight_bp (allocation out of 10000).
        successor_weight: map[u64 -> u64]
        // Presence map (0=absent, 1=registered). Needed because map default
        // is 0 for unregistered keys — can't use weight==0 to mean absent.
        successor_present: map[u64 -> u64]
        successor_count: u64 = 0
        total_weight_bp: u64 = 0

        // Witness snapshots — written by witness_authority().
        witness_count: u64 = 0
        snapshot1_addr: u64 = 0
        snapshot1_authority_bp: u64 = 0
        snapshot1_dormancy: u64 = 0
        snapshot2_addr: u64 = 0
        snapshot2_authority_bp: u64 = 0
        snapshot2_dormancy: u64 = 0
    }

    // Initialise once. Sets the three-tier dormancy ladder (doctrine defaults).
    // tier1_epochs < tier2_epochs < tier3_epochs enforced.
    // Shares are hardcoded: 2500 / 5000 / 10000 bp (25% / 50% / 100%).
    // Sets last_seen_epoch = now (issuer is alive at init time).
    fn initialise(tier1_epochs: u64, tier2_epochs: u64, tier3_epochs: u64) {
        require(caller == owner, "only issuer can initialise")
        require(self.sealed == false, "already initialised")
        require(tier1_epochs > 0, "tier1 must be positive")
        require(tier2_epochs > tier1_epochs, "tiers must be strictly ascending")
        require(tier3_epochs > tier2_epochs, "tiers must be strictly ascending")
        self.tier_threshold[0] = tier1_epochs
        self.tier_share_bp[0] = 2500
        self.tier_threshold[1] = tier2_epochs
        self.tier_share_bp[1] = 5000
        self.tier_threshold[2] = tier3_epochs
        self.tier_share_bp[2] = 10000
        self.tier_count = 3
        self.last_seen_epoch = epoch
        self.sealed = true
        emit("lineage.initialised")
    }

    // Add a successor. Issuer-only.
    // addr_u8:   successor's address index (first byte; testnet 0..255).
    // weight_bp: heir's inheritance weight in basis points (out of 10000).
    // Total across all successors must not exceed 10000.
    fn add_successor(addr_u8: u64, weight_bp: u64) {
        require(self.sealed == true, "not initialised")
        require(caller == owner, "only issuer can add successors")
        require(weight_bp > 0, "weight must be positive")
        require(self.successor_present[addr_u8] == 0, "successor already registered")
        require(self.total_weight_bp + weight_bp <= 10000, "total weight would exceed 10000 bp")
        self.successor_weight[addr_u8] = weight_bp
        self.successor_present[addr_u8] = 1
        self.total_weight_bp = self.total_weight_bp + weight_bp
        self.successor_count += 1
        emit("lineage.successor.added")
    }

    // Issuer signals they're alive. Resets dormancy to zero.
    // Any signed tx by the issuer acts as a touch in practice.
    // Reverts if called backwards in time.
    fn touch() {
        require(self.sealed == true, "not initialised")
        require(caller == owner, "only issuer can touch")
        require(epoch >= self.last_seen_epoch, "cannot touch backwards in time")
        self.last_seen_epoch = epoch
        emit("lineage.touched")
    }

    // Witness: compute authority for addr_u8 at current epoch.
    // Stores (addr, authority_bp, dormancy) into snapshot1 (first call)
    // or snapshot2 (second call). Anyone can call; pure-function result.
    // effective_bp = tier_share_bp_at_dormancy * weight_bp / 10000.
    fn witness_authority(addr_u8: u64) {
        require(self.sealed == true, "not initialised")
        let dormancy = epoch - self.last_seen_epoch
        let share_bp = 0
        let i = 0
        while i < self.tier_count {
            if self.tier_threshold[i] <= dormancy {
                share_bp = self.tier_share_bp[i]
            }
            i = i + 1
        }
        let weight_bp = self.successor_weight[addr_u8]
        let eff_bp = share_bp * weight_bp / 10000
        if self.witness_count == 0 {
            self.snapshot1_addr = addr_u8
            self.snapshot1_authority_bp = eff_bp
            self.snapshot1_dormancy = dormancy
        }
        if self.witness_count == 1 {
            self.snapshot2_addr = addr_u8
            self.snapshot2_authority_bp = eff_bp
            self.snapshot2_dormancy = dormancy
        }
        self.witness_count += 1
        emit("lineage.witnessed")
    }

    // Proof gate: proves first witness showed non-zero authority.
    // Passes when snapshot1_authority_bp > 0 (issuer has been silent
    // long enough to cross at least tier1 and has a registered successor).
    fn require_authority() {
        require(self.sealed == true, "not initialised")
        require(self.witness_count > 0, "no witness yet")
        require(self.snapshot1_authority_bp > 0, "no authority accrued — issuer still active or no successor")
        emit("lineage.authority.confirmed")
    }

    on_grace() {
        emit("lineage.energy.low — issuer lineage fading")
    }

    on_refresh() {
        emit("lineage.refreshed")
    }

    on_evaporate() {
        emit("lineage.evaporated — lineage abandoned")
    }
}
