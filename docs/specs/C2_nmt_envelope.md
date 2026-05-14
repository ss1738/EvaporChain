# C2 Stage B — NMT proof envelope

**Status:** spec (no code) — pending design approval before implementation.
**Closes:** AUDIT_2026_05_13 C2 Stage B (architectural follow-up).
**Touches:** `evaporchain-da` (`namespace.rs`, `block_da_2d.rs`).
**Type:** internal refactor; no on-wire format change; no hard fork.

---

## 1. Problem statement

C2 Stage A (PR #61, merged) closed the specific bug where the NMT
verifier rewrote the Merkle hash chain from attacker-supplied
proof bytes, letting an attacker craft proofs that the verifier
reconstructed differently than the producer constructed.

What Stage A did **not** fix: the `NamespaceProof` struct is still
a flat record of public fields:

```rust
pub struct NamespaceProof {
    pub root: NmtNode,
    pub namespace: NamespaceId,
    pub start_index: usize,
    pub end_index: usize,
    pub siblings: Vec<NmtNode>,
    pub is_absence: bool,
    /* ... */
}
```

Any caller can construct a `NamespaceProof` with arbitrary fields
and pass it to a function that takes `&NamespaceProof` without
calling `verify_namespace_proof` first. Code paths that bypass
verification compile cleanly:

```rust
// All four of these compile, three are "trust the bytes":
let inclusions = collect_inclusions(&proof);  // does NOT verify
process_namespace_data(&proof);                // ditto
audit_proof(&proof);                           // ditto
let ok = verify_namespace_proof(&proof);       // THIS verifies
```

The audit's framing: an *envelope pattern* would make verified
proofs and unverified proofs distinct types, so a function that
accepts only `VerifiedNamespaceProof` is structurally protected
from being called with a raw `NamespaceProof`.

---

## 2. Goal

Lift the verified / unverified distinction into the type system:

- (a) A raw `NamespaceProof` (the on-wire / on-disk shape) is
  parseable but has no `verify` method that returns the proof
  itself — only a `Verified<NamespaceProof>` envelope which can
  only be constructed via the verifier.
- (b) Every downstream consumer that reads proof fields takes
  `&Verified<NamespaceProof>` (or its public-getter wrappers),
  not the raw struct.
- (c) The envelope's verification step recomputes the Merkle hash
  chain from leaves to root inside the constructor — there is no
  "valid envelope without the hash check happening".

(a) + (b) + (c) mean that any code path that reads
`proof.siblings` to make a decision has, by type discipline,
already proven the hash chain. The Stage A runtime check is
preserved and is now the unique gate.

---

## 3. Design

### 3.1 Core envelope

In `evaporchain-da::namespace`:

```rust
/// Raw NMT proof — what comes off the wire / off disk. Parseable
/// but NOT structurally trusted. The fields are pub because the
/// serde derive needs them, but functions that consume a proof
/// for any decision should take `&Verified<NamespaceProof>`
/// instead — the only way to obtain that is via `Verified::new`,
/// which fails if the Merkle hash chain doesn't reconstruct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceProof {
    pub root: NmtNode,
    pub namespace: NamespaceId,
    pub start_index: usize,
    pub end_index: usize,
    pub siblings: Vec<NmtNode>,
    pub is_absence: bool,
    pub blobs: Vec<NamespacedBlob>,
}

/// An NMT proof that has been verified against its embedded root.
/// The wrapper carries the same data as `NamespaceProof`; the
/// type-level distinction is the only thing standing between
/// "we got bytes" and "the bytes hash-check up to the root".
///
/// Construction is gated by `Verified::new`. There is no
/// `Default`, no `Clone` from a non-verified source, no
/// arbitrary mutable access. `Deref<Target = NamespaceProof>`
/// gives read-only access to the underlying data.
pub struct Verified<T> {
    inner: T,
    _seal: VerifiedSeal,
}

mod sealed {
    pub struct VerifiedSeal(());
    impl VerifiedSeal {
        pub(super) fn new() -> Self { Self(()) }
    }
}
use sealed::VerifiedSeal;

impl<T> std::ops::Deref for Verified<T> {
    type Target = T;
    fn deref(&self) -> &T { &self.inner }
}
```

The `VerifiedSeal` is in a private submodule so external crates
cannot construct one. Only this module's `Verified::new` can
emit a `Verified<NamespaceProof>`.

### 3.2 Verification entry point

```rust
impl Verified<NamespaceProof> {
    /// The ONLY public path to a Verified envelope. Recomputes
    /// the Merkle hash chain leaf-to-root and asserts it matches
    /// the embedded root. Stage A's audit-flag fixed this
    /// recomputation; Stage B makes it the only gate.
    pub fn new(proof: NamespaceProof) -> Result<Self, NmtVerifyError> {
        verify_namespace_proof_strict(&proof)?;
        Ok(Self { inner: proof, _seal: VerifiedSeal::new() })
    }

    /// Convenience: unwrap to the underlying proof if you
    /// need to forward it across a boundary (e.g. gossip
    /// re-broadcast). The receiver must re-verify.
    pub fn into_inner(self) -> NamespaceProof { self.inner }
}

/// Verification with full hash-chain reconstruction. Replaces
/// the structural-only checks in the pre-Stage-A code with a
/// proper Merkle proof walk.
fn verify_namespace_proof_strict(p: &NamespaceProof) -> Result<(), NmtVerifyError> {
    // 1. Range checks (today's behavior — already in
    //    `verify_namespace_proof`).
    // 2. NMT range invariants (today's behavior).
    // 3. NEW: recompute the Merkle hash chain. For each
    //    sibling in `p.siblings`, hash up the path from the
    //    blob's data_hash to the root. Compare against
    //    `p.root.hash`. If they differ → reject.
    // 4. For absence proofs: confirm the gap-existence sibling
    //    pattern via the same hash-chain check, just rooted
    //    at the in-between node instead of a single blob.
    /* ... */
    Ok(())
}
```

### 3.3 Consumer migration

| Today                                | Stage B                          |
|--------------------------------------|----------------------------------|
| `fn collect_inclusions(&NamespaceProof)` | `&Verified<NamespaceProof>` |
| `fn process_namespace_data(&NamespaceProof)` | `&Verified<NamespaceProof>` |
| `fn audit_proof(&NamespaceProof)`    | `&Verified<NamespaceProof>`      |
| Public RPC: receives raw, calls `Verified::new` at the boundary | unchanged on wire |
| Gossip handler: same | unchanged on wire |

Any function that doesn't need to verify (e.g., size-only
inspection for logging) keeps the `&NamespaceProof` signature.
The compiler tells us at call sites which functions cross from
"I have bytes" to "I act on this proof".

