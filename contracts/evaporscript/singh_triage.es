// SinghTriage (Vital-Sign Wallet Inbox) — §A5.4 NFT Primitive.
//
// Models the wallet-opens-on-inbox paradigm: each obligation in a
// holder's inbox decays independently by its own half-life. Ignored
// items evaporate; refreshed items survive. classify_all() re-sorts
// the inbox into urgency buckets so wallets know what needs attention
// today vs what still has runway.
//
// Urgency model (horizons measured in half-life hops from now):
//   decayed:    current_energy ≤ 1  (already at death threshold)
//   today:      hops ≤ horizon_today    (default 2)
//   tomorrow:   hops ≤ horizon_tomorrow (default 5)
//   this_week:  hops ≤ horizon_week     (default 14)
//   healthy:    hops > horizon_week
//
// Energy decay formula (hyperbolic approximation, same as SinghResonance):
//   current_energy = anchor_energy * hl / (elapsed + hl)
// At elapsed = hl: energy halves exactly (one half-life).
// hops_until_threshold: count divisions by 2 until energy ≤ 1.
//
// Item lifecycle (item_status):
//   0 = empty (slot never used)
//   1 = active (tracked by classify_all)
//   2 = archived (owner dismissed it; excluded from inbox counts)
//   3 = let_die (owner chose not to refresh; excluded from inbox counts)
//
// Cultural lineage: Getting Things Done (David Allen); Inbox Zero.
// §A5.4 doctrine: the inbox that opens on urgency, not coin balance.
//
// INVENTION_STACK.md §A5.4: Singh-Triage (Vital-Sign Wallet Inbox).
// Press claim: "the wallet that opens on inbox urgency, not coin balance."

