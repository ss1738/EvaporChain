// Single source of truth: `contracts/evaporscript/mortal_nft.es`.
// Inline copy the dapp ships. Keep byte-identical to the .es file. The
// Rust integration test `crates/evaporchain-script/tests/mortal_nft_
// pilot.rs` is the regression barrier (7/7 tests).
//
// Lifecycle:
//   1. POST /api/tx/deploy-script  source=MORTAL_NFT_SOURCE → tx_hash
//   2. POST /api/tx/call-script    method="set_metadata"
//      args=[name, collection, metadata, recipient_hex]
//      → mints with `recipient` as the initial holder.
//   3. POST /api/tx/call-script    method="transfer"
//      args=[new_holder_hex] from current holder.
//   4. GET  /api/script/:id        → live state.

export const MORTAL_NFT_SOURCE = `// MortalNft — third pilot EvaporScript contract for the nft-marketplace dApp.

contract MortalNft {
    state {
        name: string = ""
        collection: string = ""
        metadata: string = ""
        sealed: bool = false

        owner: address
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
        self.owner = recipient
        self.sealed = true
        emit("nft minted")
    }

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
`;
