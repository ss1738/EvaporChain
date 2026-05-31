// Single source of truth: `contracts/evaporscript/mnemochain.es`.

export const MNEMOCHAIN_SOURCE = `contract MnemoChain {
    state {
        holder: address
        card_content_hash: string = ""

        stability: u64 = 10
        last_review_epoch: u64 = 0
        has_reviewed: bool = false

        review_count: u64 = 0
        total_again_count: u64 = 0
        total_good_count: u64 = 0
        total_hard_count: u64 = 0

        sealed: bool = false
    }

    fn arm(card_holder: address, content: string, initial_stability: u64) {
        require(caller == owner, "only deployer arms")
        require(self.sealed == false, "already armed")
        require(initial_stability > 0, "initial_stability must be positive")
        self.holder = card_holder
        self.card_content_hash = content
        self.stability = initial_stability
        self.sealed = true
        emit("card armed")
    }

    fn review(rating: u64) {
        require(self.sealed == true, "not armed")
        require(caller == self.holder, "only holder reviews")
        require(rating >= 1, "rating must be 1-4")
        require(rating <= 4, "rating must be 1-4")
        if rating == 1 {
            self.stability = self.stability / 2
            if self.stability < 1 {
                self.stability = 1
            }
            self.total_again_count += 1
        } else if rating == 2 {
            self.total_hard_count += 1
        } else if rating == 3 {
            self.stability = self.stability * 2
            self.total_good_count += 1
        } else {
            self.stability = self.stability * 3
            self.total_good_count += 1
        }
        self.last_review_epoch = epoch
        self.has_reviewed = true
        self.review_count += 1
        emit("card reviewed")
    }

    fn retrievability_bp() -> u64 {
        if self.has_reviewed == false {
            return 10000
        }
        if epoch >= self.last_review_epoch + self.stability {
            return 0
        }
        return 10000 * (self.last_review_epoch + self.stability - epoch) / self.stability
    }

    fn is_due() -> bool {
        if self.has_reviewed == false {
            return false
        }
        if epoch >= self.last_review_epoch + self.stability {
            return true
        }
        if 10 * epoch >= 10 * self.last_review_epoch + self.stability {
            return true
        }
        return false
    }

    fn transfer(to: address) {
        require(self.sealed == true, "not armed")
        require(caller == self.holder, "only current holder transfers")
        self.holder = to
        emit("card transferred")
    }

    fn is_holder(who: address) -> bool {
        if self.sealed == false {
            return false
        }
        return who == self.holder
    }

    fn card_content_view() -> string {
        require(self.sealed == true, "not armed")
        return self.card_content_hash
    }

    fn stability_view() -> u64 {
        return self.stability
    }

    fn last_review_view() -> u64 {
        return self.last_review_epoch
    }

    fn has_been_reviewed() -> bool {
        return self.has_reviewed
    }

    fn review_count_view() -> u64 {
        return self.review_count
    }

    fn again_count() -> u64 {
        return self.total_again_count
    }

    fn good_count() -> u64 {
        return self.total_good_count
    }

    fn hard_count() -> u64 {
        return self.total_hard_count
    }

    fn is_armed() -> bool {
        return self.sealed
    }

    fn epochs_until_due() -> u64 {
        if self.has_reviewed == false {
            return 0
        }
        if 10 * epoch >= 10 * self.last_review_epoch + self.stability {
            return 0
        }
        return self.last_review_epoch + self.stability / 10 - epoch
    }

    on_grace() {
        emit("card energy low — refresh to keep practising")
    }

    on_refresh() {
        emit("card refreshed")
    }

    on_evaporate() {
        emit("card evaporated — memory archived")
    }
}
`;
