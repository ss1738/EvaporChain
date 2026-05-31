// MnemoChain — Anki on-chain. Reference contract behind
// MNEMOCHAIN_CARD (0x0001_0302, Consumer lane), closing the LAST
// catalogue gap.
//
// Doctrine claim (from the catalogue): "Anki on-chain with FSRS
// forgetting curves. Portable cognitive credentials."
//
// One contract = one CARD. The deployer (owner) arm()s with a
// holder address, a content hash (off-chain payload — the
// question/answer side), and an initial stability. The holder
// review()s the card on each recall attempt, rating 1=Again /
// 2=Hard / 3=Good / 4=Easy; the contract updates stability +
// counters accordingly.
//
// FSRS is approximated, not implemented exactly. V1 EvaporScript
// has no `**` operator or bit-shift, so the proper exponential
// retrievability curve `R(t) = (1 + 19*t/(81*S))^(-0.5)` (FSRS-4.5
// shape) isn't expressible. We use a LINEAR retrievability:
// strength goes from 10000 basis points (=1.0) at review_epoch to
// 0 over `stability` epochs. The doctrine — "intervals double on
// Good, halve on Again" — holds exactly; only the within-window
// shape differs. V2 EvaporScript with `**` can swap the
// retrievability_bp() body without touching the rest.
//
// Stability progression (FSRS-inspired):
//   Again (1): stability /= 2, clamped to 1 minimum
//   Hard  (2): stability unchanged
//   Good  (3): stability *= 2
//   Easy  (4): stability *= 3
//
// Cards are TRANSFERABLE — the holder may transfer() to a new
// holder, who inherits the full review history (the portable
// cognitive credential pattern: prove you've memorised something
// without trusting the platform that taught it to you).

contract MnemoChain {
    state {
        holder: address
        card_content_hash: string = ""

        // ── FSRS-lite state ────────────────────────────────────────
        stability: u64 = 10
        last_review_epoch: u64 = 0
        // sentinel for last_review_epoch — epoch=0 vs never-reviewed
        // is the standard hazard from this session's other contracts.
        has_reviewed: bool = false

        // ── counters ───────────────────────────────────────────────
        review_count: u64 = 0
        total_again_count: u64 = 0
        total_good_count: u64 = 0
        // Hard reviews are tracked separately so analytics can
        // distinguish "barely remembered" from "remembered well."
        total_hard_count: u64 = 0

        sealed: bool = false
    }

    // Owner-only, one-shot: arm with (holder, content_hash, initial_stability).
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

    // Holder reviews the card with a rating (1=Again, 2=Hard,
    // 3=Good, 4=Easy). Stability mutates per the FSRS-lite table.
    fn review(rating: u64) {
        require(self.sealed == true, "not armed")
        require(caller == self.holder, "only holder reviews")
        require(rating >= 1, "rating must be 1-4")
        require(rating <= 4, "rating must be 1-4")
        if rating == 1 {
            // Again — memory broke; halve stability with min-1 floor.
            self.stability = self.stability / 2
            if self.stability < 1 {
                self.stability = 1
            }
            self.total_again_count += 1
        } else if rating == 2 {
            // Hard — remembered with effort; stability unchanged.
            self.total_hard_count += 1
        } else if rating == 3 {
            // Good — interval doubles (FSRS canonical).
            // EvaporScript V1 has no `*=`; use `field = field * N` form.
            self.stability = self.stability * 2
            self.total_good_count += 1
        } else {
            // Easy (rating == 4) — interval triples.
            self.stability = self.stability * 3
            self.total_good_count += 1
        }
        self.last_review_epoch = epoch
        self.has_reviewed = true
        self.review_count += 1
        emit("card reviewed")
    }

    // Current retrievability in basis points (0-10000).
    // V2 EXACT-EXPONENTIAL via `>>` (ES V2.0 operators, commit
    // cac72707, 2026-05-31). The V1 contract used linear
    // retrievability `10000 * (stability - age) / stability` as an
    // approximation because EvaporScript V1 had no `>>`. V2 halves
    // retrievability every `stability / 2` epochs:
    //   shift = (age × 2) / stability
    //   r_bp  = 10000 >> shift   (clamped to 0 at shift >= 64)
    // This is the FSRS-canonical exponential forgetting curve in
    // integer-truncated form. The (age × 2) / stability shape is
    // safer than `age / (stability / 2)` because it avoids the
    // divide-by-zero edge when stability=1 (the Again-halving floor).
    // Pre-first-review: 10000 (full strength, just learned).
    fn retrievability_bp() -> u64 {
        if self.has_reviewed == false {
            return 10000
        }
        if epoch < self.last_review_epoch {
            return 10000
        }
        if 2 * (epoch - self.last_review_epoch) / self.stability >= 64 {
            return 0
        }
        return 10000 >> (2 * (epoch - self.last_review_epoch) / self.stability)
    }

    // V2 due-for-review gate. With exponential decay there's no
    // clean 90% threshold (10000 >> shift never hits 9000 — the
    // first step below 10000 is 5000 at shift=1). So V2 fires due
    // when retrievability has at least halved (shift ≥ 1), which
    // corresponds to age ≥ stability / 2 — the memory's half-life.
    // The doctrine claim "due when the memory has half-faded" matches
    // FSRS's review-on-target-retrievability principle.
    fn is_due() -> bool {
        if self.has_reviewed == false {
            return false
        }
        if 2 * (epoch - self.last_review_epoch) >= self.stability {
            return true
        }
        return false
    }

    // Holder transfers the card. The full review history carries
    // over — the new holder gets to continue the streak.
    fn transfer(to: address) {
        require(self.sealed == true, "not armed")
        require(caller == self.holder, "only current holder transfers")
        self.holder = to
        emit("card transferred")
    }

    // ── Views ──────────────────────────────────────────────────────
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

    // V2: due-threshold age = stability / 2 (when retrievability
    // first halves to 5000bp). Already due? Return 0.
    fn epochs_until_due() -> u64 {
        if self.has_reviewed == false {
            return 0
        }
        if 2 * (epoch - self.last_review_epoch) >= self.stability {
            return 0
        }
        return self.last_review_epoch + self.stability / 2 - epoch
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
