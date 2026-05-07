//! Markdown rendering.

use std::fmt::Write;

use evaporchain_finality_attestation::{build_attestation, FinalityAttestation};

/// Operator-facing rendering knobs.
#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    /// Prefix all 32-byte hex fields with `0x`.
    pub hex_with_prefix: bool,
    /// Truncate fork lists at this many rows; 0 = unlimited.
    pub max_fork_rows: usize,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            hex_with_prefix: true,
            max_fork_rows: 0,
        }
    }
}

/// Render the attestation to a markdown string.
///
/// `precomputed_root`: if `Some`, it is shown verbatim. If `None`,
/// `build_attestation` is called and the resulting root rendered. If
/// the attestation is malformed (unsorted forks etc.), `"ERROR: <e>"`
/// appears in the root field instead — the renderer never panics.
pub fn render_attestation(
    att: &FinalityAttestation,
    precomputed_root: Option<&[u8; 32]>,
    opts: RenderOptions,
) -> String {
    let mut out = String::new();
    write_header(&mut out, att, precomputed_root, opts);
    write_causal_section(&mut out, att, opts);
    write_bell_section(&mut out, att, opts);
    write_forks_section(&mut out, att, opts);
    out
}

fn write_header(
    out: &mut String,
    att: &FinalityAttestation,
    precomputed_root: Option<&[u8; 32]>,
    opts: RenderOptions,
) {
    let _ = writeln!(out, "# Finality Attestation");
    let _ = writeln!(out);
    let _ = writeln!(out, "## Summary");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- block_hash         = {}",
        format_hex(&att.block_hash, opts.hex_with_prefix)
    );
    let _ = writeln!(out, "- finalised_at_epoch = {}", att.finalised_at_epoch);

    let root_repr = match precomputed_root {
        Some(r) => format_hex(r, opts.hex_with_prefix),
        None => match build_attestation(att) {
            Ok(r) => format_hex(&r, opts.hex_with_prefix),
            Err(e) => format!("ERROR: {e}"),
        },
    };
    let _ = writeln!(out, "- attestation_root   = {}", root_repr);
    let _ = writeln!(out, "- evaporated_forks   = {}", att.evaporated_forks.len());
    let _ = writeln!(out);
}

fn write_causal_section(out: &mut String, att: &FinalityAttestation, opts: RenderOptions) {
    let _ = writeln!(out, "## Light-Cone V2 (causal-root)");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- causal_root = {}",
        format_hex(&att.causal_root, opts.hex_with_prefix)
    );
    let _ = writeln!(out);
}

fn write_bell_section(out: &mut String, att: &FinalityAttestation, opts: RenderOptions) {
    let _ = writeln!(out, "## Bell-Beacon V2 (seed)");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- bell_seed = {}",
        format_hex(&att.bell_seed, opts.hex_with_prefix)
    );
    let _ = writeln!(out);
}

fn write_forks_section(out: &mut String, att: &FinalityAttestation, opts: RenderOptions) {
    let _ = writeln!(out, "## Evaporated forks (Evap-Fork-Cert V2)");
    let _ = writeln!(out);
    if att.evaporated_forks.is_empty() {
        let _ = writeln!(out, "_no evaporated forks committed_");
        let _ = writeln!(out);
        return;
    }

    let _ = writeln!(out, "| # | fork_root | witness |");
    let _ = writeln!(out, "|--:|:---|:---|");

    let total = att.evaporated_forks.len();
    let limit = if opts.max_fork_rows == 0 {
        total
    } else {
        opts.max_fork_rows.min(total)
    };
    for (i, w) in att.evaporated_forks.iter().enumerate().take(limit) {
        let _ = writeln!(
            out,
            "| {} | {} | {} |",
            i + 1,
            format_hex(&w.fork_root, opts.hex_with_prefix),
            format_hex(&w.witness, opts.hex_with_prefix),
        );
    }
    if limit < total {
        let _ = writeln!(out, "| … | …{} more rows… | … |", total - limit);
    }
    let _ = writeln!(out);
}