### 3.4 Serde compatibility

`Verified<T>` is **not** `Serialize` / `Deserialize`. The wire
format is exclusively the raw `NamespaceProof`. This is intentional:
deserializing a `Verified` would mean "trust the JSON to say it
was verified", which is the bug. Callers parse `NamespaceProof`
from bytes, then call `Verified::new(parsed)` to enter the
trusted track.

### 3.5 Relationship to Stage A

Stage A added the hash-chain reconstruction inside
`verify_namespace_proof`. Stage B:
1. Renames it to `verify_namespace_proof_strict` (explicit name
   so a future reader doesn't accidentally write a non-strict
   alternative).
2. Removes the public `verify_namespace_proof` returning `bool`
   from the API surface — callers that want "did this verify?"
   now ask "can I make a `Verified` envelope?" via `Verified::new
   (...).is_ok()`.
3. Wraps the verified result in the envelope so downstream
   consumers cannot accidentally skip the check.

---

## 4. What this does NOT solve

- **Bytes-on-wire integrity.** A malicious gossip peer can still
  ship a malformed `NamespaceProof`. `Verified::new` rejects it,
  but the bandwidth and parse time were spent. Rate-limit /
  ban-list at the network layer is the right defense (already in
  place via L3-merged work).
- **Producer-side bugs.** If the producer constructs an invalid
  proof, every verifier rejects it via `Verified::new`. That's
  the right behaviour — the producer is correctly held
  responsible. Stage B doesn't try to make producer bugs
  recoverable.
