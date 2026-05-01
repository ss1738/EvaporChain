/**
 * Client-side verifier for /api/light/tx-proof/:block/:tx_index responses.
 *
 * Mirrors `evaporchain-node::persistence::verify_tx_inclusion`:
 *   current = tx_hash (32-byte blake3 of the canonical tx encoding)
 *   for sibling in proof.siblings:
 *     if sibling.position == "left":  current = blake3(sibling || current)
 *     else /* "right" */              /*  current = blake3(current || sibling) */
 *   /* assert hex(current) == proof.merkle_root */
 *
 * The leaf hash itself is recomputed by the node from the typed Transaction
 * value, which we can't reconstruct in-browser without the full block payload.
 * So this verifier checks consistency of the supplied path against the
 * supplied leaf — a "path-from-leaf" attestation. To anchor the result at the
 * trusted header, callers should additionally compare proof.merkle_root
 * against the corresponding compact header's tx_merkle_root field.
 */

import { blake3 } from "@noble/hashes/blake3";
import type { TxInclusionProof } from "./api";

function hexToBytes(hex: string): Uint8Array | null {
  const s = hex.replace(/^0x/i, "").toLowerCase();
  if (s.length !== 64 || /[^0-9a-f]/.test(s)) return null;
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) out[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16);
  return out;
}

function bytesToHex(b: Uint8Array): string {
  let s = "";
  for (let i = 0; i < b.length; i++) s += b[i].toString(16).padStart(2, "0");
  return s;
}

export interface MerkleVerificationResult {
  /** Whether the recomputed root matches proof.merkle_root. */
  valid: boolean;
  /** The recomputed root (hex). */
  computed_root: string;
  /** The expected root (hex) from the proof envelope. */
  expected_root: string;
  /** Whether proof.merkle_root also equals the trusted header's tx_merkle_root, if supplied. */
  matches_header_root?: boolean;
  /** Human-readable failure cause when valid===false. */
  reason?: string;
}

/**
 * Recompute a Merkle root from `proof.tx_hash` walking up via `proof.siblings`.
 *
 * If `headerTxMerkleRoot` is supplied, also asserts that the proof's
 * declared `merkle_root` matches the trusted header — this is what makes the
 * verification "light": no trust in the indexer that built the proof.
 */
export function verifyTxInclusion(
  proof: TxInclusionProof,
  headerTxMerkleRoot?: string,
): MerkleVerificationResult {
  const leaf = hexToBytes(proof.tx_hash);
  if (!leaf) {
    return {
      valid: false,
      computed_root: "",
      expected_root: proof.merkle_root,
      reason: "tx_hash is not a valid 32-byte hex string",
    };
  }

  let current = leaf;
  for (const sib of proof.siblings) {
    const sibling = hexToBytes(sib.hash);
    if (!sibling) {
      return {
        valid: false,
        computed_root: "",
        expected_root: proof.merkle_root,
        reason: `sibling hash malformed: ${sib.hash}`,
      };
    }
    const concat = new Uint8Array(64);
    if (sib.position === "left") {
      concat.set(sibling, 0);
      concat.set(current, 32);
    } else if (sib.position === "right") {
      concat.set(current, 0);
      concat.set(sibling, 32);
    } else {
      return {
        valid: false,
        computed_root: "",
        expected_root: proof.merkle_root,
        reason: `unknown sibling position "${sib.position}"`,
      };
    }
    current = blake3(concat);
  }

  const computed_root = bytesToHex(current);
  const expected_root = proof.merkle_root.toLowerCase().replace(/^0x/i, "");
  const valid = computed_root === expected_root;

  let matches_header_root: boolean | undefined;
  if (headerTxMerkleRoot) {
    const norm = headerTxMerkleRoot.toLowerCase().replace(/^0x/i, "");
    matches_header_root = norm === expected_root;
  }

  return {
    valid,
    computed_root,
    expected_root,
    matches_header_root,
    reason: valid ? undefined : "recomputed root != proof.merkle_root",
  };
}
