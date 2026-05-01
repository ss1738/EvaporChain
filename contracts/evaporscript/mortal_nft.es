// MortalNft — third pilot EvaporScript contract for the nft-marketplace
// dApp. Closes the EvaporScript-first migration of the dapp surface.
//
// Each NFT is its own contract instance. The contract's energy IS the
// NFT's lifespan — when it evaporates, the NFT is gone (becomes a Ghost
// at the chain level, recoverable via the standard ghost-recovery flow).
//
// Two lifetimes worth distinguishing:
//   - The EvaporScript builtin `owner` is the *deployer* (the minter).
//     Always immutable after deploy.
//   - The on-chain `self.owner` state field is the *current holder*.
//     Mutable via `transfer`. Initially set by `set_metadata` to the
//     recipient, which may differ from the minter (mint-to-someone-else
//     flow is the normal case for marketplace listings).
//
// Auth model:
//   - `set_metadata`: caller == builtin `owner` (only the minter).
//   - `transfer`: caller == `self.owner` (only the current holder).

contract MortalNft {
    state {
        // NFT identity — sealed once at mint, immutable thereafter.
        name: string = ""
        collection: string = ""
        // Metadata reference — could be an IPFS hash, an HTTP URL,
        // or a content-addressed blob hash. The contract is opaque to
        // the format; the dApp interprets it.
        metadata: string = ""
        sealed: bool = false

        // Current holder. Mutable via `transfer`. Initially set by
        // set_metadata; subsequent transfer() calls update it.
        owner: address
        transfer_count: u64 = 0
        last_transfer_epoch: u64 = 0
    }

    // Mint exactly once. Deployer-gated. The `recipient` argument is the
    // initial owner — typically the minter themselves, but the
    // marketplace can mint-to-buyer in one shot by passing the buyer's
    // address here, skipping the mint→transfer round-trip.
    fn set_metadata(
        nft_name: string,
        nft_collection: string,
        nft_metadata: string,
        recipient: address
    ) {
        require(caller == owner, "only minter can seal")
        require(self.sealed == false, "nft already minted")
        self.name = nft_name
        self.collection = nft_collection
        self.metadata = nft_metadata
        self.owner = recipient
        self.sealed = true
        emit("nft minted")
    }

    // Transfer ownership to `to`. Only the current holder can call.
    // Increments `transfer_count` for chain-of-custody telemetry.
    fn transfer(to: address) {
        require(self.sealed == true, "nft not yet minted")
        require(caller == self.owner, "only current owner can transfer")
        self.owner = to
        self.transfer_count += 1
        self.last_transfer_epoch = epoch
        emit("nft transferred")
    }

    fn current_owner() -> address {
        return self.owner
    }

    fn metadata_uri() -> string {
        return self.metadata
    }

    fn transfers() -> u64 {
        return self.transfer_count
    }

    on_grace() {
        emit("nft energy low — refresh to keep alive")
    }

    on_refresh() {
        emit("nft refreshed")
    }

    on_evaporate() {
        emit("nft evaporated")
    }
}
