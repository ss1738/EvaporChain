// SinghMigrant (Wanderwrits) — §A5.3 NFT Primitive.
//
// The NFT that dies if you keep it.
//
// Each Wanderwrit has a resting_threshold (epoch count). While it keeps
// moving through novel wallets it stays healthy. Stay still past
// threshold → check_vitals() returns STALE; past 2× threshold →
// CRITICAL. The contract's energy IS the NFT lifespan — when it
// evaporates, the Wanderwrit is gone.
//
// "Novel" = the recipient address has never held this NFT before.
// visited[] tracks every address that has ever received the NFT.
// Transferring back to a prior holder is legal but yields no credit.
//
// Transfer to a novel wallet emits "wanderwrit.novel.credit". The
// off-chain coordinator applies the actual energy top-up (fraction
// of current energy per evaporchain-singh-migrant::refund module).
// This contract is the authoritative on-chain record.
//
// Cultural lineage: Trobriand kula ring (Malinowski 1922); Marcel Mauss
// The Gift (1925); chain letters; the Olympic torch; geocaching.
//
// INVENTION_STACK.md §A5.3: Singh-Migrant (Wanderwrits).
// Press claim: "the NFT that dies if you keep it."

contract SinghMigrant {
    state {
        // NFT identity — sealed once at mint, immutable thereafter.
        name: string = ""
        collection: string = ""
        metadata: string = ""
        sealed: bool = false

        // Kula-ring mechanics.
        holder: address
        resting_threshold: u64 = 0   // epochs of idle before declared stale
        last_moved_epoch: u64 = 0    // epoch of last transfer (or mint)
        transfer_count: u64 = 0
        novel_transfer_count: u64 = 0

        // Novel-wallet registry: visited[addr] = 1 iff addr has ever held
        // this NFT. Defaults to U64(0) per EvaporScript map semantics.
        visited: map[address -> u64]
    }

    // Mint exactly once. Owner-only. Sets initial holder and resting threshold.
    // `recipient` is the initial owner — may be the minter or a buyer.
    fn set_metadata(
        nft_name: string,
        nft_collection: string,
        nft_metadata: string,
        recipient: address,
        threshold: u64
    ) {
        require(caller == owner, "only minter can seal")
        require(self.sealed == false, "wanderwrit already minted")
        require(threshold > 0, "resting threshold must be positive")

        self.name = nft_name
        self.collection = nft_collection
        self.metadata = nft_metadata
        self.resting_threshold = threshold
        self.holder = recipient
        self.last_moved_epoch = epoch
        self.visited[recipient] = 1
        self.sealed = true
        emit("wanderwrit.minted")
    }

    // Transfer to a new holder. Only the current holder can call.
    // Emits "wanderwrit.novel.credit" if recipient has never held this NFT.
    // Transfer to a prior holder is legal but yields no novel credit.
    fn transfer(to: address) {
        require(self.sealed == true, "wanderwrit not yet minted")
        require(caller == self.holder, "only current holder can transfer")

        if self.visited[to] == 0 {
            self.novel_transfer_count += 1
            emit("wanderwrit.novel.credit")
        }

        self.visited[to] = 1
        self.holder = to
        self.last_moved_epoch = epoch
        self.transfer_count += 1
        emit("wanderwrit.transferred")
    }

    // Requires the Wanderwrit to be healthy (rested < threshold).
    // Reverts if stale — proving the predicate is not yet satisfied.
    // Used by the deploy runbook to prove the gate under the happy path.
    fn require_healthy() {
        require(self.sealed == true, "wanderwrit not yet minted")
        let rested = epoch - self.last_moved_epoch
        require(rested < self.resting_threshold, "wanderwrit is stale — move it")
        emit("wanderwrit.healthy.confirmed")
    }

    // Requires the Wanderwrit to be stale (rested >= threshold).
    // Reverts if still healthy — proving the stale predicate non-vacuously.
    // Used by the deploy runbook to prove the gate is actually enforced.
    fn require_stale() {
        require(self.sealed == true, "wanderwrit not yet minted")
        let rested = epoch - self.last_moved_epoch
        require(rested >= self.resting_threshold, "wanderwrit is not yet stale")
        emit("wanderwrit.stale.confirmed")
    }

    // Returns 0=healthy, 1=stale, 2=critical.
    fn check_vitals() -> u64 {
        require(self.sealed == true, "wanderwrit not yet minted")
        let rested = epoch - self.last_moved_epoch
        let double_threshold = self.resting_threshold + self.resting_threshold
        if rested >= double_threshold {
            emit("wanderwrit.critical")
            return 2
        }
        if rested >= self.resting_threshold {
            emit("wanderwrit.stale")
            return 1
        }
        emit("wanderwrit.healthy")
        return 0
    }

    fn rested_epochs() -> u64 {
        if self.sealed == false {
            return 0
        }
        return epoch - self.last_moved_epoch
    }

    fn current_holder() -> address {
        return self.holder
    }

    fn novel_transfers() -> u64 {
        return self.novel_transfer_count
    }

    // Asserts that `addr` is a PRIOR holder (already in the visited map).
    // Reverts if the address has never held this NFT.
    // Used by the deploy runbook to prove that a second transfer to a
    // prior holder would yield no novel credit (visited[addr] == 1).
    fn assert_prior_holder(addr: address) {
        require(self.sealed == true, "wanderwrit not yet minted")
        require(self.visited[addr] == 1, "address has not previously held this NFT")
        emit("wanderwrit.prior.holder.confirmed")
    }

    // Asserts that `addr` is a NOVEL address (not in visited map).
    // Reverts if the address has already held this NFT.
    fn assert_novel_address(addr: address) {
        require(self.sealed == true, "wanderwrit not yet minted")
        require(self.visited[addr] == 0, "address has already held this NFT — not novel")
        emit("wanderwrit.novel.address.confirmed")
    }

    on_grace() {
        emit("wanderwrit.energy.low — transfer to keep alive")
    }

    on_refresh() {
        emit("wanderwrit.refreshed")
    }

    on_evaporate() {
        emit("wanderwrit.evaporated — the wanderwrit has died")
    }
}
