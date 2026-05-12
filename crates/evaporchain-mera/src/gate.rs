//! MERA empirical entropy gate (§A1.8) — Rust port of
//! `research/mera-gate/run_gate.py`.
//!
//! Pipeline:
//!   activation matrix (rows = accounts, cols = blocks)
//!     → pairwise mutual-information matrix (symmetric, N×N)
//!     → top-K eigenvalues via power iteration with deflation
//!     → log-log fit (power-law) and log-linear fit (exponential)
//!     → classification: `Mera | Mps | Verkle`
//!
//! Pure-Rust, zero numeric deps. The classifier matches the Python script's
//! decision rule with the same R² thresholds and margin, so a CI run here is
//! a faithful replay of the gate.
//!
//! Designed for `n_accounts ≤ ~512`. Beyond that, the O(N²) MI loop and
//! O(K·N²) eigensolver dominate; swap to a Lanczos solver and consider
//! lifting MI to a parallel iter.

/// R² threshold for the power-law fit before MERA can be declared.
pub const POWERLAW_R2_THRESHOLD: f64 = 0.85;
/// R² threshold for the exponential fit before MPS can be declared.
pub const EXPONENTIAL_R2_THRESHOLD: f64 = 0.85;
/// Power-law R² must beat exponential R² by this margin to claim MERA.
pub const POWERLAW_BEATS_EXP_MARGIN: f64 = 0.05;
/// Default top-K eigenvalues to use for the regression fits.
pub const EIGENVALUE_FIT_K: usize = 40;
/// Default histogram bin count for the MI computation.
pub const MI_BINS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    /// Power-law spectrum dominates → use authenticated MERA.
    Mera,
    /// Exponential spectrum dominates → downshift to authenticated MPS.
    Mps,
    /// Flat / no structure → ship Verkle, drop tensor networks.
    Verkle,
}

#[derive(Debug, Clone)]
pub struct GateResult {
    pub decision: GateDecision,
    /// Slope of the power-law fit (log-log space). Larger = faster decay.
    pub powerlaw_slope: f64,
    pub powerlaw_r2: f64,
    /// Decay rate of the exponential fit (log-linear space).
    pub exponential_rate: f64,
    pub exponential_r2: f64,
    /// `eigvals[0] / eigvals[k-1]` — flatness sanity check.
    pub flat_ratio: f64,
    /// Top-K eigenvalues, descending. Empty if the spectrum was degenerate.
    pub eigvals: Vec<f64>,
    /// Human-readable reason matching the Python gate's output style.
    pub reasoning: String,
}

// ─────────────────────────── Mutual information ────────────────────────────

fn marginal_entropy(row: &[f64], bins: usize) -> f64 {
    let (lo, hi) = row
        .iter()
        .copied()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
            (a.min(x), b.max(x))
        });
    if !(lo.is_finite() && hi.is_finite()) || hi <= lo {
        return 0.0; // constant row → zero entropy
    }
    let mut counts = vec![0usize; bins];
    let span = hi - lo;
    for &x in row {
        let mut idx = (((x - lo) / span) * bins as f64) as usize;
        if idx >= bins {
            idx = bins - 1;
        }
        counts[idx] += 1;
    }
    let total: f64 = counts.iter().sum::<usize>() as f64;
    if total <= 0.0 {
        return 0.0;
    }
    counts
        .into_iter()
        .filter_map(|c| {
            if c == 0 {
                None
            } else {
                let p = c as f64 / total;
                Some(-p * p.log2())
            }
        })
        .sum()
}

fn joint_entropy(a: &[f64], b: &[f64], bins: usize) -> f64 {
    debug_assert_eq!(a.len(), b.len());
    let (a_lo, a_hi) = a
        .iter()
        .copied()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(p, q), x| {
            (p.min(x), q.max(x))
        });
    let (b_lo, b_hi) = b
        .iter()
        .copied()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(p, q), x| {
            (p.min(x), q.max(x))
        });
    if !(a_hi > a_lo && b_hi > b_lo) {
        return 0.0;
    }
    let a_span = a_hi - a_lo;
    let b_span = b_hi - b_lo;
    let mut joint = vec![0usize; bins * bins];
    for (x, y) in a.iter().zip(b.iter()) {
        let mut ix = (((x - a_lo) / a_span) * bins as f64) as usize;
        if ix >= bins {
            ix = bins - 1;
        }
        let mut iy = (((y - b_lo) / b_span) * bins as f64) as usize;
        if iy >= bins {
            iy = bins - 1;
        }
        joint[ix * bins + iy] += 1;
    }
    let total: f64 = joint.iter().sum::<usize>() as f64;
    if total <= 0.0 {
        return 0.0;
    }
    joint
        .into_iter()
        .filter_map(|c| {
            if c == 0 {
                None
            } else {
                let p = c as f64 / total;
                Some(-p * p.log2())
            }
        })
        .sum()
}

