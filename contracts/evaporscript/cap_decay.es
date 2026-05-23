// CapabilityDecayVM — §4.2 Tier-2 VM Paradigm: ocap + energy-decay.
//
// Doctrine: INVENTION_STACK.md §4.2. Cultural precedent: KeyKOS (Bomberger
// 1992), seL4 (Klein 2009), W7 (Hardy 1988 Norm Hardy).
//
// Why ocap, why decay — structural argument:
//
//   Default-deny is the inverse of EVM default-allow + ACL. In an ocap
//   system, a subject can only do something if it *holds the capability*
//   to do it. The capability IS the authorization — no tx.origin, no role
//   tables, no global authorization checks.
//
//   Energy-decay closes the *capability lifetime* hole every ocap system
//   has historically suffered: granted capabilities persist until
//   explicitly revoked. Revocation is traditionally O(holders) or
//   requires brittle revocation caveats. EvaporChain's single-λ makes
//   capability lifetime a *physical* property — every capability has an
//   energy budget that decays; below threshold, the capability is
//   *unforgeably non-invocable*. No permission check needed; the
//   capability simply ceases to exist as an authority.
//
// Three operations:
//   mint(verb, object, max_inv, init_energy) — create root capability.
//     (param named init_energy; `energy` is a reserved EvaporScript built-in)
//   attenuate(parent, child_max_inv, child_energy) — child is STRICT
//     subset of parent's authority. Child energy ≤ parent energy.
//     Attenuation is one-way: you cannot attenuate upward.
//   revoke(slot) — zero the energy. Structural: descendants fail the
//     parent-chain walk at invoke_gate without enumerating holders.
//
// Invoke gate: walks the parent chain up to 8 levels. Passes iff the
// cap itself AND every ancestor have energy > 0. Revocation of any
// ancestor kills all descendants — structural, O(depth) not O(holders).
//
// Verb codes (u64): 1=read, 2=write, 3=transfer.
// Object id: arbitrary u64 handle (e.g., resource slot).
// Parent sentinel: cap_parent[slot] = 999 means "root, no parent."
//
// Witness telemetry: witness_cap(slot) stores (energy, parent_energy)
// into snapshot1 (first call) or snapshot2 (second call).
//
// Proof gates:
//   invoke_gate(slot)         — reverts if any ancestor dead.
//   require_ancestor_dead(slot) — reverts if no ancestor dead (adversarial).
//
// Press claim: "capability-decay VM — ocap with structural energy-bound
// authority. Mint → invoke. Attenuate strict subset. Revoke root →
// descendants fail structurally. No holder enumeration."
//
// INVENTION_STACK.md §4.2: CapabilityDecayVM.

