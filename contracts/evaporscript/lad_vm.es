// Linear-Affine-Decay VM (LAD-VM) — resource wallet contract.
//
// Doctrine: INVENTION_STACK.md §4.1 row 12 — "Move resources × decay.
// 'Use it or evaporate.' Forces liveness as a type-system property."
//
// Three resource classes, each enforcing a different substructural mode:
//
//   Linear   — must be consumed exactly once; drop is rejected.
//              Models one-time-use tokens (event tickets, one-show passes).
//
//   Affine   — may be consumed at most once; explicit drop is also allowed.
//              Models optional-use coupons (discounts, optional intros).
//
//   Decaying — affine, plus a hard expiry epoch.  The resource becomes
//              void at epoch >= expires_at without any transaction.
//              Models time-bounded access passes (day passes, trial licences).
//
// Doctrine moment (LAD-VM):
//   When the contract's energy falls to zero the entire resource wallet
//   evaporates — all Linear, Affine, and Decaying resources held within
//   are void simultaneously.  "Use it or evaporate" is enforced by the
//   VM as a physical law: resources cannot outlast the energy budget.
//
// Usage flow:
//   owner calls issue_*(holder, code, ...) to mint a resource.
//   holder calls redeem_*(code) to consume it.
//   holder calls drop_affine(code) / drop_decaying(code) to abandon.
//   Any caller can call is_valid(code) to check decaying resources.
//
// State layout uses parallel u64 presence maps to handle the U64(0)
// default gotcha: map fields default to U64(0) regardless of declared
// type. Presence maps are non-zero for valid entries.