/// Pairwise mutual-information matrix over the rows of `mat`.
/// `MI[i][j] = H(i) + H(j) − H(i, j)`, clamped to ≥ 0 to absorb histogram noise.
/// Diagonal holds each row's marginal entropy.
pub fn compute_mi_matrix(mat: &[Vec<f64>], bins: usize) -> Vec<Vec<f64>> {
    let n = mat.len();
    let h: Vec<f64> = (0..n).map(|i| marginal_entropy(&mat[i], bins)).collect();
    let mut mi = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        mi[i][i] = h[i];
        for j in (i + 1)..n {
            let h_ij = joint_entropy(&mat[i], &mat[j], bins);
            let m = (h[i] + h[j] - h_ij).max(0.0);
            mi[i][j] = m;
            mi[j][i] = m;
        }
    }
    mi
}

// ────────────────────────── Top-K eigenvalues ──────────────────────────────

/// Multiply a symmetric N×N matrix by a length-N vector. Pure scalar loops —
/// cheap enough for N ≤ 512.
fn matvec(m: &[Vec<f64>], v: &[f64]) -> Vec<f64> {
    let n = m.len();
    let mut out = vec![0.0f64; n];
    for i in 0..n {
        let row = &m[i];
        let mut acc = 0.0;
        for j in 0..n {
            acc += row[j] * v[j];
        }
        out[i] = acc;
    }
    out
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn norm(v: &[f64]) -> f64 {
    dot(v, v).sqrt()
}

fn normalize(v: &mut [f64]) -> f64 {
    let n = norm(v);
    if n > 0.0 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
    n
}

/// Top-K eigenvalues (descending) of a symmetric matrix via power iteration
/// with rank-one deflation. Deterministic given a seed; the seed feeds the
/// initial vector for each eigenpair so a re-run is bit-identical.
///
/// `max_iters_per_eigval` caps the inner loop so a near-degenerate spectrum
/// can't run unboundedly. Convergence tolerance is `1e-9` on successive
/// Rayleigh quotients.
pub fn top_k_eigenvalues(matrix: &[Vec<f64>], k: usize, seed: u64) -> Vec<f64> {
    let n = matrix.len();
    if n == 0 || k == 0 {
        return Vec::new();
    }
    let k = k.min(n);
    // Working copy — we'll deflate it as we extract each eigenpair.
    let mut m: Vec<Vec<f64>> = matrix.to_vec();
    let mut eigvals = Vec::with_capacity(k);
    let mut rng = crate::synthetic::Rng::new(seed);

    for _ in 0..k {
        // Random unit-norm initial vector.
        let mut v: Vec<f64> = (0..n).map(|_| rng.next_f64() - 0.5).collect();
        if normalize(&mut v) == 0.0 {
            v[0] = 1.0;
        }

        let mut prev_lambda = 0.0;
        let max_iters = 250;
        let mut lambda = 0.0;
        for it in 0..max_iters {
            let av = matvec(&m, &v);
            // Rayleigh quotient = v^T A v (v is unit-norm).
            lambda = dot(&v, &av);
            v = av;
            if normalize(&mut v) == 0.0 {
                break; // null space — give up on this eigenpair
            }
            if it > 0 && (lambda - prev_lambda).abs() < 1e-9 * (lambda.abs() + 1e-12) {
                break;
            }
            prev_lambda = lambda;
        }

        // Power iteration of a symmetric matrix may converge to the
        // *largest-magnitude* eigenvalue, which can be negative. The MI
        // matrix is positive semi-definite in theory, but histogram noise
        // can produce small negative eigenvalues. Clamp them.
        if !lambda.is_finite() {
            break;
        }
        if lambda < 1e-12 {
            // Either we've run out of positive spectrum or the matrix is
            // already deflated below noise floor — stop early.
            eigvals.push(lambda.max(0.0));
            break;
        }
        eigvals.push(lambda);

        // Rank-one deflation: A ← A − λ·v·vᵀ. After this, v is no longer
        // an eigenvector of A (eigenvalue removed) so the next iteration
        // picks up the next-largest eigenvalue.
        for i in 0..n {
            let vi = v[i];
            let row = &mut m[i];
            for j in 0..n {
                row[j] -= lambda * vi * v[j];
            }
        }
    }

    // Sort descending; rare power-iteration ordering glitches show up here.
    eigvals.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    eigvals
}

// ─────────────────────────── Regression fits ───────────────────────────────

fn linear_regression(xs: &[f64], ys: &[f64]) -> (f64, f64) {
    debug_assert_eq!(xs.len(), ys.len());
    let n = xs.len() as f64;
    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = ys.iter().sum::<f64>() / n;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    for (x, y) in xs.iter().zip(ys.iter()) {
        let dx = x - mean_x;
        sxy += dx * (y - mean_y);
        sxx += dx * dx;
    }
    let slope = if sxx == 0.0 { 0.0 } else { sxy / sxx };
    let intercept = mean_y - slope * mean_x;
    (slope, intercept)
}

fn r_squared(ys: &[f64], preds: &[f64]) -> f64 {
    debug_assert_eq!(ys.len(), preds.len());
    let mean = ys.iter().sum::<f64>() / ys.len() as f64;
    let ss_res: f64 = ys
        .iter()
        .zip(preds.iter())
        .map(|(y, p)| (y - p).powi(2))
        .sum();
    let ss_tot: f64 = ys.iter().map(|y| (y - mean).powi(2)).sum();
    if ss_tot < 1e-15 {
        return 0.0;
    }
    1.0 - ss_res / ss_tot
}

// ─────────────────────────── Classification ────────────────────────────────

/// Fit power-law and exponential decay curves to `eigvals` and classify.
pub fn classify_spectrum(eigvals: &[f64]) -> GateResult {
    if eigvals.is_empty() {
        return GateResult {
            decision: GateDecision::Verkle,
            powerlaw_slope: 0.0,
            powerlaw_r2: 0.0,
            exponential_rate: 0.0,
            exponential_r2: 0.0,
            flat_ratio: 1.0,
            eigvals: Vec::new(),
            reasoning: "empty eigenvalue spectrum — defaulting to Verkle".to_string(),
        };
    }
    let k = eigvals.len();
    // Floor at 1e-12 to keep log() finite under noise.
    let log_eig: Vec<f64> = eigvals.iter().map(|e| e.max(1e-12).ln()).collect();
    let indices: Vec<f64> = (1..=k).map(|i| i as f64).collect();
    let log_idx: Vec<f64> = indices.iter().map(|i| i.ln()).collect();

    // Power-law fit: log(λ_i) = a − b·log(i). Slope = decay rate.
    let (pl_slope_raw, pl_intercept) = linear_regression(&log_idx, &log_eig);
    let pl_pred: Vec<f64> = log_idx
        .iter()
        .map(|x| pl_intercept + pl_slope_raw * x)
        .collect();
    let pl_r2 = r_squared(&log_eig, &pl_pred);
    let pl_slope = -pl_slope_raw;

    // Exponential fit: log(λ_i) = a − b·i. Rate = decay rate.
    let (exp_slope_raw, exp_intercept) = linear_regression(&indices, &log_eig);
    let exp_pred: Vec<f64> = indices
        .iter()
        .map(|x| exp_intercept + exp_slope_raw * x)
        .collect();
    let exp_r2 = r_squared(&log_eig, &exp_pred);
    let exp_rate = -exp_slope_raw;

    let flat_ratio = if eigvals[k - 1] > 0.0 {
        eigvals[0] / eigvals[k - 1]
    } else {
        f64::INFINITY
    };

    let (decision, reasoning) =
        if pl_r2 >= POWERLAW_R2_THRESHOLD && pl_r2 >= exp_r2 + POWERLAW_BEATS_EXP_MARGIN {
            (
                GateDecision::Mera,
                format!(
                    "Power-law fit dominates: R²={:.4}, slope={:.3}. Log-correlation \
                     structure confirmed. Authenticated Energy-MERA is the correct \
                     state commitment.",
                    pl_r2, pl_slope
                ),
            )
        } else if exp_r2 >= EXPONENTIAL_R2_THRESHOLD {
            (
                GateDecision::Mps,
                format!(
                    "Exponential fit dominates: R²={:.4}, rate={:.4}. Area-law \
                     (short-range) correlation only. Downshift to Authenticated MPS.",
                    exp_r2, exp_rate
                ),
            )
        } else {
            (
                GateDecision::Verkle,
                format!(
                    "Neither power-law (R²={:.4}) nor exponential (R²={:.4}) fits well. \
                     Flat spectrum (ratio={:.1}x). Drop MERA/MPS; ship Verkle.",
                    pl_r2, exp_r2, flat_ratio
                ),
            )
        };

    GateResult {
        decision,
        powerlaw_slope: pl_slope,
        powerlaw_r2: pl_r2,
        exponential_rate: exp_rate,
        exponential_r2: exp_r2,
        flat_ratio,
        eigvals: eigvals.to_vec(),
        reasoning,
    }
}

/// Full pipeline: activation matrix → MI → top-K eigenvalues → classification.
/// `seed` drives the power-iteration starting vector so re-runs with the
/// same input are bit-identical.
pub fn run_gate(activations: &[Vec<f64>], k: usize, bins: usize, seed: u64) -> GateResult {
    let mi = compute_mi_matrix(activations, bins);
    let eigvals = top_k_eigenvalues(&mi, k, seed);
    classify_spectrum(&eigvals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_regression_recovers_slope_and_intercept() {
        let xs: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let ys: Vec<f64> = xs.iter().map(|x| 2.0 * x + 1.0).collect();
        let (slope, intercept) = linear_regression(&xs, &ys);
        assert!((slope - 2.0).abs() < 1e-9);
        assert!((intercept - 1.0).abs() < 1e-9);
    }

    #[test]
    fn r_squared_perfect_fit_is_one() {
        let ys = vec![1.0, 2.0, 3.0, 4.0];
        assert!((r_squared(&ys, &ys) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn r_squared_constant_target_is_zero() {
        // Constant ys → ss_tot == 0 → defined as 0.
        let ys = vec![5.0, 5.0, 5.0];
        let preds = vec![1.0, 2.0, 3.0];
        assert_eq!(r_squared(&ys, &preds), 0.0);
    }

    #[test]
    fn power_iteration_finds_top_eigenvalue_of_diagonal() {
        let m = vec![
            vec![3.0, 0.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0, 0.0],
            vec![0.0, 0.0, 0.5, 0.0],
            vec![0.0, 0.0, 0.0, 0.1],
        ];
        let eig = top_k_eigenvalues(&m, 4, 0xCAFE);
        assert_eq!(eig.len(), 4);
        // First should be ≈ 3.0; tolerances accept power-iteration drift.
        assert!((eig[0] - 3.0).abs() < 1e-3, "got {}", eig[0]);
        assert!((eig[1] - 1.0).abs() < 1e-3, "got {}", eig[1]);
        assert!((eig[2] - 0.5).abs() < 1e-3, "got {}", eig[2]);
    }

    #[test]
    fn classify_synthetic_power_law_picks_mera() {
        // Pure 1/i^2 spectrum → log-log slope = 2, R² = 1.
        let eigvals: Vec<f64> = (1..=40).map(|i| 1.0 / (i as f64).powi(2)).collect();
        let r = classify_spectrum(&eigvals);
        assert_eq!(r.decision, GateDecision::Mera);
        assert!(r.powerlaw_r2 > 0.99, "pl_r2={}", r.powerlaw_r2);
    }

    #[test]
    fn classify_synthetic_exponential_picks_mps() {
        // Pure exp(-0.3·i) spectrum → log-linear slope = 0.3, R² = 1.
        let eigvals: Vec<f64> = (1..=40).map(|i| (-0.3 * i as f64).exp()).collect();
        let r = classify_spectrum(&eigvals);
        assert_eq!(r.decision, GateDecision::Mps);
        assert!(r.exponential_r2 > 0.99, "exp_r2={}", r.exponential_r2);
    }

    #[test]
    fn classify_flat_spectrum_picks_verkle() {
        // Slight noise around 1.0 — neither fit exceeds the R² threshold.
        let eigvals: Vec<f64> = (1..=40).map(|i| 1.0 + 0.001 * ((i as f64).sin())).collect();
        let r = classify_spectrum(&eigvals);
        assert_eq!(r.decision, GateDecision::Verkle);
    }
}
