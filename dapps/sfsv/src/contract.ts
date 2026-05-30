// Single source of truth: `contracts/evaporscript/sfsv.es`.

export const SFSV_SOURCE = `contract SFSV {
    state {
        future_self: address
        deposit_amount: u64 = 0
        release_epoch: u64 = 0
        sealed: bool = false

        beneficiary: address
        sold: bool = false

        withdrawn: bool = false
    }

    fn arm(self_addr: address, amount: u64, release_at: u64) {
        require(caller == owner, "only depositor arms")
        require(self.sealed == false, "already armed")
        require(amount > 0, "deposit must be positive")
        require(release_at > epoch, "release_epoch must be in the future")
        self.future_self = self_addr
        self.beneficiary = self_addr
        self.deposit_amount = amount
        self.release_epoch = release_at
        self.sealed = true
        emit("vault armed")
    }

    fn sell(buyer: address) {
        require(self.sealed == true, "not armed")
        require(self.withdrawn == false, "already withdrawn")
        require(self.sold == false, "claim already sold once")
        require(caller == self.beneficiary, "only current beneficiary sells")
        self.beneficiary = buyer
        self.sold = true
        emit("claim sold")
    }

    fn withdraw() {
        require(self.sealed == true, "not armed")
        require(self.withdrawn == false, "already withdrawn")
        require(caller == self.beneficiary, "only beneficiary withdraws")
        require(epoch >= self.release_epoch, "still locked")
        self.withdrawn = true
        emit("vault withdrawn")
    }

    fn is_releasable() -> bool {
        if self.sealed == false {
            return false
        }
        if self.withdrawn == true {
            return false
        }
        if epoch < self.release_epoch {
            return false
        }
        return true
    }

    fn epochs_until_release() -> u64 {
        if self.sealed == false {
            return 0
        }
        if epoch >= self.release_epoch {
            return 0
        }
        return self.release_epoch - epoch
    }

    fn is_beneficiary(who: address) -> bool {
        if self.sealed == false {
            return false
        }
        return who == self.beneficiary
    }

    fn is_original_future_self(who: address) -> bool {
        if self.sealed == false {
            return false
        }
        return who == self.future_self
    }

    fn deposit_amount_view() -> u64 {
        return self.deposit_amount
    }

    fn release_at() -> u64 {
        return self.release_epoch
    }

    fn is_armed() -> bool {
        return self.sealed
    }

    fn is_sold() -> bool {
        return self.sold
    }

    fn is_withdrawn() -> bool {
        return self.withdrawn
    }

    on_grace() {
        emit("vault fading — refresh to extend the lock OR the claim risks forfeit")
    }

    on_refresh() {
        emit("vault refreshed")
    }

    on_evaporate() {
        if self.withdrawn == false {
            emit("vault evaporated unwithdrawn — deposit forfeit")
        } else {
            emit("vault evaporated post-withdrawal")
        }
    }
}
`;
