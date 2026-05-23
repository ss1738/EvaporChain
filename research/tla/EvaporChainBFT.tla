--------------------------- MODULE EvaporChainBFT ---------------------------
(*
    TLA+ formal specification of EvaporChain's Tendermint BFT consensus
    with energy-based state decay, Rule-Based Consensus (BlockStateCommitment),
    and Data Availability sampling.

    Models the core safety and liveness properties of the protocol as
    implemented in evaporchain-consensus/src/tendermint.rs.

    Author: Satyawan Singh
    Date:   2026-04-21 (D11 axiom-boundary section added 2026-05-21)

    Safety:  No two honest validators commit different blocks at the same height.
    Liveness: If 2f+1 validators are honest and connected, a new block is
              eventually committed.

    Properties verified by TLC:

        TypeOK                       — all variables in declared domains.
        Agreement                    — no two honest validators commit
                                       different blocks at the same height.
        Validity                     — only proposed blocks can be committed.
        CommitRequiresQuorum         — a block is committed only if it
                                       received 2f+1 stake-weighted
                                       precommits in some round.
        LockSafety                   — if an honest validator is locked on
                                       block lb in round lr, no quorum can
                                       have formed for a different block in
                                       any earlier round (currently fails
                                       on Byzantine.cfg under late-prevote
                                       traces — see audit doc § D14).
        EquivocationDetected         — equivocating validators are slashed.
        StateCommitmentIntegrity     — every committed block has a
                                       state-commitment recorded at its
                                       height (binary abstraction: "None"
                                       vs "Committed").

    Open and not modeled here (out of TLC scope — same axiom boundary as
    PoHA.tla):

        — Real cryptographic verification of BLS aggregate signatures.
          Modeled abstractly via the prevote/precommit message sets;
          signature validity is treated as an axiom. See Coq's
          `PoHAFreeloading.v` for the crypto-game side (axiomatized).
        — Domain-separation-tag (DST) collisions across the BLS keyspace
          (audit 2026-05-17 H-2): treated as cryptographically negligible.
        — VRF unforgeability for proposer selection.
        — Hash collision resistance (BLAKE3) of block / proposal hashes.
        — Network adversary's ability to delay / partition / reorder
          messages: modeled as nondeterminism in action interleavings,
          but specific Byzantine network strategies (e.g. selective
          forwarding to exploit POL gaps) are out of scope.
        — Proof-of-Lock (POL) / valid-round propagation as implemented in
          tendermint.rs:5478-5544 (audit doc § D14).
        — Antichain mempool drain & cross-fork equivocation tracking
          (audit doc § D3 + § D10).
        — Dynamic validator set: key rotation, jailing, tombstoning,
          epoch transitions (audit doc § D7).

    See `research/tla/TLA_IMPL_DRIFT_AUDIT_2026_05_21.md` for the full
    spec-vs-impl drift accounting.
*)

EXTENDS Integers, Sequences, FiniteSets, TLC

\* ══════════════════════════════════════════════════════════════════════════
\* Constants
\* ══════════════════════════════════════════════════════════════════════════

CONSTANTS
    Validators,         \* Set of validator IDs (e.g., {0, 1, 2})
    MaxRound,           \* Maximum rounds per height before forced commit (10)
    MaxHeight,          \* Bound on heights for model checking
    Faulty,             \* Set of Byzantine validators (|Faulty| < n/3 by STAKE)
    FaultyStake,        \* Nat>0 : per-validator stake of Faulty validators
    HonestStake         \* Nat>0 : per-validator stake of Honest validators

ASSUME Faulty \subseteq Validators
ASSUME FaultyStake >= 1
ASSUME HonestStake >= 1

\* D1/D5 fix 2026-05-21: ValidatorStake function is derived inside the
\* spec rather than supplied as a CONSTANT, because TLC .cfg files only
\* accept identifier-keyed record literals, not integer-keyed function
\* literals like `[0 |-> ..., 1 |-> ...]`. The two-scalar shape lets
\* configs select either uniform stake (FaultyStake = HonestStake) or
\* a Byzantine attacker with disparate stake (FaultyStake < HonestStake)
\* without changing the spec.
ValidatorStake == [v \in Validators |-> IF v \in Faulty THEN FaultyStake ELSE HonestStake]

\* ══════════════════════════════════════════════════════════════════════════
\* Derived constants
\* ══════════════════════════════════════════════════════════════════════════

