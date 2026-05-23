// SinghResonance (Vital-Sign NFTs) — §A5.3 NFT Primitive.
//
// λ inversely coupled to engagement. Loved art slows toward immortality;
// ignored art accelerates toward zero.
//
// Attention model (V1 — explicit on-chain engagement events):
//   Anyone can call `register_engagement(weight)` to deposit attention.
//   Attention itself decays via hyperbolic window:
//     attention_now = cached_attention * attention_hl / (elapsed + attention_hl)
//   At elapsed=0: full; at elapsed=attention_hl: half (one half-life);
//   at elapsed→∞: 0. "Yesterday's likes evaporate just like the token
//   they are trying to save."
//
// Coupling formula (doctrine defaults — §A5.3):
//   attention = 0:         eff_hl = base_hl * 50 / 100    (0.5× — ignored decays 2× faster)
//   0 < attention ≤ sat:   eff_hl = base_hl * (50 + 50 * attention / sat) / 100  (ramp)
//   attention > sat:       eff_hl = base_hl * (100 + 700 * above / (above + sat)) / 100  (→8×)
//   where above = attention − saturation.
//
// Key invariants (proven by evaporchain-singh-resonance Rust tests):
//   1. Ignored (attention=0): eff_hl = 0.5× base_hl (decays 2× faster than base).
//   2. At saturation: eff_hl = base_hl (natural base rate).
//   3. Well-loved (attention >> sat): eff_hl → 8× base_hl (asymptote — "loved slows").
//   4. anti-Black-Mirror: can approach but never reach max. Capped at 8×.
//
// Cultural lineage: Tristan Harris (attention economy critique);
// Jenny Odell "How to Do Nothing"; Jaron Lanier on digital decay.
//
// INVENTION_STACK.md §A5.3: Singh-Resonance (Vital-Sign NFTs).
// Press claim: "loved art slows toward immortality; ignored art accelerates toward zero."