contract CapabilityDecayVM {
    state {
        // Per-capability storage. Slot = auto-assigned sequential u64.
        cap_verb: map[u64 -> u64]       // slot → verb code (1=read, 2=write, 3=transfer)
        cap_object: map[u64 -> u64]     // slot → object id (arbitrary u64 handle)
        cap_max_inv: map[u64 -> u64]    // slot → max invocations per epoch bound
        cap_energy: map[u64 -> u64]     // slot → current energy (0 = non-invocable)
        cap_parent: map[u64 -> u64]     // slot → parent slot (999 = root sentinel)
        cap_present: map[u64 -> u64]    // slot → 0=absent, 1=registered
        cap_count: u64 = 0

        // Witness snapshots (energy + parent_energy at call time).
        witness_count: u64 = 0
        snapshot1_slot: u64 = 0
        snapshot1_energy: u64 = 0
        snapshot1_par_energy: u64 = 0
        snapshot2_slot: u64 = 0
        snapshot2_energy: u64 = 0
        snapshot2_par_energy: u64 = 0
    }

    // Mint a root capability (no parent). Owner-only.
    // verb: 1=read, 2=write, 3=transfer.
    // object: u64 handle of the resource being authorized.
    // max_inv: upper bound on invocations per epoch.
    // init_energy: initial energy; decay brings it toward 0.
    //   (named init_energy not energy — `energy` is a reserved EvaporScript built-in)
    fn mint(verb: u64, object: u64, max_inv: u64, init_energy: u64) {
        require(caller == owner, "only owner can mint root capabilities")
        require(verb >= 1, "verb must be 1 (read), 2 (write), or 3 (transfer)")
        require(verb <= 3, "verb must be 1 (read), 2 (write), or 3 (transfer)")
        require(max_inv > 0, "max invocations must be positive")
        require(init_energy > 0, "capability energy must be positive")
        let slot = self.cap_count
        self.cap_verb[slot] = verb
        self.cap_object[slot] = object
        self.cap_max_inv[slot] = max_inv
        self.cap_energy[slot] = init_energy
        self.cap_parent[slot] = 999     // root sentinel
        self.cap_present[slot] = 1
        self.cap_count += 1
        emit("capdecay.minted")
    }

    // Attenuate: create a child capability from a live parent.
    // Authority constraint (strict subset): child_max_inv < parent.max_inv.
    // Energy constraint: child_energy ≤ parent.energy.
    // Inherits verb + object from parent (cannot elevate scope).
    // Owner-only (holder check simplified for doctrine proof).
    fn attenuate(parent_slot: u64, child_max_inv: u64, child_energy: u64) {
        require(caller == owner, "only owner can attenuate capabilities")
        require(parent_slot < self.cap_count, "parent slot not found")
        require(self.cap_present[parent_slot] == 1, "parent not registered")
        require(self.cap_energy[parent_slot] > 0, "parent capability dead — cannot attenuate dead cap")
        require(child_max_inv < self.cap_max_inv[parent_slot], "child max_inv must be STRICTLY LESS than parent — attenuation must be a strict subset")
        require(child_max_inv > 0, "child max_inv must be positive")
        require(child_energy <= self.cap_energy[parent_slot], "child energy cannot exceed parent energy")
        require(child_energy > 0, "child energy must be positive")
        let slot = self.cap_count
        self.cap_verb[slot] = self.cap_verb[parent_slot]
        self.cap_object[slot] = self.cap_object[parent_slot]
        self.cap_max_inv[slot] = child_max_inv
        self.cap_energy[slot] = child_energy
        self.cap_parent[slot] = parent_slot
        self.cap_present[slot] = 1
        self.cap_count += 1
        emit("capdecay.attenuated")
    }

    // Revoke: zero the energy on a capability. Owner-only.
    // Structural revocation: any attenuated descendants will fail
    // invoke_gate because the parent-chain walk finds energy=0.
    // No holder enumeration required — the energy is the authority.
    fn revoke(slot: u64) {
        require(caller == owner, "only owner can revoke")
        require(slot < self.cap_count, "cap slot not found")
        require(self.cap_present[slot] == 1, "cap not registered")
        self.cap_energy[slot] = 0
        emit("capdecay.revoked")
    }

    // Invoke gate: proves cap AND all ancestors have energy > 0.
    // Walks parent chain up to depth 8. Reverts if any ancestor dead.
    // This is the core ocap structural claim: revoking a parent kills
    // every descendant without enumerating holders.
    fn invoke_gate(slot: u64) {
        require(slot < self.cap_count, "cap slot not found")
        require(self.cap_present[slot] == 1, "cap not registered")
        let current = slot
        let ok_flag = 0
        let blocked = 0
        let i = 0
        while i < 8 {
            if blocked == 0 {
                if ok_flag == 0 {
                    let e = self.cap_energy[current]
                    if e == 0 {
                        blocked = 1
                    }
                    if e > 0 {
                        let par = self.cap_parent[current]
                        if par == 999 {
                            ok_flag = 1
                        }
                        if par != 999 {
                            current = par
                        }
                    }
                }
            }
            i = i + 1
        }
        require(ok_flag == 1, "invoke_gate FAILED: capability or ancestor has energy=0 (revoked or decayed)")
        emit("capdecay.invoke_gate.passed")
    }

    // Witness: snapshot energy and parent's energy for a slot.
    // Root caps: par_energy stored as 0 (sentinel has no energy slot).
    // First call → snapshot1, second → snapshot2.
    fn witness_cap(slot: u64) {
        require(slot < self.cap_count, "cap slot not found")
        require(self.cap_present[slot] == 1, "cap not registered")
        let e = self.cap_energy[slot]
        let par = self.cap_parent[slot]
        let par_e = 0
        if par != 999 {
            par_e = self.cap_energy[par]
        }
        if self.witness_count == 0 {
            self.snapshot1_slot = slot
            self.snapshot1_energy = e
            self.snapshot1_par_energy = par_e
        }
        if self.witness_count == 1 {
            self.snapshot2_slot = slot
            self.snapshot2_energy = e
            self.snapshot2_par_energy = par_e
        }
        self.witness_count += 1
        emit("capdecay.witnessed")
    }

    // Proof gate: some ancestor in the chain has energy=0.
    // Proves structural revocation propagated without holder enumeration.
    // Walks parent chain up to depth 8; requires found_dead=1.
    fn require_ancestor_dead(slot: u64) {
        require(slot < self.cap_count, "cap slot not found")
        require(self.cap_present[slot] == 1, "cap not registered")
        let current = slot
        let found_dead = 0
        let i = 0
        while i < 8 {
            if found_dead == 0 {
                let e = self.cap_energy[current]
                if e == 0 {
                    found_dead = 1
                }
                if e > 0 {
                    let par = self.cap_parent[current]
                    if par == 999 {
                        i = 7
                    }
                    if par != 999 {
                        current = par
                    }
                }
            }
            i = i + 1
        }
        require(found_dead == 1, "no ancestor dead — capability chain is still fully live")
        emit("capdecay.ancestor_dead.confirmed")
    }

    on_grace() {
        emit("capdecay.energy.low — capability registry evaporating")
    }

    on_refresh() {
        emit("capdecay.refreshed")
    }

    on_evaporate() {
        emit("capdecay.evaporated — capability registry gone")
    }
}