N == Cardinality(Validators)

\* Sum of stake of a set of validators (D1/D5 fix 2026-05-21: mirrors
\* PoHA.tla:104-111 but iterates the validator SET, not the set of
\* stake VALUES, so two validators with equal stake are correctly
\* counted twice).
StakeOf(VSet) ==
    LET RECURSIVE Sum(_)
        Sum(S) ==
            IF S = {} THEN 0
            ELSE LET x == CHOOSE x \in S : TRUE
                 IN ValidatorStake[x] + Sum(S \ {x})
    IN Sum(VSet)

TotalStake == StakeOf(Validators)

\* Byzantine assumption is now STAKE-weighted, matching Rust's
\* `attested_stake * 3 > total_stake * 2` predicate
\* (`certificate.rs:55-58`, `finality.rs:56-59`).
\* Under unequal stake, count-based `|Faulty| * 3 < |Validators|`
\* is INSUFFICIENT — a small-stake majority by count can be a
\* large-stake minority. The strict-`>` inequality matches the
\* Rust u128-safe implementation.
ASSUME StakeOf(Faulty) * 3 < TotalStake

Honest == Validators \ Faulty

\* Block values: Nil means no block proposed / voted for
BlockValues == {"BlockA", "BlockB", "Nil"}
NonNilBlocks == BlockValues \ {"Nil"}

\* ══════════════════════════════════════════════════════════════════════════
\* Phase enum — matches Rust Phase { Propose, Prevote, Precommit, Commit }
\* ══════════════════════════════════════════════════════════════════════════

Phases == {"Propose", "Prevote", "Precommit", "Commit"}

\* ══════════════════════════════════════════════════════════════════════════
\* Variables
\* ══════════════════════════════════════════════════════════════════════════

VARIABLES
    height,             \* height[v] : current height of validator v
    round,              \* round[v]  : current round of validator v
    phase,              \* phase[v]  : current phase of validator v
    lockedBlock,        \* lockedBlock[v] : block v is locked on ("Nil" if none)
    lockedRound,        \* lockedRound[v] : round at which v locked (-1 if none)
    validBlock,         \* validBlock[v]  : latest valid block seen
    validRound,         \* validRound[v]  : round of latest valid block (-1 if none)
    prevotes,           \* prevotes[h][r] : set of <<v, block>> prevote messages
    precommits,         \* precommits[h][r] : set of <<v, block>> precommit messages
    proposals,          \* proposals[h][r] : set of <<proposer, block>> proposals
    committed,          \* committed[v]   : sequence of committed <<height, block>> pairs
    daAttested,         \* daAttested[h]  : set of validators that attested DA for height h
    stateCommitment,    \* stateCommitment[h] : BlockStateCommitment included at height h
    equivocations,      \* equivocations  : set of detected equivocating validators
    slashed             \* slashed        : set of slashed validators

vars == <<height, round, phase, lockedBlock, lockedRound, validBlock, validRound,
          prevotes, precommits, proposals, committed, daAttested,
          stateCommitment, equivocations, slashed>>

\* ══════════════════════════════════════════════════════════════════════════
\* Helper operators
\* ══════════════════════════════════════════════════════════════════════════

\* Count votes for a particular block value in a vote set
VotesForBlock(voteSet, block) ==
    Cardinality({m \in voteSet : m[2] = block})

\* Stake-weighted sum of votes for a particular block value
\* (D1/D5 fix 2026-05-21).
StakeVotesForBlock(voteSet, block) ==
    StakeOf({m[1] : m \in {m \in voteSet : m[2] = block}})

\* Check if any non-nil block has quorum in vote set.
\* STAKE-WEIGHTED STRICT-> matches Rust:
\*   `attested_stake * 3 > total_stake * 2`
\* (`certificate.rs:55-58`, `finality.rs:56-59`).
\* D1/D5 fix 2026-05-21: was `>= Quorum` count-based; that was
\* equivalent only under equal stake.
HasQuorumFor(voteSet, block) ==
    StakeVotesForBlock(voteSet, block) * 3 > TotalStake * 2

\* Check if nil has quorum (stake-weighted, same predicate shape).
HasNilQuorum(voteSet) ==
    StakeVotesForBlock(voteSet, "Nil") * 3 > TotalStake * 2

