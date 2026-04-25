# EvaporChain Threat Model

## 1. System Overview

EvaporChain is a Layer 1 blockchain with thermodynamic state decay where digital
objects lose energy over time (E(t) = E₀ × 2^(−t/τ)) and eventually evaporate,
leaving only cryptographic ghost records. The system uses:

- **Tendermint-style BFT consensus** with BLS12-381 aggregate signatures
- **ML-DSA (FIPS 204)** post-quantum transaction signatures
- **Nova IVC recursive proofs** for block validity
- **Verkle trie** for state commitments
- **MMR (Merkle Mountain Range)** for evaporation nullifier accumulation
- **EvaporScript** stack-based VM for smart contracts

## 2. Trust Assumptions

### 2.1 Consensus

- **Honest majority:** ≥ 2/3 of validators by stake are honest
- **Synchrony bound:** Messages arrive within known δ after GST
- **Validator identity:** BLS public keys are correctly registered

### 2.2 Cryptography

- **ML-DSA (Dilithium3):** Module-LWE and Module-SIS are hard (NIST Level 3, ~192-bit security)
- **BLS12-381:** CDH is hard in G1 and G2 pairing groups (~128-bit security)
- **BLAKE3:** Collision and preimage resistance holds
- **Poseidon:** Algebraic attacks on width-3, 64-round Poseidon over Pallas Fp are infeasible
- **Pallas curve:** ECDLP is hard (~126-bit security)
- **Nova (HyperKZG):** Knowledge soundness under KEA assumption on BN254

### 2.3 Network

- **Authenticated channels:** libp2p with Noise protocol
- **Gossip protocol:** Messages propagate within bounded time
- **No eclipse assumption:** Validators maintain diverse peer sets

## 3. Adversary Model

### 3.1 External Adversary

**Capabilities:**
- Can observe all network traffic
- Can inject arbitrary messages into gossip
- Can create unlimited accounts and transactions
- Can control up to f < n/3 validator nodes
- Has access to quantum computers (for ML-DSA threat model)

**Cannot:**
- Break NIST-standardized cryptographic primitives
- Compromise honest validator private keys
- Violate physical network constraints

### 3.2 Malicious Validator

**Capabilities:**
- Can propose invalid blocks
- Can withhold votes or double-vote
- Can selectively include/exclude transactions
- Can collude with up to f < n/3 other validators

**Mitigated by:**
- BFT consensus requires 2/3+1 honest votes for finality
- Block validity proofs (Nova IVC) prevent invalid state transitions
- Slashing conditions for equivocation (planned)

### 3.3 Transaction-Level Adversary

**Capabilities:**
- Can submit malformed transactions
- Can attempt nonce replay
- Can attempt balance overflow/underflow
- Can deploy malicious smart contracts

## 4. Attack Surface Analysis

### 4.1 Consensus Layer

| Attack | Description | Mitigation | Status |
|--------|-------------|------------|--------|
| **Long-range attack** | Adversary forks from old state | Weak subjectivity checkpoints | Planned |
| **Nothing-at-stake** | Validators vote on multiple forks | BFT finality (single-slot) | Implemented |
| **Eclipse attack** | Isolate a node from honest peers | Peer diversity, gossip protocol | Partial |
| **Double-vote** | Validator signs conflicting blocks | Equivocation detection + slashing (10% stake) | Implemented |
| **Liveness attack** | f+1 validators go offline | Consensus halts safely (BFT guarantee) | By design |

### 4.2 Transaction Execution

| Attack | Description | Mitigation | Status |
|--------|-------------|------------|--------|
| **Nonce replay** | Re-submit old transaction | Strict nonce ordering per account | Implemented |
| **Balance overflow** | Craft tx to overflow u64 balance | Checked arithmetic, range checks | Implemented |
| **Balance underflow** | Spend more than balance | Pre-execution balance check | Implemented |
| **Self-transfer abuse** | Transfer to self for side effects | Rejected at execution | Implemented |
| **Zero-amount spam** | Flood with zero-value transfers | Rejected + gas fees | Implemented |
| **Gas exhaustion** | Submit tx exceeding block gas limit | Per-block gas limit enforcement | Implemented |
| **Signature forgery** | Forge ML-DSA signature | FIPS 204 security guarantee | Implemented |

### 4.3 State Management (Thermodynamic)

| Attack | Description | Mitigation | Status |
|--------|-------------|------------|--------|
| **Energy manipulation** | Forge energy value to prevent decay | Nova IVC range checks on decay formula | Implemented |
| **Ghost record tampering** | Modify evaporated object data | BLAKE3 data_hash commitment | Implemented |
| **Resurrection without data** | Resurrect ghost without original data | data_hash verification on resurrect | Implemented |
| **Decay formula bypass** | Skip exponential decay computation | Enforced via bit-shift in circuit (R1CS) | Implemented |
| **Grace period manipulation** | Skip or extend grace period | grace_epoch set by consensus, immutable | Implemented |
| **MMR position forgery** | Claim false MMR inclusion | MMR proof verification against root | Implemented |

### 4.4 Smart Contracts (EvaporScript)

| Attack | Description | Mitigation | Status |
|--------|-------------|------------|--------|
| **Infinite loop** | Contract loops forever | MAX_LOOP_ITERATIONS (100,000) | Implemented |
| **Stack overflow** | Push unbounded values | MAX_STACK_DEPTH (1,024) | Implemented |
| **Integer overflow** | Arithmetic wraps silently | Checked arithmetic (errors on overflow) | Implemented |
| **Gas manipulation** | Avoid gas costs | Gas deducted per opcode, 10M gas limit per call | Implemented |
| **Modulo by zero** | Division by zero crash | Explicit zero check, returns error | Implemented |
| **Memory exhaustion** | Large data allocations | Hard caps: stack 1024, strings 1MiB, maps 10K, arrays 10K, state keys 10K | Implemented |
| **Unbounded loops** | JumpIf bypasses loop limit | MAX_LOOP_ITERATIONS (100K) + MAX_STEPS (10M) on all jumps | Implemented |
| **Contract storage growth** | Unbounded state keys | MAX_STATE_KEYS (10,000) per contract | Implemented |

