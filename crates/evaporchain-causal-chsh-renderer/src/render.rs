//! Markdown rendering.

use std::fmt::Write;

use evaporchain_causal_chsh_sweep::{
    CartelKind, GridPoint, SweepCell, SweepReport, SweepRow,
};

/// Operator-facing rendering knobs.
#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    /// Print fail reasons inline in the verdict column. If false,
    /// the verdict column shows only "PASS" / "FAIL" / "INPUT_ERR" /
    /// "CARTEL_ERR" and the reasons go in a footnote section.
    pub inline_reasons: bool,
    /// Number of decimal places for the float columns.
    pub decimals: usize,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            inline_reasons: true,
            decimals: 4,
        }
    }
}

/// Render a sweep report to a doctrine `GATE_RESULT.md` string.
pub fn render_sweep_report(report: &SweepReport, opts: RenderOptions) -> String {
    let mut out = String::new();
    write_header(&mut out, report, opts);
    write_section(
        &mut out,
        "Coordinated-subset",
        CartelKind::CoordinatedSubset,
        report,
        opts,
    );
    write_section(
        &mut out,
        "Biased-coin",
        CartelKind::BiasedCoin,
        report,
        opts,
    );
    write_section(
        &mut out,
        "PR-box approximation",
        CartelKind::PrBox,
        report,
        opts,
    );
    out
}

fn write_header(out: &mut String, report: &SweepReport, opts: RenderOptions) {
    let _ = writeln!(out, "# Causal-CHSH Gate Result");
    let _ = writeln!(out);
    let _ = writeln!(out, "## Summary");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- S_honest = {:.*}",
        opts.decimals, report.s_honest
    );
    let t = report.thresholds;
    let _ = writeln!(
        out,
        "- thresholds = {{honest_ceiling={honest:.2}, cartel_floor={floor:.2}, min_gap={gap:.2}}}",
        honest = t.honest_ceiling,
        floor = t.cartel_floor,
        gap = t.min_gap,
    );
    let _ = writeln!(out, "- rows = {}", report.rows.len());
    let _ = writeln!(out);
    write_summary_table(out, report);
}

/// Per-model summary table at the top: just verdict counts +
/// floor/ceiling pointers.
fn write_summary_table(out: &mut String, report: &SweepReport) {
    let _ = writeln!(out, "| model | pass | fail | err | floor | ceiling |");
    let _ = writeln!(out, "|:---|---:|---:|---:|:---|:---|");
    for (label, kind) in [
        ("Coordinated-subset", CartelKind::CoordinatedSubset),
        ("Biased-coin", CartelKind::BiasedCoin),
        ("PR-box", CartelKind::PrBox),
    ] {
        let (p, f, e) = report.counts(kind);
        let floor = format_grid_point_opt(report.detection_floor(kind));
        let ceiling = format_grid_point_opt(report.discrimination_ceiling(kind));
        let _ = writeln!(out, "| {label} | {p} | {f} | {e} | {floor} | {ceiling} |");
    }
    let _ = writeln!(out);
}

fn format_grid_point_opt(p: Option<GridPoint>) -> String {
    match p {
        Some(g) => format!("{}/{}", g.num, g.den),
        None => "—".to_string(),
    }
}

fn write_section(
    out: &mut String,
    title: &str,
    kind: CartelKind,
    report: &SweepReport,
    opts: RenderOptions,
) {
    let _ = writeln!(out, "## {title}");
    let _ = writeln!(out);

    let _ = writeln!(out, "| num/den | S_honest | S_cartel | gap | verdict |");
    let _ = writeln!(out, "|:---|---:|---:|---:|:---|");

    let mut footnotes: Vec<String> = Vec::new();
    let mut footnote_idx: usize = 0;
    for row in report.rows.iter().filter(|r| r.kind == kind) {
        write_row(out, row, opts, &mut footnotes, &mut footnote_idx);
    }
    let _ = writeln!(out);

    let _ = writeln!(
        out,
        "- detection floor: {}",
        format_grid_point_opt(report.detection_floor(kind))
    );
    let _ = writeln!(
        out,
        "- discrimination ceiling: {}",
        format_grid_point_opt(report.discrimination_ceiling(kind))
    );
    let _ = writeln!(out);

    if !opts.inline_reasons && !footnotes.is_empty() {
        let _ = writeln!(out, "### Reasons");
        let _ = writeln!(out);
        for (i, fn_) in footnotes.iter().enumerate() {
            let _ = writeln!(out, "{}. {}", i + 1, fn_);
        }
        let _ = writeln!(out);
    }
}