fn format_hex(bytes: &[u8; 32], prefix: bool) -> String {
    let mut s = String::with_capacity(if prefix { 66 } else { 64 });
    if prefix {
        s.push_str("0x");
    }
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_finality_attestation::EvaporatedForkWitnessRef;

    fn fwr(root_byte: u8, witness_byte: u8) -> EvaporatedForkWitnessRef {
        let mut fr = [0u8; 32];
        fr[0] = root_byte;
        let mut w = [0u8; 32];
        w[0] = witness_byte;
        EvaporatedForkWitnessRef {
            fork_root: fr,
            witness: w,
        }
    }

    fn att(forks: Vec<EvaporatedForkWitnessRef>) -> FinalityAttestation {
        FinalityAttestation {
            block_hash: [1u8; 32],
            finalised_at_epoch: 12345,
            causal_root: [2u8; 32],
            bell_seed: [3u8; 32],
            evaporated_forks: forks,
        }
    }

    // ── header content ───────────────────────────────────────────

    #[test]
    fn render_starts_with_h1() {
        let s = render_attestation(&att(vec![]), None, RenderOptions::default());
        assert!(s.starts_with("# Finality Attestation"));
    }

    #[test]
    fn render_summary_includes_all_fields() {
        let s = render_attestation(&att(vec![fwr(0x10, 0xaa)]), None, RenderOptions::default());
        assert!(s.contains("## Summary"));
        assert!(s.contains("block_hash"));
        assert!(s.contains("finalised_at_epoch = 12345"));
        assert!(s.contains("attestation_root"));
        assert!(s.contains("evaporated_forks   = 1"));
    }

    #[test]
    fn render_three_commitment_sections() {
        let s = render_attestation(&att(vec![]), None, RenderOptions::default());
        assert!(s.contains("## Light-Cone V2"));
        assert!(s.contains("## Bell-Beacon V2"));
        assert!(s.contains("## Evaporated forks"));
    }

    // ── hex formatting ───────────────────────────────────────────

    #[test]
    fn hex_with_0x_prefix_by_default() {
        let s = render_attestation(&att(vec![]), None, RenderOptions::default());
        // block_hash = [1u8; 32] → "01" repeated 32 times.
        let expected = format!("0x{}", "01".repeat(32));
        assert!(s.contains(&expected));
    }

    #[test]
    fn hex_without_prefix_when_disabled() {
        let opts = RenderOptions {
            hex_with_prefix: false,
            max_fork_rows: 0,
        };
        let s = render_attestation(&att(vec![]), None, opts);
        // Should NOT contain "0x" prefix (but still 64 hex chars).
        assert!(!s.contains("= 0x"));
        assert!(s.contains(&"01".repeat(32)));
    }

    // ── attestation root sourcing ────────────────────────────────

    #[test]
    fn precomputed_root_is_rendered_verbatim() {
        let custom = [0xabu8; 32];
        let s = render_attestation(&att(vec![]), Some(&custom), RenderOptions::default());
        let expected = format!("0x{}", "ab".repeat(32));
        assert!(s.contains(&expected));
    }

    #[test]
    fn precomputed_none_falls_back_to_build() {
        let s = render_attestation(&att(vec![fwr(0x10, 0xaa)]), None, RenderOptions::default());
        // A real 32-byte root will be hexed; we just confirm there's
        // a hex root line and no ERROR: prefix.
        assert!(s.contains("attestation_root   = 0x"));
        assert!(!s.contains("ERROR:"));
    }

    #[test]
    fn malformed_attestation_renders_error_marker() {
        // Unsorted forks → build_attestation errors → renderer reports it.
        let bad = att(vec![fwr(0x20, 0x00), fwr(0x10, 0x00)]);
        let s = render_attestation(&bad, None, RenderOptions::default());
        assert!(s.contains("ERROR: "));
        // Renderer doesn't panic.
    }

    // ── forks table ──────────────────────────────────────────────

    #[test]
    fn empty_forks_emits_placeholder() {
        let s = render_attestation(&att(vec![]), None, RenderOptions::default());
        assert!(s.contains("_no evaporated forks committed_"));
    }

    #[test]
    fn forks_table_lists_all_rows_by_default() {
        let s = render_attestation(
            &att(vec![fwr(0x10, 0xaa), fwr(0x20, 0xbb), fwr(0x30, 0xcc)]),
            None,
            RenderOptions::default(),
        );
        assert!(s.contains("| 1 |"));
        assert!(s.contains("| 2 |"));
        assert!(s.contains("| 3 |"));
        assert!(!s.contains("more rows"));
    }

    #[test]
    fn max_fork_rows_truncates() {
        let opts = RenderOptions {
            hex_with_prefix: true,
            max_fork_rows: 2,
        };
        let s = render_attestation(
            &att(vec![fwr(0x10, 0xaa), fwr(0x20, 0xbb), fwr(0x30, 0xcc)]),
            None,
            opts,
        );
        assert!(s.contains("| 1 |"));
        assert!(s.contains("| 2 |"));
        assert!(s.contains("…1 more rows…"));
        assert!(!s.contains("| 3 |"));
    }

    // ── determinism ──────────────────────────────────────────────

    #[test]
    fn render_is_deterministic() {
        let a = att(vec![fwr(0x10, 0xaa), fwr(0x20, 0xbb)]);
        let s1 = render_attestation(&a, None, RenderOptions::default());
        let s2 = render_attestation(&a, None, RenderOptions::default());
        assert_eq!(s1, s2);
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "The renderer turns a FinalityAttestation into a
        // FINALITY.md-shaped doctrine document. H1 title, summary
        // block (block_hash, finalised_at_epoch, attestation_root,
        // forks count), three commitment sections (Light-Cone V2,
        // Bell-Beacon V2, Evaporated forks). Hex fields default to
        // 0x prefix; max_fork_rows truncates with a count footnote.
        // Malformed attestations render an ERROR marker rather than
        // panic."

        let s = render_attestation(
            &att(vec![fwr(0x10, 0xaa), fwr(0x20, 0xbb)]),
            None,
            RenderOptions::default(),
        );
        assert!(s.starts_with("# Finality Attestation"));
        assert!(s.contains("## Summary"));
        assert!(s.contains("## Light-Cone V2"));
        assert!(s.contains("## Bell-Beacon V2"));
        assert!(s.contains("## Evaporated forks"));
        assert!(s.contains("attestation_root   = 0x"));
        assert!(!s.contains("ERROR:"));

        // Malformed → ERROR marker, no panic.
        let bad = att(vec![fwr(0x20, 0x00), fwr(0x10, 0x00)]); // unsorted
        let s_bad = render_attestation(&bad, None, RenderOptions::default());
        assert!(s_bad.contains("ERROR:"));
    }
}