\* Total votes cast (cardinality — unchanged; some liveness
\* arguments still legitimately want a count not a stake sum).
TotalVotes(voteSet) == Cardinality(voteSet)

\* Total stake committed to a vote set (sum of distinct voter stakes).
TotalVoteStake(voteSet) == StakeOf({m[1] : m \in voteSet})

\* Get the quorum block if one exists
QuorumBlock(voteSet) ==
    CHOOSE b \in NonNilBlocks : HasQuorumFor(voteSet, b)

\* Validator has already voted in this round's vote set
HasVoted(voteSet, v) ==
    \E m \in voteSet : m[1] = v

\* ══════════════════════════════════════════════════════════════════════════
\* Message sets initialization
\* ══════════════════════════════════════════════════════════════════════════

EmptyVoteMap == [h \in 1..MaxHeight |-> [r \in 0..MaxRound |-> {}]]
EmptyProposalMap == [h \in 1..MaxHeight |-> [r \in 0..MaxRound |-> {}]]

\* ══════════════════════════════════════════════════════════════════════════
\* Initial state
\* ══════════════════════════════════════════════════════════════════════════

Init ==
    /\ height       = [v \in Validators |-> 1]
    /\ round        = [v \in Validators |-> 0]
    /\ phase        = [v \in Validators |-> "Propose"]
    /\ lockedBlock  = [v \in Validators |-> "Nil"]
    /\ lockedRound  = [v \in Validators |-> -1]
    /\ validBlock   = [v \in Validators |-> "Nil"]
    /\ validRound   = [v \in Validators |-> -1]
    /\ prevotes     = EmptyVoteMap
    /\ precommits   = EmptyVoteMap
    /\ proposals    = EmptyProposalMap
    /\ committed    = [v \in Validators |-> << >>]
    /\ daAttested   = [h \in 1..MaxHeight |-> {}]
    /\ stateCommitment = [h \in 1..MaxHeight |-> "None"]
    /\ equivocations = {}
    /\ slashed      = {}

\* ══════════════════════════════════════════════════════════════════════════
\* Actions
\* ══════════════════════════════════════════════════════════════════════════

\* ─────────────────── Propose ────────────────────────────────────────────

\* Honest proposer creates a block proposal
\* Matches tick() Phase::Propose when am_i_proposer()
ProposeBlock(v) ==
    /\ v \in Honest
    /\ height[v] <= MaxHeight
    /\ phase[v] = "Propose"
    /\ round[v] <= MaxRound
    \* v is the designated proposer for this height/round
    \* (abstracted: we allow any honest validator to propose if it's their turn)
    /\ LET h == height[v]
           r == round[v]
       IN
       \* Choose which block to propose
       \* If locked, must re-propose locked block (Tendermint rule)
       \* If valid block exists from higher round, use it
       LET block == IF lockedBlock[v] # "Nil" THEN lockedBlock[v]
                    ELSE IF validBlock[v] # "Nil" THEN validBlock[v]
                    ELSE "BlockA"  \* Fresh proposal
       IN
       /\ proposals' = [proposals EXCEPT ![h][r] = @ \cup {<<v, block>>}]
       \* Include BlockStateCommitment in every block header
       \* (Rule-Based Consensus: anchor_ref + decay_rules_hash + commitment_hash)
       /\ stateCommitment' = [stateCommitment EXCEPT ![h] =
            IF @ = "None" THEN "Committed" ELSE @]
       \* Self-prevote for own proposal (matches Rust: immediate self-prevote after propose)
       /\ prevotes' = [prevotes EXCEPT ![h][r] = @ \cup {<<v, block>>}]
       /\ phase' = [phase EXCEPT ![v] = "Prevote"]
       \* DA self-attestation on own block
       /\ daAttested' = [daAttested EXCEPT ![h] = @ \cup {v}]
       /\ UNCHANGED <<height, round, lockedBlock, lockedRound, validBlock, validRound,
                       precommits, committed, equivocations, slashed>>