fn write_row(
    out: &mut String,
    row: &SweepRow,
    opts: RenderOptions,
    footnotes: &mut Vec<String>,
    footnote_idx: &mut usize,
) {
    let p = row.point;
    let pt = format!("{}/{}", p.num, p.den);
    match &row.cell {
        SweepCell::Pass {
            s_honest,
            s_cartel,
            gap,
        } => {
            let _ = writeln!(
                out,
                "| {pt} | {sh:.*} | {sc:.*} | {g:.*} | PASS |",
                opts.decimals,
                opts.decimals,
                opts.decimals,
                sh = s_honest,
                sc = s_cartel,
                g = gap,
            );
        }
        SweepCell::Fail {
            s_honest,
            s_cartel,
            gap,
            reasons,
        } => {
            let verdict_text = if opts.inline_reasons {
                format!("FAIL: {}", reasons.join("; "))
            } else if reasons.is_empty() {
                "FAIL".to_string()
            } else {
                *footnote_idx += 1;
                footnotes.push(reasons.join("; "));
                format!("FAIL[^{}]", *footnote_idx)
            };
            let _ = writeln!(
                out,
                "| {pt} | {sh:.*} | {sc:.*} | {g:.*} | {v} |",
                opts.decimals,
                opts.decimals,
                opts.decimals,
                sh = s_honest,
                sc = s_cartel,
                g = gap,
                v = escape_pipes(&verdict_text),
            );
        }
        SweepCell::InputError(msg) => {
            let _ = writeln!(
                out,
                "| {pt} | — | — | — | INPUT_ERR: {} |",
                escape_pipes(msg)
            );
        }
        SweepCell::CartelError(msg) => {
            let _ = writeln!(
                out,
                "| {pt} | — | — | — | CARTEL_ERR: {} |",
                escape_pipes(msg)
            );
        }
    }
}

