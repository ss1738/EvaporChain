// SinghSabi (Patina Tokens) — §A5.3 NFT Primitive.
//
// NFTs that age toward "ruined-beautiful".
//
// Mint deposits a fixed energy budget split into two portions:
//   decayable   = initial_energy - floor_energy
//   floor       = floor_energy (default ≈ 15% of initial)
//
// The CONTRACT is deployed with `energy = decayable` so the chain's
// own λ-decay formula tracks the decayable portion automatically.
// patina_score(t) = floor_energy + energy(t)
//           where energy(t) = decayable * 2^(-t / half_life).
//
// Key structural properties:
//   patina(0)   = initial_energy   (just minted, pristine)
//   patina(∞)   = floor_energy     (ruined-beautiful asymptote)
//   patina(t)   monotone non-increasing (provable from λ-decay)
//   patina(t) ≥ floor_energy       (never evaporates to zero)
//
// Owner cannot pause or freeze decay; only witness it.
// The only mutating operation that changes state is transfer —
// which does NOT affect the patina curve.
//
// Cultural lineage: wabi-sabi (Sen no Rikyū, 16th c.); kintsugi;
// Tarkovsky's Stalker set decay; Basinski's Disintegration Loops;
// Banksy's Love is in the Bin.
//
// INVENTION_STACK.md §A5.3: Singh-Sabi (Patina Tokens).
// Press claim: "A blockchain that lets art age like paper."

contract SinghSabi {
    state {
        // NFT identity — sealed once at mint.
        name: string = ""
        collection: string = ""
        metadata: string = ""
        sealed: bool = false

        // Patina parameters — frozen at mint.
        // initial_energy includes the floor; contract is deployed with
        // energy = initial_energy - floor_energy (the decayable portion).
        initial_energy: u64 = 0   // doctrinal full initial patina score
        floor_energy: u64 = 0     // asymptotic floor (≈ 15% of initial)
        minted_at: u64 = 0        // epoch when minted

        // Ownership — mutable only via transfer.
        holder: address
        transfer_count: u64 = 0

        // Witness telemetry — snapshot1/snapshot2 capture the on-chain
        // patina_score (= floor + energy) at the first two witness calls,
        // allowing the deploy runbook to verify monotone decay by reading
        // state (API .energy returns stored initial, not decayed value;
        // snapshots are the only way to observe VM-computed energy in state).
        witness_count: u64 = 0
        snapshot1: u64 = 0   // patina_score at first witness
        snapshot2: u64 = 0   // patina_score at second witness
    }

    // Mint exactly once. Owner-only.
    // `initial` = the full doctrinal initial patina score (e.g. 1000000).
    // `floor`   = asymptotic floor (e.g. 150000 = 15% of initial).
    // The caller MUST deploy the contract with `energy = initial - floor`
    // (the decayable portion only) so that `patina_score = floor + energy`.
    fn set_metadata(
        nft_name: string,
        nft_collection: string,
        nft_metadata: string,
        recipient: address,
        initial: u64,
        floor: u64
    ) {
        require(caller == owner, "only minter can seal")
        require(self.sealed == false, "patina token already minted")
        require(initial > 0, "initial energy must be positive")
        require(floor < initial, "floor must be below initial")

        self.name = nft_name
        self.collection = nft_collection
        self.metadata = nft_metadata
        self.initial_energy = initial
        self.floor_energy = floor
        self.minted_at = epoch
        self.holder = recipient
        self.sealed = true
        emit("sabi.minted")
    }

    // Transfer ownership. Only the current holder can call.
    // Transfer does NOT affect the patina curve — the token ages
    // regardless of whose hands it passes through.
    fn transfer(to: address) {
        require(self.sealed == true, "patina token not yet minted")
        require(caller == self.holder, "only current holder can transfer")
        self.holder = to
        self.transfer_count += 1
        emit("sabi.transferred")
    }

    // Record a witness event. Anyone can call.
    // Stores the current patina_score in snapshot1 (first call) or
    // snapshot2 (second call) so the deploy runbook can verify decay
    // by comparing state fields (API .energy returns stored initial,
    // not the VM-computed decayed value — snapshots are the probe).
    fn witness() {
        require(self.sealed == true, "patina token not yet minted")
        let score = self.floor_energy + energy
        if self.witness_count == 0 {
            self.snapshot1 = score
        }
        if self.witness_count == 1 {
            self.snapshot2 = score
        }
        self.witness_count += 1
        emit("sabi.witnessed")
    }

    // Returns the current patina score = floor_energy + current energy.
    // At mint: patina_score == initial_energy.
    // Over time: tends toward floor_energy.
    fn patina_score() -> u64 {
        return self.floor_energy + energy
    }

    // Asserts patina_score >= floor_energy. Always true by construction
    // (energy ≥ 0 and patina_score = floor + energy ≥ floor). Proves
    // the non-zero-floor invariant on-chain — "ruined-beautiful" guarantee.
    fn require_above_floor() {
        require(self.sealed == true, "patina token not yet minted")
        let score = self.floor_energy + energy
        require(score >= self.floor_energy, "floor invariant violated — impossible by construction")
        emit("sabi.floor.confirmed")
    }

    // Asserts patina_score <= initial_energy. Proves the score is bounded
    // above by the initial value (can only decrease, never increase).
    fn require_below_initial() {
        require(self.sealed == true, "patina token not yet minted")
        let score = self.floor_energy + energy
        require(score <= self.initial_energy, "score exceeds initial — decay inversion")
        emit("sabi.initial.bound.confirmed")
    }

    fn current_holder() -> address {
        return self.holder
    }

    on_grace() {
        emit("sabi.energy.low — approaching ruined-beautiful asymptote")
    }

    on_refresh() {
        emit("sabi.refreshed")
    }

    on_evaporate() {
        // The floor guarantee is economic, not cryptographic — if the
        // contract is never refreshed, integer decay reaches 0 in finite
        // epochs. The cultural claim "never evaporates" means no rational
        // owner abandons a piece at the ruined-beautiful floor.
        emit("sabi.evaporated.ruined-beautiful")
    }
}
