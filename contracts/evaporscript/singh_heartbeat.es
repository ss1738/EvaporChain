// SinghHeartbeat (EvaporWallet-Pulse) — §A5.4 Ambient Signal.
//
// Persistent ambient signal encoding aggregate wallet energy as a pulse
// rate. Healthy: slow 60 bpm green pulse. Decay below threshold:
// arrhythmic red. Apple Watch complication shows sparkline of wallet's
// heartbeat over 24h.
//
// Vitals model (integer-only on-chain; validators agree on
// qualitative state, not pixels):
//
//   aggregate_health_bp = total_now * 100 / total_anchor  (0..100)
//     where total_now  = sum of decayed energies at epoch_now
//           total_anchor = sum of anchor energies at registration
//
//   bpm = 60 + 60 * (100 − health_bp) / 100
//     At health_bp=100: bpm=60 (resting).
//     At health_bp=0:   bpm=120 (alarmed).
//     Linear interpolation between extremes.
//
//   PulseColor (qualitative traffic-light, not RGB):
//     0 = Green  (health_bp ≥ 75) — everything is fine
//     1 = Amber  (40 ≤ health_bp < 75) — pay attention
//     2 = Red    (health_bp < 40) — something is dying
//
//   arrhythmia_amp = health_bp − worst_item_hp  (0..100)
//     Non-zero when one item is dying while others are healthy.
//     "What makes the wallet feel wrong before the user sees the inbox."
//
//   worst_item_hp = min over items of (cur_e * 100 / anchor_e)  (0..100)
//
// Empty wallet: bpm=60, color=Green, arrhythmia=0 — nothing wrong.
// Decay formula (hyperbolic): cur_e = anchor_e * hl / (elapsed + hl).
//
// Cultural lineage: Apple Watch heart-rate haptics; Tesla heartbeat-
// on-screen; Nest Leaf; Oura Ring rest signal.
//
// INVENTION_STACK.md §A5.4: Singh-Heartbeat (EvaporWallet-Pulse).
// Press claim: "your crypto wallet has a pulse."

contract SinghHeartbeat {
    state {
        // Per-item storage (slot = auto-assigned sequential u64 key).
        // item_energy: anchor energy at registration epoch.
        // item_hl:     decay half-life (epochs).
        // item_anchor: epoch of last registration (not refreshed here).
        item_energy: map[u64 -> u64]
        item_hl: map[u64 -> u64]
        item_anchor: map[u64 -> u64]
        item_count: u64 = 0

        // Last computed vitals (written by pulse(); stale until pulse() called).
        // BPM: 60=resting-healthy, 120=alarmed. Linear in aggregate health.
        last_bpm: u64 = 60
        // PulseColor: 0=Green, 1=Amber, 2=Red. Qualitative state.
        last_color: u64 = 0
        // Arrhythmia amplitude 0..100: gap between aggregate and worst item.
        last_arrhythmia: u64 = 0
        // Aggregate health in basis points /100 (0..100).
        last_health_bp: u64 = 100
        // Worst item health in basis points /100 (0..100).
        last_worst_hp: u64 = 100
        // How many times pulse() has been called.
        pulse_count: u64 = 0
    }

    // Register a wallet item. Owner-only.
    // init_energy: anchor energy (positive).
    // init_hl:     item's decay half-life in epochs (positive).
    fn register_item(init_energy: u64, init_hl: u64) {
        require(caller == owner, "only wallet owner can register items")
        require(init_energy > 0, "energy must be positive")
        require(init_hl > 0, "half-life must be positive")
        let slot = self.item_count
        self.item_energy[slot] = init_energy
        self.item_hl[slot] = init_hl
        self.item_anchor[slot] = epoch
        self.item_count += 1
        emit("heartbeat.item.registered")
    }

    // Compute wallet vitals from current item states. Anyone can call.
    // Iterates all items, applies hyperbolic decay, writes vitals to state.
    // Calling pulse() repeatedly updates last_bpm / last_color / etc.
    fn pulse() {
        let idx = 0
        let total_now = 0
        let total_anchor = 0
        let worst_hp = 100
        while idx < self.item_count {
            let hl = self.item_hl[idx]
            let anchor_e = self.item_energy[idx]
            let elapsed = epoch - self.item_anchor[idx]
            let cur_e = anchor_e * hl / (elapsed + hl)
            total_now = total_now + cur_e
            total_anchor = total_anchor + anchor_e
            let item_hp = cur_e * 100 / anchor_e
            if item_hp > 100 {
                item_hp = 100
            }
            if item_hp < worst_hp {
                worst_hp = item_hp
            }
            idx = idx + 1
        }
        let health_bp = 100
        if total_anchor > 0 {
            health_bp = total_now * 100 / total_anchor
            if health_bp > 100 {
                health_bp = 100
            }
        }
        let bpm = 60 + 60 * (100 - health_bp) / 100
        let arrhythmia = 0
        if health_bp > worst_hp {
            arrhythmia = health_bp - worst_hp
        }
        let color = 2
        if health_bp > 39 {
            color = 1
        }
        if health_bp > 74 {
            color = 0
        }
        self.last_bpm = bpm
        self.last_color = color
        self.last_arrhythmia = arrhythmia
        self.last_health_bp = health_bp
        self.last_worst_hp = worst_hp
        self.pulse_count += 1
        emit("heartbeat.pulsed")
    }

    // Proof gate: proves wallet is healthy (last pulse showed Green).
    // Reverts if pulse never computed or last_color != 0 (Green).
    fn require_healthy() {
        require(self.pulse_count > 0, "pulse not yet computed")
        require(self.last_color == 0, "wallet not healthy — pulse is Amber or Red")
        emit("heartbeat.healthy.confirmed")
    }

    // Proof gate: proves wallet has an arrhythmic signal.
    // Non-zero arrhythmia = at least one item is dying while others remain healthy.
    // "What makes the wallet feel wrong before the user sees the inbox."
    fn require_arrhythmic() {
        require(self.pulse_count > 0, "pulse not yet computed")
        require(self.last_arrhythmia > 0, "pulse is regular — no arrhythmia detected")
        emit("heartbeat.arrhythmic.confirmed")
    }

    on_grace() {
        emit("heartbeat.energy.low — wallet energy fading")
    }

    on_refresh() {
        emit("heartbeat.refreshed")
    }

    on_evaporate() {
        emit("heartbeat.evaporated — wallet pulse gone")
    }
}