- **NMT empty-range edge cases.** Today's `verify_namespace_proof`
  has multiple special cases for empty subtrees. Stage B
  preserves them inside `verify_namespace_proof_strict` —
  doesn't change the *what*, only the *who-can-enter-the-trusted-
  track* discipline.

---

## 5. Test plan

Unit (`evaporchain-da::namespace`):
- `Verified::new` accepts a proof produced by `prove_namespace`
  + verifies the Merkle hash chain → `Ok`.
- `Verified::new` rejects a proof with tampered `root.hash` →
  `Err(NmtVerifyError::RootMismatch)`.
- `Verified::new` rejects a proof with tampered `siblings[i].
  hash` → `Err(NmtVerifyError::SiblingMismatch)`.
- `Verified::new` rejects absence proofs whose
  `start_index >= end_index` doesn't match the gap structure.
- `compile_fail` rustdoc test confirming that downstream
  consumer signatures cannot accept a raw `NamespaceProof`
  where they declared `Verified<NamespaceProof>`.

Integration (`evaporchain-da/tests/nmt_envelope_integration.rs`):
- Round-trip: build → prove → wire-serialize → parse → verify
  → consume. Each step uses the right type.
- Adversarial: ship a proof through a "malicious peer" path
  (bypass `Verified::new`) — confirm the downstream call site
  refuses to compile.

---

## 6. Implementation order

1. **Envelope type**: `Verified<T>`, sealed constructor,
   `Deref`. Tested in isolation. (~80 LOC.)
2. **Strict verification function**: lift today's
   `verify_namespace_proof` into `verify_namespace_proof_strict`
   + add the Merkle-chain walk if Stage A didn't already.
   (~200 LOC including the chain walk + tests.)
3. **Consumer migration**: walk every site that takes
   `&NamespaceProof` and decide: does it consume for a decision
   (→ require `Verified`), or just inspect for logging (→ keep
   raw)? (~150 LOC across crates.)
4. **API surface tightening**: rename + privatize the legacy
   `verify_namespace_proof` to keep one clean entry point.
   (~30 LOC.)
5. **Tests**: ~100 LOC of compile_fail + integration tests.

Estimated total: ~560 LOC. Smallest of the three Stage Bs.

---

## 7. Open questions

- (Q1) **Does Stage A already do the hash-chain reconstruction?**
  The pre-fix `verify_namespace_proof` (which is what `namespace.
  rs:355` currently shows) does range checks and sibling
  validity, but does NOT walk the chain. If Stage A's hash-chain
  walk landed elsewhere (e.g. in a separate verifier module that
  the caller is expected to invoke), Stage B's job is just the
  envelope wrapping. If it didn't, Stage B has to land the
  chain walk too — at which point the LOC estimate roughly
  doubles.
- (Q2) **Envelope shape — `Verified<T>` generic vs purpose-built
  `VerifiedNamespaceProof`?** Generic is more reusable (we'd
  apply the same pattern to commit certs, valset updates,
  attestations later). Purpose-built is simpler and produces
  cleaner error messages. The Compartment design (C5 Stage B)
  has the same Q at §7-Q2 — recommend deciding both together
  to keep the workspace coherent.
- (Q3) **Stage A interaction.** If we want this Stage B to be
  a pure refactor (no consensus change), we need to confirm
  Stage A's strict verification is wired at every consumer
  site today. A pre-merge audit of "every fn that takes
  &NamespaceProof, does the caller verify first?" is the
  empirical answer. Estimated 1 day of grep + read.
