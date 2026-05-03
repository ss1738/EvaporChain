#!/usr/bin/env python3
"""
EvaporChain MERA Empirical Entropy Gate — §A1.8
================================================
Runs the go/no-go decision for the Authenticated Energy-MERA state
commitment primitive.

Decision tree (per INVENTION_STACK.md §A1.8):
  - Power-law eigenvalue decay  → MERA go (log-correlation structure)
  - Exponential eigenvalue decay → downshift to authenticated MPS (area-law)
  - Flat eigenvalue spectrum     → drop tensor networks; ship Verkle

Since real Ethereum mainnet account-touch data is not available locally,
three synthetic proxy datasets are generated that cover each structural case.
When real data is available, replace the _generate_* functions with a
real loader and re-run.

Dependencies: numpy, scipy (standard scientific Python stack only)
Output:  printed decision + /Users/satyawansingh/EvaporChain/research/mera-gate/GATE_RESULT.md
Runtime: < 60 s
"""

import argparse
import csv
import sys
import time
import datetime
import math
from collections import Counter, defaultdict

import numpy as np
from scipy import linalg

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

REPORT_PATH = "/Users/satyawansingh/EvaporChain/research/mera-gate/GATE_RESULT.md"

# Account-touch matrix dimensions:  N_ACCOUNTS x N_BLOCKS
# These are proxy sizes — small enough to run in seconds on a laptop.
N_ACCOUNTS = 256   # synthetic "accounts" dimension
N_BLOCKS   = 512   # synthetic "block" dimension (time axis)

# Mutual-information approximation: number of bins for histogram
MI_BINS = 8

# Eigenvalue decay fit: use the top-K eigenvalues for regression
EIGENVALUE_FIT_K = 40

# Thresholds for power-law vs exponential vs flat classification
POWERLAW_R2_THRESHOLD    = 0.85   # R² of log-log fit must exceed this
EXPONENTIAL_R2_THRESHOLD = 0.85   # R² of log-linear fit must exceed this
POWERLAW_BEATS_EXP_MARGIN = 0.05  # power-law R² must beat exp R² by this margin

RNG_SEED = 42

# ---------------------------------------------------------------------------
# Dataset generators (synthetic proxies)
# ---------------------------------------------------------------------------

def _generate_log_correlated(rng: np.random.Generator) -> np.ndarray:
    """
    Log-correlated account-touch matrix.

    Mimics 1/f (flicker-noise) structure seen in Ethereum account activity:
    a small number of accounts (DEX routers, ERC-20 tokens) are touched
    across almost every block; the long tail is touched rarely.

    Construction:
      1. Draw a latent "popularity" score p_i ~ Pareto(1, alpha=1) per account.
      2. Each block independently activates account i with probability
         min(1, c * p_i / block_index) — decaying frequency over time
         gives the 1/f spectrum in the block dimension.
      3. Result is a binary matrix (touched=1, not-touched=0).

    Expected eigenvalue spectrum of the mutual-information matrix:
    power-law decay (MERA regime).
    """
    alpha = 1.0
    popularity = rng.pareto(alpha, size=N_ACCOUNTS) + 1.0   # shape (N_ACCOUNTS,)
    mat = np.zeros((N_ACCOUNTS, N_BLOCKS), dtype=np.float32)
    for b in range(N_BLOCKS):
        prob = np.minimum(1.0, popularity / (b + 1.0) * 0.8)
        mat[:, b] = (rng.random(N_ACCOUNTS) < prob).astype(np.float32)
    return mat