contract SinghResonance {
    state {
        // NFT identity — sealed once at mint, immutable thereafter.
        name: string = ""
        collection: string = ""
        metadata: string = ""
        sealed: bool = false

        // Ownership — mutable only via transfer.
        holder: address
        transfer_count: u64 = 0

        // Engagement window (decaying attention budget).
        // cached_attention = attention as of attention_anchor epoch.
        // Effective attention at epoch E: cached_attention * attention_hl / (elapsed + attention_hl)
        cached_attention: u64 = 0
        attention_anchor: u64 = 0
        attention_hl: u64 = 0       // attention half-life (epochs)

        // Coupling params — frozen at mint.
        base_hl: u64 = 0            // token's base half-life (epochs)
        saturation: u64 = 0         // attention level that maps to mid scale (1.0×)

        // Lifetime engagement audit trail.
        engagement_total: u64 = 0

        // Witness telemetry — snapshot attention + effective_hl at first two witness calls.
        // Snapshots are the only way to observe the VM-computed coupling result in state.
        witness_count: u64 = 0
        snapshot1_attention: u64 = 0
        snapshot1_eff_hl: u64 = 0
        snapshot2_attention: u64 = 0
        snapshot2_eff_hl: u64 = 0
    }

    // Mint exactly once. Owner-only.
    // `nft_base_hl`     — token's base half-life (the natural decay rate at mid-engagement).
    // `nft_attention_hl`— half-life of the attention window itself.
    // `nft_saturation`  — attention units that produce mid scale (1.0× base_hl).
    fn set_metadata(
        nft_name: string,
        nft_collection: string,
        nft_metadata: string,
        recipient: address,
        nft_base_hl: u64,
        nft_attention_hl: u64,
        nft_saturation: u64
    ) {
        require(caller == owner, "only minter can seal")
        require(self.sealed == false, "resonance token already minted")
        require(nft_base_hl > 0, "base half-life must be positive")
        require(nft_attention_hl > 0, "attention half-life must be positive")
        require(nft_saturation > 0, "saturation must be positive")

        self.name = nft_name
        self.collection = nft_collection
        self.metadata = nft_metadata
        self.holder = recipient
        self.base_hl = nft_base_hl
        self.attention_hl = nft_attention_hl
        self.saturation = nft_saturation
        self.attention_anchor = epoch
        self.sealed = true
        emit("resonance.minted")
    }

    // Register engagement. Anyone can call; weight is the attention deposited.
    // Decays existing attention to the current epoch first, then adds weight.
    // Re-anchors attention_anchor to current epoch so future decay is measured
    // from now. "Yesterday's likes evaporate" — the window has memory of only
    // recent engagement.
    fn register_engagement(weight: u64) {
        require(self.sealed == true, "resonance token not yet minted")
        require(weight > 0, "engagement weight must be positive")
        let elapsed = epoch - self.attention_anchor
        let decayed = self.cached_attention * self.attention_hl / (elapsed + self.attention_hl)
        self.cached_attention = decayed + weight
        self.attention_anchor = epoch
        self.engagement_total += weight
        emit("resonance.engaged")
    }

    // Record a witness event. Anyone can call.
    // Computes attention_now (decayed from anchor) and effective_hl via the
    // coupling formula, then stores both into snapshot1 (first call) or
    // snapshot2 (second call). Snapshots are the doctrinal proof probes —
    // the only way to read the on-chain coupling result from state.
    fn witness() {
        require(self.sealed == true, "resonance token not yet minted")
        let elapsed = epoch - self.attention_anchor
        let attention_now = self.cached_attention * self.attention_hl / (elapsed + self.attention_hl)

        // Piecewise coupling formula.
        // Default: attention=0 → min scale (0.5×). Local re-assigned by if-branches.
        let eff_hl = self.base_hl * 50 / 100
        if attention_now > 0 {
            if attention_now <= self.saturation {
                // Linear ramp: min → mid as attention goes 0 → saturation.
                eff_hl = self.base_hl * (50 + 50 * attention_now / self.saturation) / 100
            }
            if attention_now > self.saturation {
                // Asymptote toward 8× as attention >> saturation.
                let above = attention_now - self.saturation
                let approach = 700 * above / (above + self.saturation)
                eff_hl = self.base_hl * (100 + approach) / 100
            }
        }

        if self.witness_count == 0 {
            self.snapshot1_attention = attention_now
            self.snapshot1_eff_hl = eff_hl
        }
        if self.witness_count == 1 {
            self.snapshot2_attention = attention_now
            self.snapshot2_eff_hl = eff_hl
        }
        self.witness_count += 1
        emit("resonance.witnessed")
    }

    // Transfer ownership. Only the current holder can call.
    // Transfer does not affect the engagement window or coupling state.
    fn transfer(to: address) {
        require(self.sealed == true, "resonance token not yet minted")
        require(caller == self.holder, "only current holder can transfer")
        self.holder = to
        self.transfer_count += 1
        emit("resonance.transferred")
    }

    // Proves the token is "loved": current attention ≥ saturation.
    // At saturation, eff_hl = base_hl (natural rate); above it, eff_hl > base_hl.
    // Reverts if attention is below saturation — proving the gate non-vacuously.
    fn require_loved() {
        require(self.sealed == true, "resonance token not yet minted")
        let elapsed = epoch - self.attention_anchor
        let attention_now = self.cached_attention * self.attention_hl / (elapsed + self.attention_hl)
        require(attention_now >= self.saturation, "token not loved — attention below saturation")
        emit("resonance.loved.confirmed")
    }

    // Proves the token is "ignored": no engagement ever registered.
    // An ignored token decays at 0.5× its base half-life (eff_hl = base_hl/2).
    // Reverts if any engagement has been registered.
    fn require_ignored() {
        require(self.sealed == true, "resonance token not yet minted")
        require(self.engagement_total == 0, "token has been engaged — not ignored")
        emit("resonance.ignored.confirmed")
    }

    on_grace() {
        emit("resonance.energy.low — witness more; love this token")
    }

    on_refresh() {
        emit("resonance.refreshed")
    }

    on_evaporate() {
        emit("resonance.evaporated — the art died unseen")
    }
}
