//! Lane O.3 — real Ethereum Causal-CHSH gate runner.
//!
//! Reads a CSV of `BlockSummary` rows produced by
//! `research/causal-chsh/scrape_eth.py`, runs `extract_chsh_samples`
//! against it (honest sample), generates a same-size synthetic cartel
//! injection, runs `run_synthetic_gate` with the doctrine thresholds,
//! and writes `research/causal-chsh/GATE_RESULT.md` with the locked
//! verdict.
//!
//! ## Pre-commit discipline (MERA-style)
//!
//! The thresholds (honest_ceiling=1.8, cartel_floor=2.2, min_gap=0.4)
//! are doctrine-locked in `gate::GateThresholds::doctrine()` BEFORE
//! this binary runs. The verdict is binary: pass → primitive ships
//! as a Tier-0-supporting row in INVENTION_STACK.md; fail → the
//! crate is retained as a research artefact only, same as MERA.
//!
//! Usage:
//!
//!   cargo run -p evaporchain-causal-chsh --release --bin causal_chsh_run_gate \
//!       -- HONEST.csv [--window-secs 60]
//!
//! CSV format (header row required):
//!
//!   height,timestamp_secs,energy,gas,tx_count

use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use evaporchain_causal_chsh::{
    chsh::compute_chsh_s,
    extract_chsh_samples,
    gate::{run_synthetic_gate, GateThresholds, GateVerdict},
    synthesize_max_cartel_samples, BlockSummary,
};

const REPORT_PATH: &str = "/Users/satyawansingh/EvaporChain/research/causal-chsh/GATE_RESULT.md";

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "Usage: {} HONEST.csv [--window-secs N] [--report PATH]",
            args[0]
        );
        std::process::exit(2);
    }
    let csv_path = PathBuf::from(&args[1]);
    let mut window_secs: u64 = 60;
    let mut report_path = PathBuf::from(REPORT_PATH);
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--window-secs" => {
                window_secs = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(60);
                i += 2;
            }
            "--report" => {
                report_path = PathBuf::from(args.get(i + 1).cloned().unwrap_or_default());
                i += 2;
            }
            other => {
                eprintln!("unknown arg: {}", other);
                std::process::exit(2);
            }
        }
    }

    println!(
        "Lane O.3 — Causal-CHSH real-Eth gate · csv={} window={}s",
        csv_path.display(),
        window_secs
    );

    let trace = read_csv(&csv_path);
    println!("loaded {} blocks", trace.len());
    if trace.len() < 50 {
        eprintln!(
            "too few blocks ({} < 50) — gate verdict would be noise-bound",
            trace.len()
        );
        std::process::exit(3);
    }

    let honest = extract_chsh_samples(&trace, window_secs);
    let n = honest.samples_ab.len()
        + honest.samples_ab_prime.len()
        + honest.samples_a_prime_b.len()
        + honest.samples_a_prime_b_prime.len();
    let n_per_bucket = honest.samples_ab.len();
    println!(
        "honest: {} total samples ({}/{}/{}/{} per setting-pair)",
        n,
        honest.samples_ab.len(),
        honest.samples_ab_prime.len(),
        honest.samples_a_prime_b.len(),
        honest.samples_a_prime_b_prime.len()
    );

    if n_per_bucket < 5 {
        eprintln!(
            "under-populated buckets ({} per setting-pair) — widen --window-secs",
            n_per_bucket
        );
        std::process::exit(3);
    }

    let s_honest = compute_chsh_s(&honest).expect("honest S");
    println!("S_honest = {:.6}", s_honest);

    let cartel = synthesize_max_cartel_samples(n_per_bucket);
    let s_cartel = compute_chsh_s(&cartel).expect("cartel S");
    println!("S_cartel = {:.6}", s_cartel);

    let thresholds = GateThresholds::doctrine();
    let verdict = run_synthetic_gate(&honest, &cartel, thresholds);
    let (verdict_label, body) = format_verdict(&verdict);
    println!("\nVERDICT: {}\n{}", verdict_label, body);

    let report = render_report(
        &csv_path,
        window_secs,
        trace.len(),
        n,
        s_honest,
        s_cartel,
        &thresholds,
        &verdict,
        &verdict_label,
    );
    if let Some(parent) = report_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = File::create(&report_path).expect("create report");
    f.write_all(report.as_bytes()).expect("write report");
    println!("\nReport: {}", report_path.display());

    // Exit code 0 = pass, 1 = fail, others reserved.
    match verdict {
        GateVerdict::Pass { .. } => std::process::exit(0),
        GateVerdict::Fail { .. } => std::process::exit(1),
        GateVerdict::InputError(_) => std::process::exit(4),
    }
}

fn read_csv(path: &PathBuf) -> Vec<BlockSummary> {
    let f = File::open(path).expect("open csv");
    let mut rdr = BufReader::new(f);
    let mut header = String::new();
    rdr.read_line(&mut header).expect("read header");
    let mut out = Vec::new();
    for line in rdr.lines() {
        let line = line.expect("read line");
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 5 {
            continue;
        }
        out.push(BlockSummary {
            height: cols[0].parse().unwrap_or(0),
            timestamp_secs: cols[1].parse().unwrap_or(0),
            energy: cols[2].parse().unwrap_or(0),
            gas: cols[3].parse().unwrap_or(0),
            tx_count: cols[4].parse().unwrap_or(0),
        });
    }
    out
}