contract SinghTriage {
    state {
        // Initialisation gate.
        sealed: bool = false

        // Urgency horizon thresholds in half-life hops (set once at initialise).
        horizon_today: u64 = 0
        horizon_tomorrow: u64 = 0
        horizon_week: u64 = 0

        // Per-item storage (slot = u64 key, auto-assigned sequentially).
        // item_status: 0=empty, 1=active, 2=archived, 3=let_die.
        item_energy: map[u64 -> u64]
        item_hl: map[u64 -> u64]
        item_anchor: map[u64 -> u64]
        item_status: map[u64 -> u64]
        item_count: u64 = 0

        // Inbox headline counts — stale until classify_all() is called.
        count_today: u64 = 0
        count_tomorrow: u64 = 0
        count_this_week: u64 = 0
        count_healthy: u64 = 0
        count_decayed: u64 = 0

        // count_archived is authoritative immediately on archive_item().
        count_archived: u64 = 0

        // Proof snapshots — set by require_urgent().
        last_urgent_slot: u64 = 0
        last_urgent_hops: u64 = 0
    }

    // Initialise once. Sets urgency horizons in half-life hops.
    // Enforces h_today < h_tomorrow < h_week.
    fn initialise(h_today: u64, h_tomorrow: u64, h_week: u64) {
        require(caller == owner, "only owner can initialise")
        require(self.sealed == false, "already initialised")
        require(h_today > 0, "today horizon must be positive")
        require(h_tomorrow > h_today, "horizons must be strictly ascending")
        require(h_week > h_tomorrow, "horizons must be strictly ascending")
        self.horizon_today = h_today
        self.horizon_tomorrow = h_tomorrow
        self.horizon_week = h_week
        self.sealed = true
        emit("triage.initialised")
    }

    // Register a new item. Auto-assigns next slot (= item_count).
    // init_energy: starting energy units. init_hl: item's decay half-life in epochs.
    fn register_item(init_energy: u64, init_hl: u64) {
        require(self.sealed == true, "not initialised")
        require(caller == owner, "only owner can register items")
        require(init_energy > 0, "energy must be positive")
        require(init_hl > 0, "half-life must be positive")
        let slot = self.item_count
        self.item_energy[slot] = init_energy
        self.item_hl[slot] = init_hl
        self.item_anchor[slot] = epoch
        self.item_status[slot] = 1
        self.item_count += 1
        emit("triage.registered")
    }

    // Classify all active items into urgency buckets.
    // Resets count_today / count_tomorrow / count_this_week / count_healthy /
    // count_decayed before recomputing. count_archived is not touched.
    // Anyone can call; no ownership restriction (observational function).
    fn classify_all() {
        require(self.sealed == true, "not initialised")
        self.count_today = 0
        self.count_tomorrow = 0
        self.count_this_week = 0
        self.count_healthy = 0
        self.count_decayed = 0
        let idx = 0
        while idx < self.item_count {
            if self.item_status[idx] == 1 {
                let elapsed = epoch - self.item_anchor[idx]
                let hl = self.item_hl[idx]
                let cur_e = self.item_energy[idx] * hl / (elapsed + hl)
                if cur_e <= 1 {
                    self.count_decayed += 1
                }
                if cur_e > 1 {
                    let hops = 0
                    let e_tmp = cur_e
                    while e_tmp > 1 {
                        e_tmp = e_tmp / 2
                        hops = hops + 1
                    }
                    if hops <= self.horizon_today {
                        self.count_today += 1
                    }
                    if hops > self.horizon_today {
                        if hops <= self.horizon_tomorrow {
                            self.count_tomorrow += 1
                        }
                        if hops > self.horizon_tomorrow {
                            if hops <= self.horizon_week {
                                self.count_this_week += 1
                            }
                            if hops > self.horizon_week {
                                self.count_healthy += 1
                            }
                        }
                    }
                }
            }
            idx = idx + 1
        }
        emit("triage.classified")
    }

    // Archive an active item (status 1 → 2). Immediately increments count_archived.
    fn archive_item(slot: u64) {
        require(self.sealed == true, "not initialised")
        require(caller == owner, "only owner can archive")
        require(self.item_status[slot] == 1, "slot not active")
        self.item_status[slot] = 2
        self.count_archived += 1
        emit("triage.archived")
    }

    // Let an item die (status 1 → 3). Owner chose not to refresh it.
    fn let_die_item(slot: u64) {
        require(self.sealed == true, "not initialised")
        require(caller == owner, "only owner can let die")
        require(self.item_status[slot] == 1, "slot not active")
        self.item_status[slot] = 3
        emit("triage.let.die")
    }

    // Refresh an item: decay its energy to now, add top_up, re-anchor.
    // Extends the item's runway — moves it toward Healthy in the next classify_all.
    fn refresh_item(slot: u64, top_up: u64) {
        require(self.sealed == true, "not initialised")
        require(caller == owner, "only owner can refresh")
        require(self.item_status[slot] == 1, "slot not active")
        require(top_up > 0, "top_up must be positive")
        let elapsed = epoch - self.item_anchor[slot]
        let hl = self.item_hl[slot]
        let cur_e = self.item_energy[slot] * hl / (elapsed + hl)
        self.item_energy[slot] = cur_e + top_up
        self.item_anchor[slot] = epoch
        emit("triage.refreshed")
    }

    // Proof gate: proves item at slot is in the Today bucket (hops ≤ horizon_today).
    // Stores slot + hops into last_urgent_slot / last_urgent_hops for telemetry.
    // Reverts if the item is archived, let-die, decayed, or beyond today horizon.
    fn require_urgent(slot: u64) {
        require(self.sealed == true, "not initialised")
        require(self.item_status[slot] == 1, "slot not active")
        let elapsed = epoch - self.item_anchor[slot]
        let hl = self.item_hl[slot]
        let cur_e = self.item_energy[slot] * hl / (elapsed + hl)
        require(cur_e > 1, "item already decayed — not urgent")
        let hops = 0
        let e_tmp = cur_e
        while e_tmp > 1 {
            e_tmp = e_tmp / 2
            hops = hops + 1
        }
        require(hops <= self.horizon_today, "item not urgent — beyond today horizon")
        self.last_urgent_slot = slot
        self.last_urgent_hops = hops
        emit("triage.urgent.confirmed")
    }

    on_grace() {
        emit("triage.energy.low — your inbox needs attention")
    }

    on_refresh() {
        emit("triage.refreshed")
    }

    on_evaporate() {
        emit("triage.evaporated — inbox abandoned")
    }
}
