// Single source of truth: `contracts/evaporscript/mayfly.es`.
// Byte-stable inline copy. Pilot at `mod mayfly_pilot` is the
// regression barrier proving this exact source parses, compiles,
// and is totality-clean.

export const MAYFLY_SOURCE = `contract Mayfly {
    state {
        holder: address
        metadata: string = ""
        born_epoch: u64 = 0
        sealed: bool = false
        transfer_count: u64 = 0
    }

    fn hatch(meta: string) {
        require(caller == owner, "only minter hatches")
        require(self.sealed == false, "already hatched")
        self.holder = owner
        self.metadata = meta
        self.born_epoch = epoch
        self.sealed = true
        emit("mayfly hatched")
    }

    fn transfer(to: address) {
        require(self.sealed == true, "not yet hatched")
        require(caller == self.holder, "only current holder transfers")
        self.holder = to
        self.transfer_count += 1
        emit("mayfly transferred")
    }

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
`;
