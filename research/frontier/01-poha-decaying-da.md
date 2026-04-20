# Primitive 1: Proof-of-Historical-Availability (PoHA)

## Problem

Every DA layer (Celestia, EigenDA, Avail, EIP-4844 blobs) assumes data should be available indefinitely or pruned after a fixed TTL. No system models data availability as a continuous, decaying resource.

EvaporChain's core insight — state has energy that decays — should extend to DA itself. An earthquake reading from 6 hours ago is less important than one from 10 seconds ago. The DA guarantees should reflect this.

## What Exists

- **Celestia:** 2D Reed-Solomon + DAS. Data assumed permanent (or pruned by full nodes after weeks).
- **EIP-4844:** Blobs pruned after ~18 days. Blunt TTL, no gradient.
- **EvaporChain (current):** `evaporation_da.rs` proves data WAS available before evaporation. This is already novel but not formalized as a standalone primitive.

## The Idea

DA certificates carry energy and half-life, just like on-chain objects.

### Mechanism

1. When a block is produced, DA shards are erasure-coded and distributed (existing).
2. Validators who successfully sample shards produce a `DAAttestation` (existing).
3. **New:** The resulting `DACertificate` is assigned initial energy `E_0` and half-life `tau`.
4. **New:** Each epoch, a random subset of validators re-sample a random subset of DA certificates. Certificates that receive fresh attestations get an energy boost. Certificates nobody samples cool naturally.
5. **New:** The certificate lifecycle mirrors objects: Active -> Grace -> Ghost -> Evaporated.
   - **Active (hot):** Full shards available, recent attestations, any peer can reconstruct.
   - **Grace (warm):** Shards may be partially pruned, but enough exist for reconstruction.
   - **Ghost (cold):** Only the commitment root + hash survives in the MMR. Shards pruned.
   - **Evaporated:** Certificate hash in MMR only. Data is gone.

### What This Enables

- Light clients can assess data "freshness" — a hot certificate means the data is definitely available right now.
- Storage nodes naturally shed old data without explicit garbage collection.
- The chain self-regulates storage: high-traffic data stays warm, abandoned data cools and evaporates.
- Bridges and rollups can set minimum DA temperature thresholds for accepting proofs.

## Cryptographic Details

- DA certificate: `(block_number, commitment_root, shard_count, energy, half_life, bls_aggregate_sig, attestor_bitmap)`
- Re-attestation: validators sign `(cert_hash, current_epoch)` — lightweight, no re-sampling of actual data required for warm certificates
- Energy boost on re-attestation: `E_new = E_current + delta` (capped at E_0)
- Pruning rule: nodes MAY prune shards for certificates with energy < threshold

## Existing Foundation

- `crates/evaporchain-da/certificate.rs` — `DAAttestation`, `DACertificate`, `CertificateBuilder`
- `crates/evaporchain-da/evaporation_da.rs` — proves data was available before evaporation
- `crates/evaporchain-consensus/src/tendermint.rs` — `make_da_attestation()`, `try_build_da_certificate()`

## Build Plan

1. Add energy/half_life fields to `DACertificate`
2. Implement certificate decay in the evaporation engine
3. Add re-attestation protocol (random sampling of existing certificates each epoch)
4. Implement certificate lifecycle (active -> grace -> ghost -> evaporated)
5. Modify DA storage to prune shards based on certificate energy
6. Add API endpoints for certificate temperature queries

## Difficulty

3-4 months. Medium risk. The sampling protocol needs careful security analysis to prevent "attestation freeloading" (validators claiming to have data they don't).

## Publication Potential

High. "Data availability as a thermodynamic resource" is a novel framing. No prior work. Target: ACM CCS or USENIX Security as a systems paper.
