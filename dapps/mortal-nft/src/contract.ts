// Single source of truth: `contracts/evaporscript/mortal_nft.es`.
// Cargo pilot lives at `crates/evaporchain-script/tests/mortal_nft_pilot.rs`.

export const MORTAL_NFT_SOURCE = `contract MortalNft {
    state {
        name: string = ""
        collection: string = ""
        metadata: string = ""
        sealed: bool = false

        holder: address
        transfer_count: u64 = 0
        last_transfer_epoch: u64 = 0
    }

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
        self.holder = recipient
        self.sealed = true
        emit("nft minted")
    }

    fn transfer(to: address) {
        require(self.sealed == true, "nft not yet minted")
        require(caller == self.holder, "only current owner can transfer")
        self.holder = to
        self.transfer_count += 1
        self.last_transfer_epoch = epoch
        emit("nft transferred")
    }

    fn current_owner() -> address {
        return self.holder
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
}`;
