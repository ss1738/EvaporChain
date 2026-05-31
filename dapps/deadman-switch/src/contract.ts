// Single source of truth: `contracts/evaporscript/deadman_switch.es`.
// Byte-stable inline copy. Keep in sync — the test suite mirrors
// the field/method names + signatures from the .es file.

export const DEADMAN_SWITCH_SOURCE = `contract DeadManSwitch {
    state {
        holder: address
        secret_hash: string = ""
        refresh_window: u64 = 0

        last_refresh_epoch: u64 = 0
        has_refreshed: bool = false
        refresh_count: u64 = 0

        released: bool = false
        released_at_epoch: u64 = 0
        released_by: address
        revealed_secret: string = ""

        sealed: bool = false
    }

    fn arm(switch_holder: address, payload_hash: string, window_epochs: u64) {
        require(caller == owner, "only deployer arms")
        require(self.sealed == false, "already armed")
        require(window_epochs > 0, "window must be positive")
        self.holder = switch_holder
        self.secret_hash = payload_hash
        self.refresh_window = window_epochs
        self.last_refresh_epoch = epoch
        self.has_refreshed = true
        self.refresh_count = 1
        self.sealed = true
        emit("dead-man switch armed")
    }

    fn refresh() {
        require(self.sealed == true, "not armed")
        require(self.released == false, "already released")
        require(caller == self.holder, "only holder refreshes")
        self.last_refresh_epoch = epoch
        self.has_refreshed = true
        self.refresh_count += 1
        emit("switch refreshed")
    }

    fn trigger_early(plaintext: string) {
        require(self.sealed == true, "not armed")
        require(self.released == false, "already released")
        require(caller == self.holder, "only holder triggers early")
        self.released = true
        self.released_at_epoch = epoch
        self.released_by = self.holder
        self.revealed_secret = plaintext
        emit("switch triggered early by holder")
    }

    fn release_dead(plaintext: string) {
        require(self.sealed == true, "not armed")
        require(self.released == false, "already released")
        require(epoch >= self.last_refresh_epoch + self.refresh_window,
                "deadline not yet passed")
        self.released = true
        self.released_at_epoch = epoch
        self.released_by = caller
        self.revealed_secret = plaintext
        emit("switch released — deadline lapsed")
    }

    fn transfer_holder(new_holder: address) {
        require(self.sealed == true, "not armed")
        require(self.released == false, "released — holder role retired")
        require(caller == self.holder, "only current holder transfers")
        self.holder = new_holder
        emit("holder transferred")
    }

    fn is_armed() -> bool {
        return self.sealed
    }

    fn is_released() -> bool {
        return self.released
    }

    fn is_alive() -> bool {
        if self.sealed == false {
            return false
        }
        if self.released == true {
            return false
        }
        if epoch >= self.last_refresh_epoch + self.refresh_window {
            return false
        }
        return true
    }

    fn is_releasable() -> bool {
        if self.sealed == false {
            return false
        }
        if self.released == true {
            return false
        }
        if epoch < self.last_refresh_epoch + self.refresh_window {
            return false
        }
        return true
    }

    fn epochs_until_deadline() -> u64 {
        if self.sealed == false {
            return 0
        }
        if epoch >= self.last_refresh_epoch + self.refresh_window {
            return 0
        }
        return self.last_refresh_epoch + self.refresh_window - epoch
    }

    fn secret_hash_view() -> string {
        return self.secret_hash
    }

    fn revealed_secret_view() -> string {
        require(self.released == true, "not yet released")
        return self.revealed_secret
    }

    fn released_at_view() -> u64 {
        require(self.released == true, "not yet released")
        return self.released_at_epoch
    }

    fn refresh_count_view() -> u64 {
        return self.refresh_count
    }

    fn last_refresh_view() -> u64 {
        return self.last_refresh_epoch
    }

    fn holder_view() -> address {
        require(self.sealed == true, "not armed")
        return self.holder
    }

    fn is_holder(who: address) -> bool {
        if self.sealed == false {
            return false
        }
        return who == self.holder
    }

    on_grace() {
        emit("switch contract energy low — refresh or release before evaporation")
    }

    on_refresh() {
        emit("switch contract refreshed")
    }

    on_evaporate() {
        emit("switch evaporated — secret commitment archived to tomb")
    }
}`;