/// Pipes break GFM tables; replace with U+2758 LIGHT VERTICAL BAR.
fn escape_pipes(s: &str) -> String {
    s.replace('|', "\u{2758}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_causal_chsh::chsh::ConcurrentPairSamples;
    use evaporchain_causal_chsh::gate::GateThresholds;
    use evaporchain_causal_chsh_sweep::{run_sweep, SweepGrid};

    fn balanced_honest(n: usize) -> ConcurrentPairSamples {
        let bal: Vec<i8> = (0..n)
            .map(|i| if i % 2 == 0 { 1 } else { -1 })
            .collect();
        ConcurrentPairSamples {
            samples_ab: bal.clone(),
            samples_ab_prime: bal.clone(),
            samples_a_prime_b: bal.clone(),
            samples_a_prime_b_prime: bal,
        }
    }

    fn doctrine_report() -> SweepReport {
        let h = balanced_honest(64);
        run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            [42u8; 32],
        )
        .unwrap()
    }

    #[test]
    fn render_starts_with_h1_header() {
        let r = doctrine_report();
        let s = render_sweep_report(&r, RenderOptions::default());
        assert!(s.starts_with("# Causal-CHSH Gate Result"));
    }

    #[test]
    fn render_contains_summary_section_with_threshold_line() {
        let r = doctrine_report();
        let s = render_sweep_report(&r, RenderOptions::default());
        assert!(s.contains("## Summary"));
        assert!(s.contains("honest_ceiling=1.80"));
        assert!(s.contains("cartel_floor=2.20"));
        assert!(s.contains("min_gap=0.40"));
    }

    #[test]
    fn render_contains_three_model_sections() {
        let r = doctrine_report();
        let s = render_sweep_report(&r, RenderOptions::default());
        assert!(s.contains("## Coordinated-subset"));
        assert!(s.contains("## Biased-coin"));
        assert!(s.contains("## PR-box approximation"));
    }

    #[test]
    fn render_has_eleven_data_rows_per_model() {
        let r = doctrine_report();
        let s = render_sweep_report(&r, RenderOptions::default());
        // The doctrine grid has 11 rows per model. Count pipe-separated
        // table rows that start with "| 0/10" .. "| 10/10".
        let mut grid_row_count = 0usize;
        for line in s.lines() {
            for n in 0..=10 {
                if line.starts_with(&format!("| {}/10", n)) {
                    grid_row_count += 1;
                }
            }
        }
        // 11 grid points × 3 models = 33 grid rows in the per-model
        // tables, plus the same set of point labels do NOT appear in
        // the summary table (which uses model name, not point).
        assert_eq!(grid_row_count, 33);
    }

    #[test]
    fn render_marks_endpoint_pass_and_fail() {
        let r = doctrine_report();
        let s = render_sweep_report(&r, RenderOptions::default());
        // Endpoint num=0 fails; num=10 passes — should appear in
        // markdown output.
        assert!(s.contains("PASS"));
        assert!(s.contains("FAIL"));
    }

    #[test]
    fn detection_floor_line_present() {
        let r = doctrine_report();
        let s = render_sweep_report(&r, RenderOptions::default());
        assert!(s.contains("detection floor:"));
        assert!(s.contains("discrimination ceiling:"));
    }

    #[test]
    fn footnote_mode_emits_reasons_section() {
        let r = doctrine_report();
        let opts = RenderOptions {
            inline_reasons: false,
            decimals: 4,
        };
        let s = render_sweep_report(&r, opts);
        // FAIL rows produce footnote markers.
        assert!(s.contains("FAIL[^1]"));
        assert!(s.contains("### Reasons"));
        assert!(s.contains("1."));
    }

    #[test]
    fn inline_mode_does_not_emit_reasons_section() {
        let r = doctrine_report();
        let opts = RenderOptions {
            inline_reasons: true,
            decimals: 4,
        };
        let s = render_sweep_report(&r, opts);
        assert!(!s.contains("### Reasons"));
        // FAIL rows should contain "FAIL: " prefix.
        assert!(s.contains("FAIL:"));
    }

    #[test]
    fn decimals_option_changes_output_width() {
        let r = doctrine_report();
        let s2 = render_sweep_report(
            &r,
            RenderOptions {
                inline_reasons: true,
                decimals: 2,
            },
        );
        let s6 = render_sweep_report(
            &r,
            RenderOptions {
                inline_reasons: true,
                decimals: 6,
            },
        );
        assert!(s6.len() > s2.len());
    }

    #[test]
    fn pipe_escape_does_not_break_table() {
        // Synthetic SweepCell with a pipe in the message would
        // normally break GFM tables. Smoke-test by reaching the
        // escape function directly.
        assert_eq!(escape_pipes("a|b|c"), "a\u{2758}b\u{2758}c");
    }

    #[test]
    fn render_is_deterministic() {
        let r = doctrine_report();
        let a = render_sweep_report(&r, RenderOptions::default());
        let b = render_sweep_report(&r, RenderOptions::default());
        assert_eq!(a, b);
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Lane O.5 ships the markdown renderer for the
        // Causal-CHSH sweep. A SweepReport becomes a doctrine
        // GATE_RESULT.md string with: H1 title, summary block (S_honest
        // + thresholds + count), summary table (per-model pass/fail/err
        // counts + floor/ceiling), three per-model tables (one row per
        // grid point with S_honest, S_cartel, gap, verdict), and
        // detection-floor / discrimination-ceiling annotations.
        // Inline-reasons mode keeps fail reasons in the verdict column;
        // footnote mode pulls them into a Reasons section."
        let r = doctrine_report();
        let s = render_sweep_report(&r, RenderOptions::default());

        // H1 + summary + three sections.
        assert!(s.starts_with("# Causal-CHSH Gate Result"));
        assert!(s.contains("## Summary"));
        assert!(s.contains("## Coordinated-subset"));
        assert!(s.contains("## Biased-coin"));
        assert!(s.contains("## PR-box approximation"));

        // Floor + ceiling annotations on every section.
        let floor_count = s.matches("detection floor:").count();
        let ceiling_count = s.matches("discrimination ceiling:").count();
        assert_eq!(floor_count, 3);
        assert_eq!(ceiling_count, 3);

        // Footnote mode flips the rendering.
        let s_fn = render_sweep_report(
            &r,
            RenderOptions {
                inline_reasons: false,
                decimals: 4,
            },
        );
        assert!(s_fn.contains("### Reasons"));
        assert!(!s.contains("### Reasons"));
    }
}