\* Byzantine proposer can propose anything, including equivocating blocks.
\*
\* D12 fix 2026-05-21: must update stateCommitment[h] like ProposeBlock
\* does (line 207-209). The spec's stateCommitment abstraction is binary:
\* "None" until any block proposal lands at height h, "Committed" once a
\* proposal exists. Previously ByzantinePropose left stateCommitment
\* UNCHANGED, so a Byzantine-proposed block that honest validators voted
\* on and committed would leave stateCommitment[h] = "None", spuriously
\* violating StateCommitmentIntegrity. In the Rust impl, every block
\* header carries a BlockStateCommitment regardless of proposer identity
\* — Byzantine proposers might supply an INVALID BSC, but they can't
\* omit the field; honest validators reject blocks with malformed BSC
\* (validation logic abstracted out of this spec). The simple binary
\* abstraction lets us model "BSC present" without modeling validity.
ByzantinePropose(v) ==
    /\ v \in Faulty
    /\ height[v] <= MaxHeight
    /\ phase[v] = "Propose"
    /\ LET h == height[v]
           r == round[v]
       IN
       \* Faulty proposer may propose any block, even two different ones (equivocation)
       \E block \in NonNilBlocks :
           /\ proposals' = [proposals EXCEPT ![h][r] = @ \cup {<<v, block>>}]
           /\ stateCommitment' = [stateCommitment EXCEPT ![h] =
                IF @ = "None" THEN "Committed" ELSE @]
           /\ prevotes' = [prevotes EXCEPT ![h][r] = @ \cup {<<v, block>>}]
           /\ phase' = [phase EXCEPT ![v] = "Prevote"]
           /\ UNCHANGED <<height, round, lockedBlock, lockedRound, validBlock, validRound,
                           precommits, committed, daAttested,
                           equivocations, slashed>>

\* Propose timeout — no proposal received, send nil prevote
\* Matches tick() propose_timeout path
ProposeTimeout(v) ==
    /\ v \in Honest
    /\ height[v] <= MaxHeight
    /\ phase[v] = "Propose"
    /\ LET h == height[v]
           r == round[v]
       IN
       /\ ~HasVoted(prevotes[h][r], v)
       /\ prevotes' = [prevotes EXCEPT ![h][r] = @ \cup {<<v, "Nil">>}]
       /\ phase' = [phase EXCEPT ![v] = "Prevote"]
       /\ UNCHANGED <<height, round, lockedBlock, lockedRound, validBlock, validRound,
                       precommits, proposals, committed, daAttested,
                       stateCommitment, equivocations, slashed>>

\* ─────────────────── Prevote (receive proposal + vote) ──────────────────

\* Honest validator receives proposal and sends prevote
\* Matches on_message() ConsensusMessage::Proposal handling.
\*
\* D14 fix 2026-05-21: gate the action on "no later-round activity
\* exists for this height" — this models Rust's on_message stale-
\* round handling at tendermint.rs:4998-5011, where a validator
\* receiving a round-r message while round_state.round > r drops
\* the message, and a round_state.round < msg.round() triggers
\* RoundSkip before processing. The net effect: a validator cannot
\* vote in a stale round when later-round messages are visible.
\*
\* Without this gate, TLC's nondeterministic interleaving lets a
\* round-r validator vote in round r EVEN AFTER other validators
\* advanced and produced round (r+1) activity, retrospectively
\* completing a stake-quorum for a different block in round r that
\* the now-locked validator in round r+1 couldn't have observed.
\* This was the LockSafety violation surfaced after D12 closed the
\* depth-8 StateCommitmentIntegrity mask (audit doc § D14).
\*
\* The gate is essentially "RoundSkip(v) is not currently enabled
\* OR v is already at the latest round with activity" — meaning the
\* validator must take RoundSkip BEFORE this action when both are
\* enabled. Combined with weak fairness on RoundSkip, this matches
\* the Rust ordering exactly.
ReceiveProposalAndPrevote(v) ==
    /\ v \in Honest
    /\ height[v] <= MaxHeight
    /\ phase[v] \in {"Propose", "Prevote"}
    /\ LET h == height[v]
           r == round[v]
       IN
       \* D14 gate: no later-round activity visible at this height
       /\ ~(\E futureR \in (r+1)..MaxRound :
               \E w \in Validators, b \in BlockValues :
                   \/ <<w, b>> \in prevotes[h][futureR]
                   \/ <<w, b>> \in precommits[h][futureR])
       /\ \E p \in Validators, block \in NonNilBlocks :
           /\ <<p, block>> \in proposals[h][r]
           /\ ~HasVoted(prevotes[h][r], v)
           \* Lock check (D2 fix 2026-05-21): mirror Rust's stricter rule from
           \* tendermint.rs:5532-5544 — once locked on a block, vote for that
           \* block; if locked on a different block, vote Nil regardless of
           \* lockedRound. The previous spec admitted a more permissive branch
           \* (vote for new block whenever lockedRound < r) that the Rust
           \* implementation never takes; that branch let the model reach
           \* states the production code cannot, so TLC was proving a weaker
           \* property than what we actually ship.
           /\ LET voteBlock ==
                  IF lockedBlock[v] = "Nil" THEN block
                  ELSE IF lockedBlock[v] = block THEN block
                  ELSE "Nil"
              IN
              /\ prevotes' = [prevotes EXCEPT ![h][r] = @ \cup {<<v, voteBlock>>}]
              /\ phase' = [phase EXCEPT ![v] = "Prevote"]
              \* DA sampling: if voting for block, attest data availability
              /\ daAttested' = IF voteBlock # "Nil"
                               THEN [daAttested EXCEPT ![h] = @ \cup {v}]
                               ELSE daAttested
              /\ UNCHANGED <<height, round, lockedBlock, lockedRound, validBlock, validRound,
                              precommits, proposals, committed, stateCommitment,
                              equivocations, slashed>>