contract LadVm {

    state {
        // ── Type tags ─────────────────────────────────────────────────────
        // 0 = no resource; 1 = Linear; 2 = Affine; 3 = Decaying
        resource_type: map[u64 -> u64]

        // ── Holder (address as address) ───────────────────────────────────
        holder_present: map[u64 -> u64]
        holder_addr:    map[u64 -> address]

        // ── Human-readable code/claim (string tag) ────────────────────────
        code_present: map[u64 -> u64]
        code_str:     map[u64 -> string]

        // ── Consumed / dropped flag ────────────────────────────────────────
        // 0 = live; 1 = consumed; 2 = dropped
        status: map[u64 -> u64]

        // ── Decaying: expiry epoch ─────────────────────────────────────────
        expires_at: map[u64 -> u64]

        // ── Monotone counter ──────────────────────────────────────────────
        next_id: u64 = 0
        linear_count:  u64 = 0
        affine_count:  u64 = 0
        decaying_count: u64 = 0
    }

    // ── Issue helpers (owner-only) ────────────────────────────────────────

    // Issue a Linear resource.  The holder MUST redeem it; drop is illegal.
    fn issue_linear(holder: address, code: string) -> u64 {
        require(caller == owner, "only owner can issue")
        let id = self.next_id
        self.resource_type[id] = 1
        self.holder_present[id] = 1
        self.holder_addr[id] = holder
        self.code_present[id] = 1
        self.code_str[id] = code
        self.status[id] = 0
        self.next_id += 1
        self.linear_count += 1
        emit("linear issued")
        return id
    }

    // Issue an Affine resource.  Holder may redeem or drop.
    fn issue_affine(holder: address, code: string) -> u64 {
        require(caller == owner, "only owner can issue")
        let id = self.next_id
        self.resource_type[id] = 2
        self.holder_present[id] = 1
        self.holder_addr[id] = holder
        self.code_present[id] = 1
        self.code_str[id] = code
        self.status[id] = 0
        self.next_id += 1
        self.affine_count += 1
        emit("affine issued")
        return id
    }

    // Issue a Decaying resource.  Expires at `expires_epoch`.
    fn issue_decaying(holder: address, code: string, expires_epoch: u64) -> u64 {
        require(caller == owner, "only owner can issue")
        require(expires_epoch > epoch, "expiry must be in the future")
        let id = self.next_id
        self.resource_type[id] = 3
        self.holder_present[id] = 1
        self.holder_addr[id] = holder
        self.code_present[id] = 1
        self.code_str[id] = code
        self.status[id] = 0
        self.expires_at[id] = expires_epoch
        self.next_id += 1
        self.decaying_count += 1
        emit("decaying issued")
        return id
    }

    // ── Consume ───────────────────────────────────────────────────────────

    // Redeem a Linear resource.  Caller must be the holder; marks consumed.
    // A second call is REJECTED — Linear enforces exactly-once semantics.
    fn redeem_linear(id: u64) {
        require(self.resource_type[id] == 1, "not a linear resource")
        require(self.holder_present[id] > 0, "resource does not exist")
        require(self.status[id] == 0, "already consumed or dropped")
        // Caller must be the holder.  Use address comparison.
        require(caller == self.holder_addr[id], "caller is not the holder")
        self.status[id] = 1
        emit("linear redeemed")
    }

    // Redeem an Affine resource.  Caller must be the holder; marks consumed.
    fn redeem_affine(id: u64) {
        require(self.resource_type[id] == 2, "not an affine resource")
        require(self.holder_present[id] > 0, "resource does not exist")
        require(self.status[id] == 0, "already consumed or dropped")
        require(caller == self.holder_addr[id], "caller is not the holder")
        self.status[id] = 1
        emit("affine redeemed")
    }

    // Redeem a Decaying resource.  Also checks the expiry gate.
    fn redeem_decaying(id: u64) {
        require(self.resource_type[id] == 3, "not a decaying resource")
        require(self.holder_present[id] > 0, "resource does not exist")
        require(self.status[id] == 0, "already consumed or dropped")
        require(epoch < self.expires_at[id], "resource has expired")
        require(caller == self.holder_addr[id], "caller is not the holder")
        self.status[id] = 1
        emit("decaying redeemed")
    }

    // ── Drop ──────────────────────────────────────────────────────────────

    // Explicitly drop an Affine resource.  Allowed; marks as dropped.
    fn drop_affine(id: u64) {
        require(self.resource_type[id] == 2, "not an affine resource")
        require(self.holder_present[id] > 0, "resource does not exist")
        require(self.status[id] == 0, "already consumed or dropped")
        require(caller == self.holder_addr[id], "caller is not the holder")
        self.status[id] = 2
        emit("affine dropped")
    }

    // Explicitly drop a Decaying resource.  Allowed even before expiry.
    fn drop_decaying(id: u64) {
        require(self.resource_type[id] == 3, "not a decaying resource")
        require(self.holder_present[id] > 0, "resource does not exist")
        require(self.status[id] == 0, "already consumed or dropped")
        require(caller == self.holder_addr[id], "caller is not the holder")
        self.status[id] = 2
        emit("decaying dropped")
    }

    // Trying to drop a Linear resource is REJECTED (substructural invariant).
    fn drop_linear(id: u64) {
        require(self.resource_type[id] == 1, "not a linear resource")
        // Linear resources cannot be dropped; they must be consumed exactly once.
        require(false, "Linear resource cannot be dropped — must be redeemed exactly once")
    }

    // ── Queries ───────────────────────────────────────────────────────────

    // Returns 1 if a decaying resource is still live (not expired, not consumed/dropped).
    fn is_valid(id: u64) -> u64 {
        if self.resource_type[id] == 0 {
            return 0
        }
        if self.status[id] > 0 {
            return 0
        }
        // For Decaying resources, also check expiry.
        if self.resource_type[id] == 3 {
            if epoch >= self.expires_at[id] {
                return 0
            }
        }
        return 1
    }

    fn resource_status(id: u64) -> u64 {
        return self.status[id]
    }

    fn resource_type_of(id: u64) -> u64 {
        return self.resource_type[id]
    }

    fn resource_code(id: u64) -> string {
        require(self.code_present[id] > 0, "no resource at id")
        return self.code_str[id]
    }

    fn counts() -> u64 {
        // Returns total = linear + affine + decaying (for verification).
        return self.linear_count + self.affine_count + self.decaying_count
    }
}
