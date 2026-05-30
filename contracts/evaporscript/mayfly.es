// Mayfly — the doctrine-purest NFT. The reference contract behind
// the catalogue's MAYFLY (0x0001_0005), the 5th and final entry in
// the NFT lane.
//
// Mayflies live one day. This NFT does the same: deployed with a
// very short half-life, it fades to nothing in epochs, not years.
// The contract's OWN energy IS the NFT's lifespan — there is no
// "expire" method, no terminal state to set, no eviction code path.
// Decay is automatic at the chain layer; the contract just publishes
// who holds the mayfly while it's alive, exposes its metadata, and
// enforces transfer.
//
// Unusual for an NFT: refresh works. Calling /api/v1/contract/refresh
// re-energises the contract, which the doctrine calls "defying the
// mayfly's nature." The contract logs this as an event so the holder
// can see they're paying to keep something temporary alive past its
// design.
//
// Deploy flow:
//   1. POST /api/tx/deploy-script  source_code = MAYFLY_SOURCE,
//      energy = 1000, half_life = 10  (defaults — finishes in ~100 epochs)
//   2. POST /api/tx/call-script  method = "hatch", args = [metadata]
//   Then `transfer(to)` while alive; `read_metadata()` / `age_epochs()`
//   any time.

contract Mayfly {
    state {
        holder: address
        metadata: string = ""
        born_epoch: u64 = 0
        sealed: bool = false
        transfer_count: u64 = 0
    }

    // One-shot: the minter (owner) seals metadata and self-assigns
    // as the first holder. After this, the metadata is immutable for
    // the lifetime of the contract.
    fn hatch(meta: string) {
        require(caller == owner, "only minter hatches")
        require(self.sealed == false, "already hatched")
        self.holder = owner
        self.metadata = meta
        self.born_epoch = epoch
        self.sealed = true
        emit("mayfly hatched")
    }

    // Current holder transfers to a new holder. No restrictions on
    // who can receive — mayflies are bearer assets while alive.
    fn transfer(to: address) {
        require(self.sealed == true, "not yet hatched")
        require(caller == self.holder, "only current holder transfers")
        self.holder = to
        self.transfer_count += 1
        emit("mayfly transferred")
    }

    // Open read — anyone can inspect metadata while the contract is
    // alive. The doctrine's claim is that liveness IS the access
    // control: once evaporated, the contract is gone and so are
    // these views.
    fn read_metadata() -> string {
        require(self.sealed == true, "not yet hatched")
        return self.metadata
    }

    fn is_holder(who: address) -> bool {
        if self.sealed == false {
            return false
        }
        return who == self.holder
    }

    fn is_hatched() -> bool {
        return self.sealed
    }

    fn born() -> u64 {
        return self.born_epoch
    }

    // Age in epochs since hatching. Returns 0 if not yet hatched
    // (the bool sentinel handles the epoch-0-hatch case — same
    // lesson from witnessfit / mortal_dao, applied pre-emptively).
    fn age_epochs() -> u64 {
        if self.sealed == false {
            return 0
        }
        return epoch - self.born_epoch
    }

    fn transfers_total() -> u64 {
        return self.transfer_count
    }

    on_grace() {
        emit("mayfly fading")
    }

    on_refresh() {
        emit("mayfly refreshed — defying nature")
    }

    on_evaporate() {
        emit("mayfly gone — ephemeral as designed")
    }
}