\* ─────────────────── Prevote → Precommit transition ─────────────────────

\* 2f+1 prevotes for a block → lock and precommit
\* Matches tick() Phase::Prevote when check_prevote_quorum() returns Some(Some(hash))
PrevoteQuorumReached(v) ==
    /\ v \in Honest
    /\ height[v] <= MaxHeight
    /\ phase[v] = "Prevote"
    /\ LET h == height[v]
           r == round[v]
       IN
       \* There exists a non-nil block with 2f+1 prevotes
       /\ \E block \in NonNilBlocks :
           /\ HasQuorumFor(prevotes[h][r], block)
           \* Lock on this block (matches Rust: locked_block = proposed_block, locked_round = round)
           /\ lockedBlock' = [lockedBlock EXCEPT ![v] = block]
           /\ lockedRound' = [lockedRound EXCEPT ![v] = r]
           /\ validBlock' = [validBlock EXCEPT ![v] = block]
           /\ validRound' = [validRound EXCEPT ![v] = r]
           \* Send precommit for block
           /\ precommits' = [precommits EXCEPT ![h][r] = @ \cup {<<v, block>>}]
           /\ phase' = [phase EXCEPT ![v] = "Precommit"]
           /\ UNCHANGED <<height, round, prevotes, proposals, committed,
                           daAttested, stateCommitment, equivocations, slashed>>

\* 2f+1 nil prevotes or timeout → precommit nil
\* Matches tick() prevote_timeout path
PrevoteNilQuorumOrTimeout(v) ==
    /\ v \in Honest
    /\ height[v] <= MaxHeight
    /\ phase[v] = "Prevote"
    /\ LET h == height[v]
           r == round[v]
       IN
       \* D14 fix 2026-05-21: the timeout-fallback must NOT fire when
       \* any block has stake quorum — otherwise this action can
       \* preempt the lock that PrevoteQuorumReached should set,
       \* admitting a LockSafety violation under stake-weighted Byzantine
       \* model checking. In Rust, PrevoteQuorumReached's effect is
       \* OBSERVATIONAL on the same vote stream — a validator that sees
       \* the stake quorum locks; only when no block quorum exists does
       \* the timer-driven nil precommit fire.
       /\ \/ HasNilQuorum(prevotes[h][r])
          \/ /\ TotalVoteStake(prevotes[h][r]) * 3 > TotalStake * 2
             /\ \A b \in NonNilBlocks : ~HasQuorumFor(prevotes[h][r], b)
       /\ precommits' = [precommits EXCEPT ![h][r] = @ \cup {<<v, "Nil">>}]
       /\ phase' = [phase EXCEPT ![v] = "Precommit"]
       /\ UNCHANGED <<height, round, lockedBlock, lockedRound, validBlock, validRound,
                       prevotes, proposals, committed, daAttested,
                       stateCommitment, equivocations, slashed>>

\* ─────────────────── Precommit → Commit / Next Round ────────────────────