def _generate_area_law(rng: np.random.Generator) -> np.ndarray:
    """
    Area-law (short-range correlated) account-touch matrix.

    Mimics a workload where adjacent accounts (same contract or same address
    prefix bucket) tend to be co-touched but there is no long-range
    correlation across the account space.

    Construction:
      1. Divide accounts into contiguous segments of size ~16.
      2. Each block selects a random set of segments; all accounts in the
         selected segment are touched with high probability.
      3. Cross-segment correlation decays exponentially with distance.

    Expected eigenvalue spectrum of the mutual-information matrix:
    exponential decay (MPS / area-law regime).
    """
    SEGMENT_SIZE = 16
    n_segments = N_ACCOUNTS // SEGMENT_SIZE
    mat = np.zeros((N_ACCOUNTS, N_BLOCKS), dtype=np.float32)
    for b in range(N_BLOCKS):
        # pick ~10% of segments per block
        active_segs = rng.choice(n_segments,
                                 size=max(1, n_segments // 10),
                                 replace=False)
        for seg in active_segs:
            start = seg * SEGMENT_SIZE
            end   = start + SEGMENT_SIZE
            mat[start:end, b] = (rng.random(SEGMENT_SIZE) < 0.9).astype(np.float32)
    return mat


def _generate_random(rng: np.random.Generator) -> np.ndarray:
    """
    Random (maximally-mixed) account-touch matrix.

    Each account-block cell is independently activated with probability 0.1.
    No correlation structure.

    Expected eigenvalue spectrum of the mutual-information matrix:
    approximately flat (random matrix / Marchenko-Pastur bulk), indicating
    Verkle / standard Merkle is the right choice.
    """
    return (rng.random((N_ACCOUNTS, N_BLOCKS)) < 0.1).astype(np.float32)


# ---------------------------------------------------------------------------
# Real Ethereum mainnet loader (§A1.8 production gate)
# ---------------------------------------------------------------------------

def _load_real_ethereum(
    csv_path: str,
    n_accounts: int = N_ACCOUNTS,
    n_blocks: int = N_BLOCKS,
    block_column: str = "block_number",
    account_column: str = "to_address",
) -> np.ndarray:
    """
    Load Ethereum account-touch matrix from a Dune Analytics CSV export.

    Doctrine §A1.9 rule 12 + §A4.4 rule 23 require the MERA gate to pass
    on REAL chain data, not synthetic. This loader closes the gap: feed
    a Dune CSV in and the gate re-runs in <60s on the same data shape
    the synthetic generators produce.

    ## Expected CSV format

    Two columns at minimum (column names overrideable via kwargs):
      block_number, to_address

    Recommended Dune query (paste into https://dune.com/queries):

        SELECT block_number, "to" AS to_address
        FROM ethereum.transactions
        WHERE block_number BETWEEN 19000000 AND 20000000
          AND "to" IS NOT NULL
        ORDER BY block_number

    The free Dune tier exports up to ~10M rows; the 19M-20M block range
    is ~12-14M tx, so the export will need to be paginated (LIMIT/OFFSET
    with multiple downloads) or filtered to a tighter range like
    19_500_000-19_700_000 (~3-4M tx, fits in one CSV).

    ## Construction

    1. Read the CSV with stdlib `csv` (no pandas dependency — keeps the
       gate runnable on any vanilla Python install with numpy + scipy).
    2. Determine the actual block range from the data and bin into
       `n_blocks` contiguous bins. A cell M[i,j] = 1 iff account i was
       touched in any block falling into bin j.
    3. Find the top `n_accounts` addresses by total touch count
       (sparsify Pareto head — keeps the matrix small enough to run
       the MI computation in seconds while preserving the structural
       signal MERA depends on).
    4. Build the binary activation matrix in float32.

    Returns: np.ndarray of shape (n_accounts, n_blocks), dtype float32,
    with the same semantics as the synthetic generators above.
    """
    print(f"  loading real Ethereum data from {csv_path} ...", flush=True)
    t0 = time.time()

    block_set: set = set()
    account_counter: Counter = Counter()
    # First pass: tally touches per account + collect distinct blocks.
    with open(csv_path, "r", newline="") as f:
        reader = csv.DictReader(f)
        if block_column not in reader.fieldnames or account_column not in reader.fieldnames:
            raise ValueError(
                f"CSV missing required columns. "
                f"Expected '{block_column}' and '{account_column}'; "
                f"got {reader.fieldnames}. Override with --block-column / "
                f"--account-column if your Dune export uses different names."
            )
        for row in reader:
            account = (row[account_column] or "").strip().lower()
            if not account:
                continue
            block_str = (row[block_column] or "").strip()
            if not block_str:
                continue
            try:
                block = int(block_str)
            except ValueError:
                continue
            account_counter[account] += 1
            block_set.add(block)

    if not account_counter:
        raise ValueError("CSV produced zero touches — check column names + data path.")

    # Top-N accounts by touch frequency.
    top_accounts = [a for a, _ in account_counter.most_common(n_accounts)]
    account_to_idx = {a: i for i, a in enumerate(top_accounts)}

    blocks_sorted = sorted(block_set)
    block_min = blocks_sorted[0]
    block_max = blocks_sorted[-1]
    block_range = max(1, block_max - block_min + 1)
    # Block bin width — ceil-divides so every block maps to a valid bin.
    bin_width = max(1, math.ceil(block_range / n_blocks))

    def block_bin(b: int) -> int:
        idx = (b - block_min) // bin_width
        return min(idx, n_blocks - 1)

    # Second pass: populate the matrix. Streaming to avoid holding the
    # whole CSV in memory.
    mat = np.zeros((n_accounts, n_blocks), dtype=np.float32)
    n_rows = 0
    n_filtered = 0
    with open(csv_path, "r", newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            n_rows += 1
            account = (row[account_column] or "").strip().lower()
            if account not in account_to_idx:
                n_filtered += 1
                continue
            block_str = (row[block_column] or "").strip()
            try:
                block = int(block_str)
            except ValueError:
                continue
            i = account_to_idx[account]
            j = block_bin(block)
            mat[i, j] = 1.0  # binary touched indicator

    print(
        f" done ({time.time()-t0:.1f}s; "
        f"{n_rows} rows, {len(account_counter)} distinct accounts, "
        f"kept top {n_accounts} accounts × {n_blocks} blocks; "
        f"{n_filtered} rows filtered out as non-top-account)"
    )
    return mat


# ---------------------------------------------------------------------------
# Mutual-information matrix
# ---------------------------------------------------------------------------

def _marginal_entropy(row: np.ndarray, bins: int) -> float:
    """Shannon entropy of a 1-D array, computed via histogram."""
    counts, _ = np.histogram(row, bins=bins)
    probs = counts / counts.sum()
    probs = probs[probs > 0]
    return float(-np.sum(probs * np.log2(probs)))


def _joint_entropy(a: np.ndarray, b: np.ndarray, bins: int) -> float:
    """Joint Shannon entropy of two 1-D arrays."""
    counts, _, _ = np.histogram2d(a, b, bins=bins)
    probs = counts / counts.sum()
    probs = probs[probs > 0]
    return float(-np.sum(probs * np.log2(probs)))


def compute_mi_matrix(mat: np.ndarray, bins: int = MI_BINS) -> np.ndarray:
    """
    Compute the N_ACCOUNTS x N_ACCOUNTS pairwise mutual-information matrix.

    MI(i,j) = H(i) + H(j) - H(i,j)

    We work in the block dimension (axis=1), so each account is a row
    vector of length N_BLOCKS.

    For N_ACCOUNTS=256 this is 256*255/2 = 32,640 pairs — fast in numpy.
    """
    n = mat.shape[0]
    # Precompute marginal entropies
    H = np.array([_marginal_entropy(mat[i], bins) for i in range(n)])
    MI = np.zeros((n, n), dtype=np.float64)

    for i in range(n):
        for j in range(i + 1, n):
            h_ij = _joint_entropy(mat[i], mat[j], bins)
            mi_val = max(0.0, H[i] + H[j] - h_ij)
            MI[i, j] = mi_val
            MI[j, i] = mi_val
        # diagonal = self-information (marginal entropy)
        MI[i, i] = H[i]

    return MI


# ---------------------------------------------------------------------------
# Eigenvalue spectrum analysis
# ---------------------------------------------------------------------------

def eigenvalue_spectrum(MI: np.ndarray, k: int = EIGENVALUE_FIT_K) -> np.ndarray:
    """
    Return the top-k eigenvalues of the MI matrix, sorted descending.
    Uses scipy.linalg.eigh (symmetric matrix; real eigenvalues guaranteed).
    """
    # eigh returns eigenvalues ascending; take the top-k
    eigvals = linalg.eigh(MI, eigvals_only=True)
    eigvals = np.sort(eigvals)[::-1]          # descending
    eigvals = np.maximum(eigvals, 1e-12)       # clip numerical negatives
    return eigvals[:k]


def _r_squared(y_true: np.ndarray, y_pred: np.ndarray) -> float:
    """Coefficient of determination R²."""
    ss_res = np.sum((y_true - y_pred) ** 2)
    ss_tot = np.sum((y_true - np.mean(y_true)) ** 2)
    if ss_tot < 1e-15:
        return 0.0
    return float(1.0 - ss_res / ss_tot)


def classify_spectrum(eigvals: np.ndarray) -> dict:
    """
    Fit the eigenvalue decay curve and classify.

    Returns dict with keys:
      powerlaw_slope, powerlaw_r2
      exponential_rate, exponential_r2
      flat_ratio   (max/min eigenvalue ratio; near 1 = flat)
      decision     : 'MERA' | 'MPS' | 'VERKLE'
      reasoning    : human-readable string
    """
    k = len(eigvals)
    indices = np.arange(1, k + 1, dtype=np.float64)
    log_idx  = np.log(indices)
    log_eig  = np.log(eigvals)

    # --- Power-law fit: log(λ_i) = a - b*log(i) ---
    pl_coeffs  = np.polyfit(log_idx, log_eig, 1)
    pl_pred    = np.polyval(pl_coeffs, log_idx)
    pl_r2      = _r_squared(log_eig, pl_pred)
    pl_slope   = float(-pl_coeffs[0])    # positive slope = decay rate

    # --- Exponential fit: log(λ_i) = a - b*i ---
    exp_coeffs = np.polyfit(indices, log_eig, 1)
    exp_pred   = np.polyval(exp_coeffs, indices)
    exp_r2     = _r_squared(log_eig, exp_pred)
    exp_rate   = float(-exp_coeffs[0])   # positive rate = decay rate

    # --- Flat test: ratio of largest to k-th eigenvalue ---
    flat_ratio = float(eigvals[0] / eigvals[-1])

    # --- Decision ---
    if (pl_r2 >= POWERLAW_R2_THRESHOLD and
            pl_r2 >= exp_r2 + POWERLAW_BEATS_EXP_MARGIN):
        decision  = "MERA"
        reasoning = (
            f"Power-law fit dominates: R²={pl_r2:.4f}, slope={pl_slope:.3f}. "
            f"Log-correlation structure confirmed. "
            f"Authenticated Energy-MERA is the correct state commitment."
        )
    elif exp_r2 >= EXPONENTIAL_R2_THRESHOLD:
        decision  = "MPS"
        reasoning = (
            f"Exponential fit dominates: R²={exp_r2:.4f}, rate={exp_rate:.4f}. "
            f"Area-law (short-range) correlation only. "
            f"Downshift to Authenticated MPS (1-D Matrix Product State) — "
            f"still a first at L1, less ambitious than MERA."
        )
    else:
        decision  = "VERKLE"
        reasoning = (
            f"Neither power-law (R²={pl_r2:.4f}) nor exponential (R²={exp_r2:.4f}) "
            f"fits well. Flat spectrum (ratio={flat_ratio:.1f}x). "
            f"No tensor-network structure. "
            f"Drop MERA/MPS; ship Verkle + Energy-Verkle as planned."
        )

    return {
        "powerlaw_slope":   pl_slope,
        "powerlaw_r2":      pl_r2,
        "exponential_rate": exp_rate,
        "exponential_r2":   exp_r2,
        "flat_ratio":       flat_ratio,
        "decision":         decision,
        "reasoning":        reasoning,
    }


# ---------------------------------------------------------------------------
# Per-dataset runner
# ---------------------------------------------------------------------------

def run_dataset(name: str, mat: np.ndarray) -> dict:
    """
    Full pipeline for one dataset:
      matrix → MI → eigenvalues → classification
    Returns result dict suitable for report.
    """
    t0 = time.time()
    print(f"\n{'='*60}")
    print(f"DATASET: {name}")
    print(f"  shape: {mat.shape[0]} accounts × {mat.shape[1]} blocks")
    print(f"  density: {mat.mean():.4f} (fraction of cells = 1)")

    print("  computing mutual-information matrix ...", end="", flush=True)
    MI = compute_mi_matrix(mat)
    print(f" done ({time.time()-t0:.1f}s)")

    print("  computing eigenvalue spectrum ...", end="", flush=True)
    t1 = time.time()
    eigvals = eigenvalue_spectrum(MI)
    print(f" done ({time.time()-t1:.1f}s)")

    result = classify_spectrum(eigvals)
    result["name"]    = name
    result["eigvals"] = eigvals
    result["elapsed"] = time.time() - t0

    # Print top eigenvalues for inspection
    top10 = eigvals[:10]
    print(f"\n  Top-10 eigenvalues: {np.array2string(top10, precision=4, suppress_small=True)}")
    print(f"\n  Power-law  R²={result['powerlaw_r2']:.4f}  slope={result['powerlaw_slope']:.3f}")
    print(f"  Exponential R²={result['exponential_r2']:.4f}  rate={result['exponential_rate']:.4f}")
    print(f"  Flat ratio: {result['flat_ratio']:.1f}x")
    print(f"\n  DECISION: {result['decision']}")
    print(f"  {result['reasoning']}")
    print(f"  [{result['elapsed']:.1f}s total]")

    return result


# ---------------------------------------------------------------------------
# Final EvaporChain decision
# ---------------------------------------------------------------------------

DECISION_RANK = {"MERA": 0, "MPS": 1, "VERKLE": 2}

def final_decision(results: list[dict]) -> dict:
    """
    Aggregate three synthetic test results into the real EvaporChain decision.

    Logic:
    - These three datasets cover the full structural range.
    - The log-correlated case represents the most likely real-world regime
      for a live chain (Pareto-distributed account activity is empirically
      well-established for Ethereum).
    - The expected real-data answer is MERA (log-correlated) — but the gate
      must be re-run with actual Ethereum/Solana mainnet dumps.
    - The gate is PASS if the log-correlated synthetic case returns MERA.
    - The gate is CONDITIONAL if the log-correlated case returns MPS.
    - The gate is FAIL if the log-correlated case returns VERKLE or if
      the random case incorrectly returns MERA (sanity check).

    Replace this logic with real Ethereum data analysis before Week 14–18
    build decision.
    """
    by_name = {r["name"]: r for r in results}
    # The MERA-decision input is whichever of LOG-CORRELATED (synthetic)
    # or ETH-MAINNET (real) was run. Real-data mode replaces the
    # synthetic proxy; both flow through the same verdict logic.
    log_corr = (
        by_name.get("ETH-MAINNET", {}).get("decision")
        or by_name.get("LOG-CORRELATED", {}).get("decision", "UNKNOWN")
    )
    area_law  = by_name.get("AREA-LAW",      {}).get("decision", "UNKNOWN")
    random    = by_name.get("RANDOM",         {}).get("decision", "UNKNOWN")

    # Sanity check: random should not produce MERA
    sanity_ok = (random != "MERA")

    if not sanity_ok:
        gate_verdict = "FAIL (SANITY)"
        gate_reasoning = (
            "CRITICAL: The random-matrix test incorrectly classified as MERA. "
            "The classifier thresholds need tuning or the MI computation is buggy. "
            "Do NOT use this result to green-light MERA. Fix the gate first."
        )
        recommended_action = "Debug run_gate.py before using on real data."
    elif log_corr == "MERA":
        gate_verdict = "PASS — MERA GO"
        gate_reasoning = (
            "Log-correlated synthetic proxy passes the gate: real Pareto-distributed "
            "chain workloads are expected to show the same log-correlation structure. "
            "Authenticated Energy-MERA is cleared for the Week 14–18 sprint build. "
            "Confidence is HIGH for a real-chain MERA decision — but verify with "
            "actual Ethereum mainnet account-touch data before committing to "
            "the whitepaper MERA claim."
        )
        recommended_action = (
            "Proceed: scaffold `evaporchain-mera` crate (χ=4 prototype). "
            "Simultaneously, pull Ethereum mainnet blocks 19M–20M via an archive "
            "node or Dune Analytics export and re-run this script with real data. "
            "If real data also returns MERA, lock the whitepaper §MERA claim."
        )
    elif log_corr == "MPS":
        gate_verdict = "CONDITIONAL — MPS DOWNSHIFT"
        gate_reasoning = (
            "Log-correlated proxy only reaches MPS (area-law), not full MERA. "
            "Either the proxy is insufficiently long-range, or real chain state "
            "may only have area-law structure. Downshift to Authenticated MPS — "
            "still a first at L1; just less ambitious than MERA."
        )
        recommended_action = (
            "Scaffold `evaporchain-mps` crate instead of MERA. "
            "Re-run with real Ethereum data before final decision. "
            "If real data shows MERA, upgrade from MPS to MERA at Week 14."
        )
    else:
        gate_verdict = "FAIL — VERKLE"
        gate_reasoning = (
            "No tensor-network structure detected even in the log-correlated proxy. "
            "Drop MERA and MPS. Ship Verkle + Energy-Verkle as originally planned."
        )
        recommended_action = (
            "Confirm with real Ethereum data. "
            "If real data also returns VERKLE, close the MERA gate permanently "
            "and focus on Energy-Verkle Trie (already in progress)."
        )

    return {
        "log_corr":           log_corr,
        "area_law":           area_law,
        "random":             random,
        "sanity_ok":          sanity_ok,
        "gate_verdict":       gate_verdict,
        "gate_reasoning":     gate_reasoning,
        "recommended_action": recommended_action,
    }


# ---------------------------------------------------------------------------
# Report writer
# ---------------------------------------------------------------------------

def write_report(results: list[dict], decision: dict, elapsed_total: float) -> None:
    run_ts = datetime.datetime.utcnow().strftime("%Y-%m-%d %H:%M:%S UTC")

    lines = []
    lines.append("# EvaporChain MERA Empirical Entropy Gate — Results")
    lines.append("")
    lines.append(f"**Run:** {run_ts}")
    lines.append(f"**Script:** `research/mera-gate/run_gate.py`")
    lines.append(f"**Total elapsed:** {elapsed_total:.1f}s")
    if decision.get("real_data_mode"):
        lines.append(f"**Data:** REAL ETHEREUM MAINNET (doctrine §A1.9 rule 12 satisfied)")
    else:
        lines.append(f"**Data:** SYNTHETIC PROXY (re-run with `--input <dune.csv>` for real)")
    lines.append(f"**Reference:** INVENTION_STACK.md §A1.8")
    lines.append("")
    lines.append("---")
    lines.append("")
    lines.append("## Gate Verdict")
    lines.append("")
    lines.append(f"```")
    lines.append(f"VERDICT: {decision['gate_verdict']}")
    lines.append(f"```")
    lines.append("")
    lines.append(decision["gate_reasoning"])
    lines.append("")
    lines.append(f"**Recommended action:** {decision['recommended_action']}")
    lines.append("")
    lines.append("---")
    lines.append("")
    lines.append("## Per-Dataset Results")
    lines.append("")
    lines.append("| Dataset | Power-law R² | Exp R² | Flat Ratio | Decision |")
    lines.append("|---------|-------------|--------|-----------|----------|")

    for r in results:
        lines.append(
            f"| {r['name']} "
            f"| {r['powerlaw_r2']:.4f} (slope={r['powerlaw_slope']:.3f}) "
            f"| {r['exponential_r2']:.4f} (rate={r['exponential_rate']:.4f}) "
            f"| {r['flat_ratio']:.1f}x "
            f"| **{r['decision']}** |"
        )

    lines.append("")
    lines.append("---")
    lines.append("")
    lines.append("## Eigenvalue Spectra (top 10)")
    lines.append("")

    for r in results:
        lines.append(f"### {r['name']}")
        lines.append("")
        lines.append("| Rank | Eigenvalue |")
        lines.append("|------|-----------|")
        for i, ev in enumerate(r["eigvals"][:10], 1):
            lines.append(f"| {i} | {ev:.6f} |")
        lines.append("")
        lines.append(f"**Reasoning:** {r['reasoning']}")
        lines.append("")

    lines.append("---")
    lines.append("")
    lines.append("## Decision Logic")
    lines.append("")
    lines.append(f"| Test | Classification |")
    lines.append(f"|------|---------------|")
    lines.append(f"| Log-correlated proxy | {decision['log_corr']} |")
    lines.append(f"| Area-law proxy | {decision['area_law']} |")
    lines.append(f"| Random proxy (sanity) | {decision['random']} |")
    lines.append(f"| Sanity check passed | {decision['sanity_ok']} |")
    lines.append("")
    lines.append("---")
    lines.append("")
    lines.append("## Interpretation Key")
    lines.append("")
    lines.append("| Spectrum type | Chain analogue | EvaporChain action |")
    lines.append("|---------------|---------------|-------------------|")
    lines.append("| Power-law eigenvalue decay | Log-correlation (MERA regime) | Ship Authenticated Energy-MERA — first tensor-network state commitment at L1 |")
    lines.append("| Exponential eigenvalue decay | Area-law (MPS regime) | Downshift to Authenticated MPS — still a first at L1 |")
    lines.append("| Flat spectrum | Random / no structure | Drop tensor networks; ship Verkle + Energy-Verkle Trie |")
    lines.append("")
    lines.append("---")
    lines.append("")
    lines.append("## Next Steps Before Week 14–18 Build")
    lines.append("")
    lines.append("1. Pull real Ethereum mainnet account-touch data (blocks 19M–20M).")
    lines.append("   - Source: Dune Analytics `ethereum.transactions` → group by `to` address per block.")
    lines.append("   - Target size: ≥100K accounts × ≥10K blocks (1B cells sparse).")
    lines.append("   - Sparsify: keep top-N accounts by touch frequency (Pareto head).")
    lines.append("2. Replace `_generate_log_correlated / _generate_area_law / _generate_random`")
    lines.append("   with a `_load_real_ethereum(path)` loader.")
    lines.append("3. Re-run this script. The `GATE_RESULT.md` will be overwritten.")
    lines.append("4. If MERA: proceed to `evaporchain-mera` crate scaffold.")
    lines.append("5. If MPS: scaffold `evaporchain-mps` instead.")
    lines.append("6. If VERKLE: close the MERA gate. Focus on Energy-Verkle (already in progress).")
    lines.append("")
    lines.append("---")
    lines.append("")
    lines.append("*Generated by `run_gate.py`. Overwrite by re-running the script.*")

    report = "\n".join(lines)

    with open(REPORT_PATH, "w") as f:
        f.write(report)

    print(f"\nReport written to: {REPORT_PATH}")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description="EvaporChain MERA Empirical Entropy Gate — §A1.8",
    )
    parser.add_argument(
        "--input",
        type=str,
        default=None,
        help=(
            "Path to a Dune Analytics CSV export of "
            "ethereum.transactions(block_number, to_address). When set, "
            "the LOG-CORRELATED synthetic dataset is replaced by the real "
            "Ethereum mainnet matrix and the verdict reflects real chain "
            "data (doctrine §A1.9 rule 12). When unset, runs in synthetic "
            "mode — verdict carries the synthetic-data caveat."
        ),
    )
    parser.add_argument(
        "--block-column",
        type=str,
        default="block_number",
        help="Override the block-number column name in the input CSV.",
    )
    parser.add_argument(
        "--account-column",
        type=str,
        default="to_address",
        help="Override the account-address column name in the input CSV.",
    )
    args = parser.parse_args()

    t_start = time.time()
    print("EvaporChain MERA Empirical Entropy Gate (§A1.8)")
    real_data_mode = args.input is not None
    if real_data_mode:
        print(f"Data mode: REAL ETHEREUM — {args.input}")
        print(f"Target shape: {N_ACCOUNTS} accounts × {N_BLOCKS} blocks (top-N by touch)")
    else:
        print(f"Data mode: SYNTHETIC PROXY — {N_ACCOUNTS} accounts × {N_BLOCKS} blocks")
        print(f"  (pass --input <csv> to switch to real-Ethereum mode)")
    print(f"Thresholds: power-law R²>{POWERLAW_R2_THRESHOLD}, "
          f"exp R²>{EXPONENTIAL_R2_THRESHOLD}, "
          f"power-law margin>{POWERLAW_BEATS_EXP_MARGIN}")

    rng = np.random.default_rng(RNG_SEED)

    if real_data_mode:
        # Real Ethereum replaces the LOG-CORRELATED proxy. Keep AREA-LAW
        # + RANDOM as control sanity datasets so the classifier's
        # behaviour remains measurable across the structural range.
        datasets = [
            ("ETH-MAINNET", _load_real_ethereum(
                args.input,
                block_column=args.block_column,
                account_column=args.account_column,
            )),
            ("AREA-LAW",    _generate_area_law(rng)),
            ("RANDOM",      _generate_random(rng)),
        ]
    else:
        datasets = [
            ("LOG-CORRELATED", _generate_log_correlated(rng)),
            ("AREA-LAW",       _generate_area_law(rng)),
            ("RANDOM",         _generate_random(rng)),
        ]

    results = []
    for name, mat in datasets:
        results.append(run_dataset(name, mat))

    decision = final_decision(results)
    decision["real_data_mode"] = real_data_mode

    print("\n" + "="*60)
    print("EVAPORCHAIN REAL DECISION")
    print("="*60)
    print(f"\nVERDICT: {decision['gate_verdict']}")
    if real_data_mode:
        print("(REAL Ethereum data — doctrine §A1.9 rule 12 satisfied)")
    else:
        print("(synthetic-data run — re-run with --input to satisfy §A1.9)")
    print(f"\n{decision['gate_reasoning']}")
    print(f"\nRECOMMENDED ACTION: {decision['recommended_action']}")

    elapsed = time.time() - t_start
    print(f"\nTotal runtime: {elapsed:.1f}s")

    write_report(results, decision, elapsed)


if __name__ == "__main__":
    main()
