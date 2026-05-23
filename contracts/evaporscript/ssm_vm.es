// SinghStrategyMachines (SSM) — §A5.1 game-semantic contracts.
//
// Doctrine: INVENTION_STACK.md §A5.1. Cultural precedent: Hyland-Ong-Nickau
// 1994 (game semantics for PCF); Abramsky-Jagadeesan-Malacaria 2000;
// Ghica-McCusker IPA 2003.
//
// Formalism: HO arenas + innocent strategies. A contract is an *arena* A;
// a program is an *innocent strategy* σ : A. Execution = play between
// Proponent P (contract) and Opponent O (callers / adversary).
//
// Decay synergy — STRUCTURAL:
//   Game arenas have *justification pointers*: move k is *justified* by
//   earlier move j (j < k). A play is valid iff every move's justifier
//   has positive residual energy — the *visibility condition*. Decay is
//   exactly the visibility-condition erasure: when a P-move's energy falls
//   to 0, every O-challenge it justified becomes invisible and cannot
//   proceed. No explicit revocation; the game tree prunes itself.
//
//   This is the paradigm-grade claim: "the contract is a proof you can win
//   against any adversary, mechanically." Decay is the mechanism that makes
//   the proof *time-bounded* — justification chains cannot outlive the
//   energy of the moves that justify them.
//
// Game structure:
//   O plays the initial move (no justifier; justifier=999 root sentinel).
//   P responds to an O-move (P-move justified by the O-move it answers).
//   O challenges a P-move (O-challenge justified by the P-move it disputes).
//   Proponent wins a round iff every visible O-move has a live P-response.
//
// Contract operations:
//   o_move(init_energy)         — O plays root move (justifier=999).
//   p_respond(o_slot, init_energy) — P answers O-move; sets move_response.
//   o_challenge(p_slot, init_energy) — O challenges a P-move; justified by it.
//   drain_move(slot, amount)    — consume visibility energy (simulates decay).
//   witness_move(slot)          — snapshot player+energy+justifier_energy.
//   check_strategy()            — verify P has live response to all visible O-moves.
//   require_strategy_holds()    — gate: check_ran + strategy_holds == 1.
//   require_move_invisible(slot) — gate: move's justifier has energy=0 (pruned).
//
// Visibility rule: an O-move is visible iff its justifier has energy > 0
//   (or it is the initial move with justifier=999). An invisible O-move
//   requires no P-response; the game implicitly concedes that subtree.
//
// Deploy-script demo:
//   strategy mode: o_move(1000) → p_respond(0, 800) → o_challenge(1, 600) →
//     p_respond(2, 500) → witness_move(0, snap1) → witness_move(1, snap2) →
//     check_strategy → require_strategy_holds PASSED.
//     Proves P has a live strategy covering all visible O-moves.
//
//   decay mode: o_move(1000) → p_respond(0, 800) → o_challenge(1, 600) →
//     drain_move(slot=1, 800) → witness_move(2, snap1: justifier_energy=0) →
//     require_move_invisible(2) PASSED.
//     Proves: draining P-move slot=1 makes O-challenge slot=2 invisible.
//     "The game tree prunes itself — no explicit revocation needed."
//
// Press claim: "the contract is a proof you can win against any adversary,
// mechanically. Decay is the visibility condition: unjustified-by-decayed-
// move means illegal play, means the adversary's challenge disappears."
//
// INVENTION_STACK.md §A5.1: SinghStrategyMachines.