\* 2f+1 precommits for a block → commit!
\* Matches tick() Phase::Precommit when check_precommit_quorum() returns Some(Some(hash))
PrecommitQuorumCommit(v) ==
    /\ v \in Honest
    /\ height[v] <= MaxHeight
    /\ phase[v] = "Precommit"
    /\ LET h == height[v]
           r == round[v]
       IN
       /\ \E block \in NonNilBlocks :
           /\ HasQuorumFor(precommits[h][r], block)
           \* Commit the block
           /\ committed' = [committed EXCEPT ![v] = Append(@, <<h, block>>)]
           \* Advance to next height
           /\ height' = [height EXCEPT ![v] = h + 1]
           /\ round' = [round EXCEPT ![v] = 0]
           /\ phase' = [phase EXCEPT ![v] = "Propose"]
           \* Clear lock for new height
           /\ lockedBlock' = [lockedBlock EXCEPT ![v] = "Nil"]
           /\ lockedRound' = [lockedRound EXCEPT ![v] = -1]
           /\ validBlock' = [validBlock EXCEPT ![v] = "Nil"]
           /\ validRound' = [validRound EXCEPT ![v] = -1]
           /\ UNCHANGED <<prevotes, precommits, proposals, daAttested,
                           stateCommitment, equivocations, slashed>>

\* 2f+1 nil precommits → advance round (or reset at max)
\* Matches advance_round() when check_precommit_quorum() returns Some(None)
PrecommitNilAdvanceRound(v) ==
    /\ v \in Honest
    /\ height[v] <= MaxHeight
    /\ phase[v] = "Precommit"
    /\ LET h == height[v]
           r == round[v]
           nextR == IF r + 1 >= MaxRound THEN 0 ELSE r + 1
       IN
       /\ HasNilQuorum(precommits[h][r])
       /\ round' = [round EXCEPT ![v] = nextR]
       /\ phase' = [phase EXCEPT ![v] = "Propose"]
       /\ UNCHANGED <<height, lockedBlock, lockedRound, validBlock, validRound,
                       prevotes, precommits, proposals, committed, daAttested,
                       stateCommitment, equivocations, slashed>>

\* Precommit timeout → advance round (or reset at max)
\* Matches tick() precommit_timeout path + advance_round() max-round reset
PrecommitTimeout(v) ==
    /\ v \in Honest
    /\ height[v] <= MaxHeight
    /\ phase[v] = "Precommit"
    /\ LET r == round[v]
           nextR == IF r + 1 >= MaxRound THEN 0 ELSE r + 1
       IN
       /\ round' = [round EXCEPT ![v] = nextR]
       /\ phase' = [phase EXCEPT ![v] = "Propose"]
       /\ UNCHANGED <<height, lockedBlock, lockedRound, validBlock, validRound,
                       prevotes, precommits, proposals, committed, daAttested,
                       stateCommitment, equivocations, slashed>>

\* ─────────────────── Byzantine actions ──────────────────────────────────

\* Byzantine validator can send any prevote
ByzantinePrevote(v) ==
    /\ v \in Faulty
    /\ height[v] <= MaxHeight
    /\ LET h == height[v]
           r == round[v]
       IN
       \E block \in BlockValues :
           /\ prevotes' = [prevotes EXCEPT ![h][r] = @ \cup {<<v, block>>}]
           /\ UNCHANGED <<height, round, phase, lockedBlock, lockedRound, validBlock,
                           validRound, precommits, proposals, committed, daAttested,
                           stateCommitment, equivocations, slashed>>

\* Byzantine validator can send any precommit
ByzantinePrecommit(v) ==
    /\ v \in Faulty
    /\ height[v] <= MaxHeight
    /\ LET h == height[v]
           r == round[v]
       IN
       \E block \in BlockValues :
           /\ precommits' = [precommits EXCEPT ![h][r] = @ \cup {<<v, block>>}]
           /\ UNCHANGED <<height, round, phase, lockedBlock, lockedRound, validBlock,
                           validRound, prevotes, proposals, committed, daAttested,
                           stateCommitment, equivocations, slashed>>

\* ─────────────────── Equivocation detection ─────────────────────────────
\* Matches Rust: proposals_seen tracking + prevote/precommit equivocation checks

