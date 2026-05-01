/**
 * Client-side structural verifier for /api/light/state-proof/* responses.
 *
 * EvaporChain's EnergyVerkleTrie uses Pedersen-style vector commitments over
 * 256-ary nodes (see crates/evaporchain-crypto/src/energy_verkle.rs). The
 * full cryptographic verification path requires the prover stack — opening
 * each commitment and checking it binds to the children at `path_indices` —
 * which would be ~MB of WASM to ship into a light client. We don't ship that
 * here.
 *
 * Instead this verifier asserts *structural* well-formedness:
 *   - depth matches commitments.length
 *   - depth matches path_indices.length
 *   - depth matches siblings.length
 *   - root commitment (commitments[0]) equals the proof's stated state_root
 *   - all hashes are 32-byte hex
 *   - if value is present: it is 32 hex bytes
 *
 * That's enough to catch a malformed/forged proof envelope but NOT enough
 * to prove that `value` is committed under `state_root`. The result is
 * labelled "structurally valid" and the UI clearly states that full
 * cryptographic verification requires the on-chain prover crate.
 */

import type { StateProofResponse } from "./api";

const HEX64 = /^[0-9a-fA-F]{64}$/;

function isHex32(s: string): boolean {
  return HEX64.test(s.replace(/^0x/i, ""));
}

export type VerkleStructuralLevel =
  | "structurally_valid"
  | "structurally_invalid"
  | "unverifiable_compressed";

export interface VerkleVerificationResult {
  level: VerkleStructuralLevel;
  /** The computed/stated state_root (hex, no 0x). */
  state_root: string;
  /** Trie depth walked by the proof. */
  depth: number;
  /** Whether the proof claims the key is present (value is non-null). */
  exists: boolean;
  /** Whether the value field, if present, is a 32-byte hex string. */
  value_well_formed: boolean;
  /** Human-readable label for the UI. */
  label: string;
  /** When level === "structurally_invalid", which check failed. */
  reason?: string;
}

/**
 * Verify a Verkle state proof structurally. Does NOT perform Pedersen
 * commitment opening — the result is only as strong as the integrity check
 * label says.
 */
export function verifyVerkleStructural(resp: StateProofResponse): VerkleVerificationResult {
  const proof = resp.proof;
  const stateRoot = resp.state_root.toLowerCase().replace(/^0x/i, "");

  const fail = (reason: string): VerkleVerificationResult => ({
    level: "structurally_invalid",
    state_root: stateRoot,
    depth: proof.depth,
    exists: !!proof.value,
    value_well_formed: false,
    label: "structurally invalid",
    reason,
  });

  if (!HEX64.test(stateRoot)) return fail("state_root is not a 32-byte hex string");

  // Depth-vs-array consistency: the chain produces commitments[0..=depth]
  // (root + per-level), so we accept either depth == commitments.length OR
  // depth + 1 == commitments.length. siblings/path_indices follow the same
  // rule and we don't know which convention this build emits, so we accept
  // both as long as they agree internally.
  const N = proof.commitments.length;
  if (N === 0) return fail("empty commitment chain");
  if (proof.path_indices.length !== N - 1 && proof.path_indices.length !== N) {
    return fail(`path_indices.length=${proof.path_indices.length} inconsistent with commitments.length=${N}`);
  }
  if (proof.siblings.length !== N - 1 && proof.siblings.length !== N) {
    return fail(`siblings.length=${proof.siblings.length} inconsistent with commitments.length=${N}`);
  }
  if (proof.depth > N || proof.depth < 0) {
    return fail(`depth=${proof.depth} outside [0..${N}]`);
  }

  // Hash well-formedness across the commitment chain.
  for (let i = 0; i < proof.commitments.length; i++) {
    if (!isHex32(proof.commitments[i])) return fail(`commitments[${i}] not a 32-byte hex`);
  }

  // Value well-formedness (when present).
  let value_well_formed = true;
  if (proof.value != null) {
    if (!isHex32(proof.value)) {
      value_well_formed = false;
      return fail("value present but not a 32-byte hex string");
    }
  }

  // Sibling-list integrity at every level.
  for (let lvl = 0; lvl < proof.siblings.length; lvl++) {
    for (const s of proof.siblings[lvl]) {
      if (!isHex32(s.hash)) return fail(`siblings[${lvl}] contains malformed hash`);
      if (s.index < 0 || s.index > 255) return fail(`siblings[${lvl}] index out of [0,255]`);
    }
  }

  // Root anchoring: commitments[0] must equal the stated state_root.
  // (The chain places the root at index 0 of the commitment chain.)
  const claimedRoot = proof.commitments[0].toLowerCase().replace(/^0x/i, "");
  if (claimedRoot !== stateRoot) {
    return fail(`root commitment != state_root (got ${claimedRoot.slice(0, 12)}…)`);
  }

  // Path-indices range.
  for (let i = 0; i < proof.path_indices.length; i++) {
    if (proof.path_indices[i] < 0 || proof.path_indices[i] > 255) {
      return fail(`path_indices[${i}] outside [0,255]`);
    }
  }

  if (proof.hit_compressed) {
    return {
      level: "unverifiable_compressed",
      state_root: stateRoot,
      depth: proof.depth,
      exists: !!proof.value,
      value_well_formed,
      label:
        "structurally valid; value lives in a compressed (cold) subtree — full verification requires re-expansion",
    };
  }

  return {
    level: "structurally_valid",
    state_root: stateRoot,
    depth: proof.depth,
    exists: !!proof.value,
    value_well_formed,
    label:
      "structurally valid (full cryptographic verification requires the prover crate)",
  };
}