fn format_verdict(v: &GateVerdict) -> (String, String) {
    match v {
        GateVerdict::Pass { s_honest, s_cartel, gap } => (
            "PASS — Causal-CHSH SHIPS".to_string(),
            format!(
                "  S_honest = {:.4}  (< 1.80 ceiling ✓)\n  S_cartel = {:.4}  (> 2.20 floor ✓)\n  gap      = {:.4}  (> 0.40 min ✓)",
                s_honest, s_cartel, gap
            ),
        ),
        GateVerdict::Fail { s_honest, s_cartel, gap, reasons } => (
            "FAIL — DROP".to_string(),
            format!(
                "  S_honest = {:.4}\n  S_cartel = {:.4}\n  gap      = {:.4}\n\nReasons:\n{}",
                s_honest, s_cartel, gap,
                reasons.iter().map(|r| format!("  - {}", r)).collect::<Vec<_>>().join("\n")
            ),
        ),
        GateVerdict::InputError(msg) => (
            "INPUT ERROR".to_string(),
            format!("  {}", msg),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_report(
    csv_path: &Path,
    window_secs: u64,
    n_blocks: usize,
    n_samples_total: usize,
    s_honest: f64,
    s_cartel: f64,
    th: &GateThresholds,
    verdict: &GateVerdict,
    verdict_label: &str,
) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let gap = s_cartel - s_honest;
    let pass_ceiling = if s_honest < th.honest_ceiling {
        "✓"
    } else {
        "✗"
    };
    let pass_floor = if s_cartel > th.cartel_floor {
        "✓"
    } else {
        "✗"
    };
    let pass_gap = if gap > th.min_gap { "✓" } else { "✗" };

    let mut report = format!(
        "# EvaporChain Causal-CHSH Empirical Gate — Results\n\n\
         **Run (unix epoch):** {now}\n\
         **Source:** `{}`\n\
         **Concurrency window:** {window_secs} s\n\
         **Blocks analysed:** {n_blocks}\n\
         **Total ±1 samples:** {n_samples_total} (~ {} per setting-pair)\n\
         **Reference:** `crates/evaporchain-causal-chsh/src/lib.rs` doctrine block\n\n\
         ---\n\n\
         ## Gate Verdict\n\n\
         ```\nVERDICT: {verdict_label}\n```\n\n\
         | Quantity | Value | Threshold | Pass? |\n\
         |---|---|---|---|\n\
         | S_honest | {:.4} | < {:.2} | {} |\n\
         | S_cartel | {:.4} | > {:.2} | {} |\n\
         | gap (S_cartel − S_honest) | {:.4} | > {:.2} | {} |\n\n",
        csv_path.display(),
        n_samples_total / 4,
        s_honest,
        th.honest_ceiling,
        pass_ceiling,
        s_cartel,
        th.cartel_floor,
        pass_floor,
        gap,
        th.min_gap,
        pass_gap,
    );

    match verdict {
        GateVerdict::Pass { .. } => {
            report.push_str(
                "## Doctrine action\n\n\
                 All three thresholds passed. **Causal-CHSH SHIPS** as a \
                 Tier-0-supporting row in `INVENTION_STACK.md §A1.3`. The \
                 inequality empirically discriminates honest from cartel \
                 traffic on real Ethereum blocks under the concurrency-window \
                 proxy. EvaporChain's first 100% original frontier primitive \
                 has earned its slot.\n\n\
                 Next steps:\n\
                 - Reserve the §A1.3 row + cartel-detector cross-reference\n\
                 - Lane O.4: consensus integration — wire a `cartel_alarm` \
                   that runs the gate on rolling windows + emits an alarm \
                   event when S exceeds the cartel_floor\n",
            );
        }
        GateVerdict::Fail { reasons, .. } => {
            report.push_str("## Doctrine action\n\n**FAIL — DROP.**\n\n");
            report.push_str("Reasons:\n");
            for r in reasons {
                report.push_str(&format!("- {}\n", r));
            }
            report.push_str(
                "\nPer the pre-commit doctrine (locked in the crate root \
                 docstring before running): the primitive is dropped. The \
                 `evaporchain-causal-chsh` crate is retained as a research \
                 artefact only — usable as a reference implementation if \
                 EvaporChain ever runs a true LightCone testnet whose \
                 concurrent-block topology might satisfy the bound where \
                 the linear-Eth proxy did not.\n\n\
                 Same MERA-style discipline. The gate is a feature, not a \
                 bug — a primitive that can fail empirically is one that \
                 can ship credibly when it doesn't.\n",
            );
        }
        GateVerdict::InputError(msg) => {
            report.push_str(&format!("## Input error\n\n{}\n", msg));
        }
    }
    report
}