DetectEquivocation(v) ==
    /\ v \in Validators
    /\ height[v] <= MaxHeight
    /\ LET h == height[v]
           r == round[v]
       IN
       \* Check if v sent two different prevotes in the same round
       \/ (\E b1, b2 \in BlockValues :
            /\ b1 # b2
            /\ <<v, b1>> \in prevotes[h][r]
            /\ <<v, b2>> \in prevotes[h][r]
            /\ equivocations' = equivocations \cup {v}
            /\ slashed' = slashed \cup {v}
            /\ UNCHANGED <<height, round, phase, lockedBlock, lockedRound, validBlock,
                            validRound, prevotes, precommits, proposals, committed,
                            daAttested, stateCommitment>>)
       \* Check if v sent two different precommits in the same round
       \/ (\E b1, b2 \in BlockValues :
            /\ b1 # b2
            /\ <<v, b1>> \in precommits[h][r]
            /\ <<v, b2>> \in precommits[h][r]
            /\ equivocations' = equivocations \cup {v}
            /\ slashed' = slashed \cup {v}
            /\ UNCHANGED <<height, round, phase, lockedBlock, lockedRound, validBlock,
                            validRound, prevotes, precommits, proposals, committed,
                            daAttested, stateCommitment>>)

\* ─────────────────── Round skip ─────────────────────────────────────────
\* Matches Rust: on_message() round-skip logic when msg.round() > round_state.round
\*
\* D14 fix 2026-05-21: gate RoundSkip on "no block has stake quorum in
\* my current round". If a quorum exists for some block b in round r,
\* the validator MUST lock on b via PrevoteQuorumReached before skipping
\* to a later round — otherwise the validator advances unlocked, then
\* votes for a different block in the new round, and a later validator
\* completing the round-r quorum creates a retrospective quorum the
\* now-locked-elsewhere validator couldn't have observed. This is the
\* second half of the LockSafety fix (audit doc § D14); D14a gates
\* ReceiveProposalAndPrevote on no-later-round-activity, D14b tightens
\* PrevoteNilQuorumOrTimeout on no-block-has-quorum.
RoundSkip(v) ==
    /\ v \in Honest
    /\ height[v] <= MaxHeight
    /\ LET h == height[v]
           r == round[v]
       IN
       \* D14 gate: a block with stake quorum in this round must be
       \* locked on (via PrevoteQuorumReached) before skipping.
       /\ \A b \in NonNilBlocks : ~HasQuorumFor(prevotes[h][r], b)
       \* If we see votes from a future round, skip to it
       /\ \E futureR \in (r+1)..MaxRound :
           /\ \E w \in Validators, b \in BlockValues :
               \/ <<w, b>> \in prevotes[h][futureR]
               \/ <<w, b>> \in precommits[h][futureR]
           /\ round' = [round EXCEPT ![v] = futureR]
           /\ phase' = [phase EXCEPT ![v] = "Propose"]
           /\ UNCHANGED <<height, lockedBlock, lockedRound, validBlock, validRound,
                           prevotes, precommits, proposals, committed, daAttested,
                           stateCommitment, equivocations, slashed>>

\* ══════════════════════════════════════════════════════════════════════════
\* Next-state relation
\* ══════════════════════════════════════════════════════════════════════════

Next ==
    \E v \in Validators :
        \/ ProposeBlock(v)
        \/ ByzantinePropose(v)
        \/ ProposeTimeout(v)
        \/ ReceiveProposalAndPrevote(v)
        \/ PrevoteQuorumReached(v)
        \/ PrevoteNilQuorumOrTimeout(v)
        \/ PrecommitQuorumCommit(v)
        \/ PrecommitNilAdvanceRound(v)
        \/ PrecommitTimeout(v)
        \/ ByzantinePrevote(v)
        \/ ByzantinePrecommit(v)
        \/ DetectEquivocation(v)
        \/ RoundSkip(v)

Spec == Init /\ [][Next]_vars

\* ══════════════════════════════════════════════════════════════════════════
\* Safety Properties
\* ══════════════════════════════════════════════════════════════════════════

\* SAFETY 1: Agreement — no two honest validators commit different blocks
\* at the same height. This is THE critical safety property.
Agreement ==
    \A v1, v2 \in Honest :
        \A i \in 1..Len(committed[v1]) :
            \A j \in 1..Len(committed[v2]) :
                (committed[v1][i][1] = committed[v2][j][1])  \* Same height
                => (committed[v1][i][2] = committed[v2][j][2])  \* Same block

