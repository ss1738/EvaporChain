// PaymentSplit — fourth pilot EvaporScript contract. The first stdlib
// contract that turns a payment-flow primitive into a decay-native one.
//
// Decay-thesis hook: classic payment splitters (OpenZeppelin's PaymentSplitter,
// 0xSplits) hold funds *forever* — if a recipient never claims, the chain
// pays the storage cost in perpetuity. PaymentSplit gives every share a
// lifespan equal to the contract's own energy: claim while energy lasts,
// or your share lapses. The on_evaporate hook records that all unclaimed
// shares are forfeit, releasing the chain from carrying them.
//
// Setup is two-phase to stay inside EvaporScript's structural-totality
// contract (no array args in the pilot grammar):
//
//   1. After deploy, the deployer calls `add_recipient(addr, share_bps)`
//      for each recipient. share_bps is in basis points (0..=10000).
//   2. `seal()` locks the recipient set once `total_bps == 10000`.
//
// After seal, anyone can `deposit(amount)` (cumulatively tracked in
// `total_deposited`). Each recipient calls `claim()` to record their pull
// of `(total_deposited * share_bps[caller]) / 10000 - claimed[caller]`.
// Actual fund movement is handled by the chain runtime / a coordinator —
// this contract is the on-chain ledger of *who is owed what and who has
// taken what*.
//
// Auth model:
//   - `add_recipient` / `seal`:    caller == builtin `owner` (deployer).
//   - `deposit`:                   open (anyone can fund the split).
//   - `claim`:                     caller's own row only, gated on
//                                  `share_bps[caller] > 0`.
//   - `forfeit`:                   on_evaporate path — emits the doctrine
//                                  signal that unclaimed shares lapse.
//
// Bounded-state proof:
//   - `share_bps`, `claimed` keyed by address — bounded by `recipient_count`
//     which itself is bounded by gas (each `add_recipient` burns gas, so
//     attacker can't grow the map without paying for it).
//   - No loops over recipient set. Every method is O(1) in map ops.

contract PaymentSplit {
    state {
        // Recipient ledger. share_bps[addr] in basis points (0..=10000),
        // claimed[addr] is the cumulative-pulled amount per recipient.
        share_bps: map[address -> u64]
        claimed: map[address -> u64]

        // Setup invariants. total_bps must equal 10000 before seal.
        total_bps: u64 = 0
        recipient_count: u64 = 0
        sealed: bool = false

        // Cumulative inflow. Monotonic — never decreases (claims pull
        // against this, but the running total stays put as the basis
        // for share computation).
        total_deposited: u64 = 0

        // Forfeit accounting. Bumped only when on_evaporate fires.
        // unclaimed_at_evaporate captures what was on the table at death.
        forfeit_signaled: bool = false
        unclaimed_at_evaporate: u64 = 0
    }

    // Phase 1: register a recipient with their share. Repeatable until
    // seal. share_bps is basis points (1..=10000); cumulative `total_bps`
    // must not exceed 10000. The deployer can re-call with the same
    // address only once — overwrites are blocked to keep the audit trail
    // unambiguous.
    fn add_recipient(recipient: address, share: u64) {
        require(caller == owner, "only deployer can add recipients")
        require(self.sealed == false, "split already sealed")
        require(share > 0, "share must be positive")
        require(share <= 10000, "share exceeds 10000 bps")
        require(self.share_bps[recipient] == 0, "recipient already added")
        require(self.total_bps + share <= 10000, "total bps would exceed 10000")
        self.share_bps[recipient] = share
        self.total_bps += share
        self.recipient_count += 1
        emit("recipient added")
    }

    // Phase 2: seal the recipient set. Requires `total_bps == 10000`
    // exactly — under-allocation would leave dust unaccounted for and
    // make the unclaimed-at-evaporate calculation ambiguous.
    fn seal() {
        require(caller == owner, "only deployer can seal")
        require(self.sealed == false, "already sealed")
        require(self.total_bps == 10000, "total bps must equal 10000 to seal")
        self.sealed = true
        emit("split sealed")
    }

    // Open inflow: anyone can deposit into the split. The contract
    // doesn't custody the funds itself in the pilot grammar — the chain
    // runtime / a coordinator handles actual transfers; this method
    // updates the ledger that drives claim math.
    fn deposit(amount: u64) {
        require(self.sealed == true, "split not yet sealed")
        require(amount > 0, "deposit must be positive")
        self.total_deposited += amount
        emit("deposit recorded")
    }

    // Each recipient claims their share. The owed amount is computed
    // from the cumulative basis: `(total_deposited * share_bps) / 10000`,
    // minus what the caller has already claimed. This is the standard
    // pull-payment pattern adapted to decay semantics: the caller can
    // only pull while the contract is alive; once it evaporates, any
    // unclaimed delta is forfeit.
    fn claim() -> u64 {
        require(self.sealed == true, "split not yet sealed")
        let bps = self.share_bps[caller]
        require(bps > 0, "caller is not a recipient")
        let owed_total = (self.total_deposited * bps) / 10000
        let already_claimed = self.claimed[caller]
        require(owed_total > already_claimed, "nothing to claim")
        let delta = owed_total - already_claimed
        self.claimed[caller] = owed_total
        emit("claim recorded")
        return delta
    }

    // View: cumulative entitlement for an address (what they'd be owed
    // if they claimed right now, gross of prior claims).
    fn entitlement_of(addr: address) -> u64 {
        let bps = self.share_bps[addr]
        if bps == 0 {
            return 0
        }
        return (self.total_deposited * bps) / 10000
    }

    // View: outstanding (claimable) balance for an address.
    fn pending_of(addr: address) -> u64 {
        let bps = self.share_bps[addr]
        if bps == 0 {
            return 0
        }
        let owed = (self.total_deposited * bps) / 10000
        let pulled = self.claimed[addr]
        if owed <= pulled {
            return 0
        }
        return owed - pulled
    }

    fn share_of(addr: address) -> u64 {
        return self.share_bps[addr]
    }

    fn total_pool() -> u64 {
        return self.total_deposited
    }

    fn recipients() -> u64 {
        return self.recipient_count
    }

    on_grace() {
        emit("split energy low — claim before evaporation or your share lapses")
    }

    on_refresh() {
        emit("split refreshed")
    }

    // Doctrine moment: when the contract evaporates, any pending balance
    // across all recipients is forfeit. We can't iterate the map here
    // (no array-of-keys in the pilot grammar), but we can stamp the
    // forfeit signal so an off-chain auditor can reconstruct the table
    // from event logs + the final ledger state.
    on_evaporate() {
        self.forfeit_signaled = true
        self.unclaimed_at_evaporate = self.total_deposited
        emit("split evaporated — all unclaimed shares forfeit")
    }
}