### 4.5 Network Layer

| Attack | Description | Mitigation | Status |
|--------|-------------|------------|--------|
| **Gossip flood** | Send oversized messages | MAX_GOSSIP_MESSAGE_SIZE (10MB) + per-peer rate limiting (500/10s) | Implemented |
| **Deserialization bomb** | Malformed data crashes node | Size check before deserialize | Implemented |
| **RwLock poisoning** | Crash thread holding lock | safe_read/safe_write recovery | Implemented |
| **Peer starvation** | Deny block sync to target | Multiple peer connections | Implemented |
| **Sync poisoning** | Accept invalid blocks during sync | BLS commit certificate verification on all synced blocks | Implemented |

### 4.6 Proving System

| Attack | Description | Mitigation | Status |
|--------|-------------|------------|--------|
| **Proof forgery** | Submit false Nova proof | HyperKZG verification (BN254) | Implemented |
| **Witness manipulation** | Lie about balance/energy witnesses | R1CS constraints + range checks | Implemented |
| **Field wraparound** | Exploit finite field arithmetic | 32-bit range checks on all values | Implemented |
| **Constraint undercount** | Missing constraint allows cheat | 11,762 constraints, conservation laws | Verified |

### 4.7 Cryptographic Primitives

| Attack | Description | Mitigation | Status |
|--------|-------------|------------|--------|
| **Quantum key recovery** | Shor's algorithm on ECDSA | ML-DSA (lattice-based, NIST L3) | Implemented |
| **BLS rogue-key** | Adversarial public key selection | Proof-of-possession with DST separation | Implemented |
| **Poseidon algebraic** | GRÖBNER basis attack on S-box | 64 rounds (8F+56P), conservative | By design |
| **Verkle binding break** | Find collision in commitments | Pallas ECDLP hardness | By design |
| **Hash collision** | BLAKE3 collision | 256-bit output, no known attacks | By design |

## 5. Data Flow Diagram

```
User → [ML-DSA Sign] → Transaction → [Gossip Network] → Validator Pool
                                                              ↓
                                                    [BFT Consensus Round]
                                                    (BLS aggregate sigs)
                                                              ↓
                                                    [Block Execution]
                                                    ├─ Transfer: balance ± checked
                                                    ├─ Create: object + energy
                                                    ├─ Refresh: energy deposit
                                                    ├─ Script: VM execution (gas-metered)
                                                    └─ Decay: E(t) = E₀ × 2^(-t/τ)
                                                              ↓
                                                    [State Commitment]
                                                    ├─ Verkle root (live state)
                                                    └─ MMR root (ghost records)
                                                              ↓
                                                    [Nova IVC Fold]
                                                    (recursive proof)
                                                              ↓
                                                    [Block Finalized]
```

## 6. Residual Risks

### 6.1 Known Gaps (Pre-Mainnet)

| Risk | Severity | Mitigation Plan |
|------|----------|-----------------|
| No slashing implementation | High | Phase 7 (mainnet genesis) |
| No weak subjectivity checkpoints | Medium | Pre-mainnet implementation |
| BLS rogue-key attack | Medium | Add proof-of-possession to validator registration |
| No formal verification of circuits | Medium | Engage audit firm for R1CS review |
| Poseidon field mismatch (Pallas vs BN256) | Low | Document; circuit uses BN256 natively |
| No encrypted mempool in production | Low | AES-GCM commit-reveal implemented, needs integration |

### 6.2 Acceptable Risks

- **f < n/3 safety bound:** Standard BFT assumption, well-understood
- **Post-quantum timeline:** ML-DSA provides protection; BLS12-381 consensus sigs
  are not post-quantum but can be upgraded via hard fork
- **32-bit range checks:** Sufficient for u64 values (energy, balance); could be
  extended to 64-bit at ~2× constraint cost if needed

## 7. Security Invariants

These properties must hold at all times:

1. **Balance conservation:** Sum of all account balances after a block equals
   sum before, minus fees burned, plus block rewards (if any)
2. **Nonce monotonicity:** Account nonces strictly increase by 1 per executed transaction
3. **Energy decay monotonicity:** Object energy never increases without a Refresh transaction
4. **Ghost integrity:** Every ghost record's data_hash matches BLAKE3(original_data)
5. **MMR append-only:** MMR leaf count never decreases; existing entries are immutable
6. **Consensus finality:** Once a block reaches 2/3+1 votes, it is never reverted
7. **Proof soundness:** No valid Nova proof exists for an invalid state transition
   (under computational assumptions)

## 8. Recommendations for External Auditors

Priority areas for review:

1. **Custom Poseidon implementation** — Round constants, S-box, MDS matrix generation
   (`crates/evaporchain-crypto/src/hash.rs`)
2. **Verkle trie commitments** — Pedersen vector commitment, proof generation/verification
   (`crates/evaporchain-crypto/src/verkle.rs`)
3. **Nova R1CS circuit** — Balance conservation, range checks, energy decay constraints
   (`crates/evaporchain-proving/src/nova.rs`)
4. **Evaporation lifecycle** — State transitions Active→Grace→Ghost, grace period enforcement
   (`crates/evaporchain-state/src/evaporation.rs`)
5. **EvaporScript VM** — Stack/loop/gas limits, checked arithmetic
   (`crates/evaporchain-script/src/vm.rs`)