\* SAFETY 2: Validity — only proposed blocks can be committed
\* (honest validators don't fabricate blocks)
Validity ==
    \A v \in Honest :
        \A i \in 1..Len(committed[v]) :
            LET h == committed[v][i][1]
                b == committed[v][i][2]
            IN
            \E r \in 0..MaxRound :
                \E p \in Validators : <<p, b>> \in proposals[h][r]

\* SAFETY 3: No commit without quorum — a block is committed only if
\* it received 2f+1 precommits in some round
CommitRequiresQuorum ==
    \A v \in Honest :
        \A i \in 1..Len(committed[v]) :
            LET h == committed[v][i][1]
                b == committed[v][i][2]
            IN
            \E r \in 0..MaxRound : HasQuorumFor(precommits[h][r], b)

\* SAFETY 4: Lock safety — if an honest validator locks on block B in round R,
\* then no quorum of prevotes can form for a different block B' in any
\* round R' < R at the same height.
\* This is the core of Tendermint's safety argument.
LockSafety ==
    \A v \in Honest :
        lockedBlock[v] # "Nil" =>
            LET h == height[v]
                lr == lockedRound[v]
                lb == lockedBlock[v]
            IN
            \A r2 \in 0..(lr-1) :
                \A b2 \in NonNilBlocks :
                    b2 # lb => ~HasQuorumFor(prevotes[h][r2], b2)

\* SAFETY 5: Equivocating validators are always detected and slashed
EquivocationDetected ==
    \A v \in Validators :
        v \in equivocations => v \in slashed

\* SAFETY 6: BlockStateCommitment integrity — every committed block at a
\* height has a state commitment (Rule-Based Consensus guarantee)
StateCommitmentIntegrity ==
    \A v \in Honest :
        \A i \in 1..Len(committed[v]) :
            LET h == committed[v][i][1]
                b == committed[v][i][2]
            IN
            stateCommitment[h] = "Committed"

\* Combined safety invariant
SafetyInvariant ==
    /\ Agreement
    /\ Validity
    /\ CommitRequiresQuorum
    /\ LockSafety
    /\ EquivocationDetected
    /\ StateCommitmentIntegrity

\* ══════════════════════════════════════════════════════════════════════════
\* Liveness Properties (under fairness)
\* ══════════════════════════════════════════════════════════════════════════

\* LIVENESS 1: Progress — if all honest validators are at the same height
\* and can communicate, eventually one of them commits a block.
\* (Requires weak fairness on honest validator actions)
Progress ==
    \A h \in 1..MaxHeight :
        (\A v \in Honest : height[v] = h)
        ~> (\E v \in Honest : height[v] > h)

\* LIVENESS 2: DA availability — if a block is committed, eventually
\* 2f+1 validators attest to its data availability
\* (Models the DA sampling + attestation flow from perform_da_sampling())
DAEventualAttestation ==
    \A h \in 1..MaxHeight :
        (\E v \in Honest : \E i \in 1..Len(committed[v]) : committed[v][i][1] = h)
        ~> (StakeOf(daAttested[h] \cap Honest) * 3 > TotalStake * 2)  \* D1/D5: stake-weighted

\* Fairness assumption: honest validators eventually take their enabled actions
Fairness ==
    \A v \in Honest :
        /\ WF_vars(ProposeBlock(v))
        /\ WF_vars(ProposeTimeout(v))
        /\ WF_vars(ReceiveProposalAndPrevote(v))
        /\ WF_vars(PrevoteQuorumReached(v))
        /\ WF_vars(PrevoteNilQuorumOrTimeout(v))
        /\ WF_vars(PrecommitQuorumCommit(v))
        /\ WF_vars(PrecommitNilAdvanceRound(v))
        /\ WF_vars(PrecommitTimeout(v))

LiveSpec == Spec /\ Fairness

\* ══════════════════════════════════════════════════════════════════════════
\* Type invariant (for debugging / sanity)
\* ══════════════════════════════════════════════════════════════════════════

TypeOK ==
    /\ height \in [Validators -> 1..(MaxHeight + 1)]
    /\ round \in [Validators -> 0..MaxRound]
    /\ phase \in [Validators -> Phases]
    /\ lockedBlock \in [Validators -> BlockValues]
    /\ lockedRound \in [Validators -> -1..MaxRound]
    /\ validBlock \in [Validators -> BlockValues]
    /\ validRound \in [Validators -> -1..MaxRound]
    /\ equivocations \subseteq Validators
    /\ slashed \subseteq Validators

=============================================================================