contract SinghStrategyMachines {
    state {
        // Per-move storage. Slot = auto-assigned sequential u64.
        move_player: map[u64 -> u64]      // 0=Opponent, 1=Proponent
        move_justifier: map[u64 -> u64]   // justification pointer (999 = root/initial)
        move_energy: map[u64 -> u64]      // visibility budget (0 = invisible, pruned from game)
        move_response: map[u64 -> u64]    // for O-moves: P-move slot answering it (999=none)
        move_present: map[u64 -> u64]     // 0=absent, 1=registered
        move_count: u64 = 0

        // Strategy check result (written by check_strategy).
        check_ran: u64 = 0
        strategy_holds: u64 = 0          // 1 = P has live response to all visible O-moves

        // Witness snapshots (player + energy + justifier_energy at call time).
        witness_count: u64 = 0
        snapshot1_slot: u64 = 0
        snapshot1_player: u64 = 0
        snapshot1_energy: u64 = 0
        snapshot1_justifier_energy: u64 = 0
        snapshot2_slot: u64 = 0
        snapshot2_player: u64 = 0
        snapshot2_energy: u64 = 0
        snapshot2_justifier_energy: u64 = 0
    }

    // Opponent plays the initial (root) move. No justifier required.
    // init_energy: visibility budget for this move.
    // This is the first move in the game; all subsequent play is justified
    // transitively from this root.
    fn o_move(init_energy: u64) {
        require(caller == owner, "only owner can add moves to the game arena")
        require(init_energy > 0, "move energy must be positive — zero energy = immediately invisible")
        let slot = self.move_count
        self.move_player[slot] = 0
        self.move_justifier[slot] = 999   // root sentinel: no justifier
        self.move_energy[slot] = init_energy
        self.move_response[slot] = 999    // no P-response yet
        self.move_present[slot] = 1
        self.move_count += 1
        emit("ssm.move.opponent.played")
    }

    // Proponent responds to an O-move. The P-move is justified by the O-move it answers.
    // Also records the response pointer: move_response[o_slot] = new P-move slot.
    // Responding to an O-move is the core of an innocent strategy: P commits
    // to the *specific O-move* it is answering.
    fn p_respond(o_slot: u64, init_energy: u64) {
        require(caller == owner, "only owner can add moves to the game arena")
        require(o_slot < self.move_count, "O-move slot not found")
        require(self.move_present[o_slot] == 1, "O-move not registered")
        require(self.move_player[o_slot] == 0, "p_respond: target must be an Opponent move (player=0)")
        require(init_energy > 0, "move energy must be positive")
        let slot = self.move_count
        self.move_player[slot] = 1
        self.move_justifier[slot] = o_slot  // P-move is justified by the O-move it answers
        self.move_energy[slot] = init_energy
        self.move_response[slot] = 999      // P-moves do not have a response field
        self.move_response[o_slot] = slot   // link: O-move is answered by this new P-move
        self.move_present[slot] = 1
        self.move_count += 1
        emit("ssm.move.proponent.responded")
    }

    // Opponent challenges a P-move. The O-challenge is justified by the P-move it disputes.
    // When the P-move's energy decays to 0, this O-challenge becomes invisible
    // (justifier dead → visibility condition fails → challenge pruned structurally).
    fn o_challenge(p_slot: u64, init_energy: u64) {
        require(caller == owner, "only owner can add moves to the game arena")
        require(p_slot < self.move_count, "P-move slot not found")
        require(self.move_present[p_slot] == 1, "P-move not registered")
        require(self.move_player[p_slot] == 1, "o_challenge: target must be a Proponent move (player=1)")
        require(init_energy > 0, "move energy must be positive")
        let slot = self.move_count
        self.move_player[slot] = 0
        self.move_justifier[slot] = p_slot  // O-challenge justified by the P-move
        self.move_energy[slot] = init_energy
        self.move_response[slot] = 999      // no P-response yet
        self.move_present[slot] = 1
        self.move_count += 1
        emit("ssm.move.opponent.challenged")
    }

    // Drain visibility energy from a move. Simulates decay erasing a move from the arena.
    // When a P-move's energy hits 0, every O-challenge it justified becomes invisible.
    // This is the structural pruning claim — no explicit revocation, only energy exhaustion.
    fn drain_move(slot: u64, amount: u64) {
        require(slot < self.move_count, "move slot not found")
        require(self.move_present[slot] == 1, "move not registered")
        require(amount > 0, "drain amount must be positive")
        require(self.move_energy[slot] >= amount, "drain amount exceeds move energy — would underflow")
        self.move_energy[slot] = self.move_energy[slot] - amount
        emit("ssm.move.drained")
    }

    // Witness: snapshot player + energy + justifier_energy for slot at call time.
    // Root moves (justifier=999) have justifier_energy=0 (sentinel has no energy).
    // First call → snapshot1, second call → snapshot2.
    fn witness_move(slot: u64) {
        require(slot < self.move_count, "move slot not found")
        require(self.move_present[slot] == 1, "move not registered")
        let p = self.move_player[slot]
        let e = self.move_energy[slot]
        let jus = self.move_justifier[slot]
        let je = 0
        if jus != 999 {
            je = self.move_energy[jus]
        }
        if self.witness_count == 0 {
            self.snapshot1_slot = slot
            self.snapshot1_player = p
            self.snapshot1_energy = e
            self.snapshot1_justifier_energy = je
        }
        if self.witness_count == 1 {
            self.snapshot2_slot = slot
            self.snapshot2_player = p
            self.snapshot2_energy = e
            self.snapshot2_justifier_energy = je
        }
        self.witness_count += 1
        emit("ssm.move.witnessed")
    }

    // Check P's strategy: verify P has a live response to every visible O-move.
    // Visibility rule: an O-move is visible iff its justifier has energy > 0,
    //   OR it is the initial move (justifier=999 root sentinel).
    // strategy_holds = 1 if all visible O-moves have live P-responses.
    // A live P-response: move_response[o_slot] != 999 AND that P-move has energy > 0.
    fn check_strategy() {
        let i = 0
        let fails = 0
        while i < self.move_count {
            if self.move_present[i] == 1 {
                if self.move_player[i] == 0 {
                    let jus = self.move_justifier[i]
                    let jus_alive = 1
                    if jus != 999 {
                        let je = self.move_energy[jus]
                        if je == 0 {
                            jus_alive = 0
                        }
                    }
                    if jus_alive == 1 {
                        let resp = self.move_response[i]
                        if resp == 999 {
                            fails = fails + 1
                        }
                        if resp != 999 {
                            let re = self.move_energy[resp]
                            if re == 0 {
                                fails = fails + 1
                            }
                        }
                    }
                }
            }
            i = i + 1
        }
        if fails == 0 {
            self.strategy_holds = 1
        }
        self.check_ran = 1
        emit("ssm.strategy.checked")
    }

    // Proof gate: P has a complete live strategy — all visible O-moves are answered.
    // "The contract is a proof you can win against any adversary, mechanically."
    fn require_strategy_holds() {
        require(self.check_ran == 1, "check_strategy not yet called")
        require(self.strategy_holds == 1, "strategy does not hold — some visible O-move is unanswered or its P-response has zero energy")
        emit("ssm.strategy.confirmed")
    }

    // Proof gate: the move at slot has become invisible — its justifier has energy=0.
    // Proves structural pruning: draining a P-move kills every O-challenge it justified.
    // "The game tree prunes itself — the adversary's challenge disappears."
    fn require_move_invisible(slot: u64) {
        require(slot < self.move_count, "move slot not found")
        require(self.move_present[slot] == 1, "move not registered")
        let jus = self.move_justifier[slot]
        require(jus != 999, "initial moves have no justifier — only justified moves can become invisible")
        let je = self.move_energy[jus]
        require(je == 0, "justifier still has positive energy — move is still visible in the arena")
        emit("ssm.move.invisible.confirmed")
    }

    on_grace() {
        emit("ssm.energy.low — game arena evaporating")
    }

    on_refresh() {
        emit("ssm.refreshed")
    }

    on_evaporate() {
        emit("ssm.evaporated — game arena gone")
    }
}
