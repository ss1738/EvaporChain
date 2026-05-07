//! Argv parsing + input/output orchestration.

use std::io::{Read, Write};

use evaporchain_causal_chsh::chsh::ConcurrentPairSamples;
use evaporchain_causal_chsh::gate::GateThresholds;
use evaporchain_causal_chsh_renderer::{render_sweep_report, RenderOptions};
use evaporchain_causal_chsh_sweep::{run_sweep, SweepGrid};
use thiserror::Error;

/// Parsed command-line arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliArgs {
    /// Path to honest-sample JSON, or "-" for stdin.
    pub input: String,
    /// Path to write rendered markdown, or "-" for stdout. Default "-".
    pub output: String,
    /// Sweep seed as a 32-byte hex string. Default = all-zeros.
    pub seed: [u8; 32],
}

impl Default for CliArgs {
    fn default() -> Self {
        Self {
            input: String::new(),
            output: "-".to_string(),
            seed: [0u8; 32],
        }
    }
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("missing required argument: {0}")]
    MissingArgument(&'static str),
    #[error("unknown argument: {0}")]
    UnknownArgument(String),
    #[error("flag {flag} requires a value")]
    FlagMissingValue { flag: String },
    #[error("seed must be a 32-byte hex string (64 hex chars), got {got_chars} chars")]
    SeedWrongLength { got_chars: usize },
    #[error("seed contains non-hex character: {0}")]
    SeedBadHex(char),
    #[error("io error: {0}")]
    Io(String),
    #[error("json parse error: {0}")]
    JsonParse(String),
    #[error("sweep error: {0}")]
    Sweep(String),
}

/// Pluggable I/O so tests can drive the pipeline without touching
/// real files or stdin/stdout.
pub trait CliIo {
    fn read_input(&mut self, path: &str) -> Result<String, CliError>;
    fn write_output(&mut self, path: &str, content: &str) -> Result<(), CliError>;
}

/// Production I/O: filesystem + stdin/stdout.
pub struct StdIo;

impl CliIo for StdIo {
    fn read_input(&mut self, path: &str) -> Result<String, CliError> {
        if path == "-" {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| CliError::Io(format!("stdin: {e}")))?;
            Ok(buf)
        } else {
            std::fs::read_to_string(path).map_err(|e| CliError::Io(format!("{path}: {e}")))
        }
    }

    fn write_output(&mut self, path: &str, content: &str) -> Result<(), CliError> {
        if path == "-" {
            std::io::stdout()
                .write_all(content.as_bytes())
                .map_err(|e| CliError::Io(format!("stdout: {e}")))?;
            Ok(())
        } else {
            std::fs::write(path, content).map_err(|e| CliError::Io(format!("{path}: {e}")))
        }
    }
}

/// Parse `argv[1..]` into a `CliArgs`. Accepts `--input`, `--output`,
/// `--seed` long flags. No abbreviation.
pub fn parse_args(argv: &[String]) -> Result<CliArgs, CliError> {
    let mut args = CliArgs::default();
    let mut iter = argv.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--input" => {
                let v = iter.next().ok_or_else(|| CliError::FlagMissingValue {
                    flag: "--input".into(),
                })?;
                args.input = v.clone();
            }
            "--output" => {
                let v = iter.next().ok_or_else(|| CliError::FlagMissingValue {
                    flag: "--output".into(),
                })?;
                args.output = v.clone();
            }
            "--seed" => {
                let v = iter.next().ok_or_else(|| CliError::FlagMissingValue {
                    flag: "--seed".into(),
                })?;
                args.seed = parse_hex_seed(v)?;
            }
            other => return Err(CliError::UnknownArgument(other.to_string())),
        }
    }
    if args.input.is_empty() {
        return Err(CliError::MissingArgument("--input"));
    }
    Ok(args)
}

fn parse_hex_seed(s: &str) -> Result<[u8; 32], CliError> {
    let trimmed = s.strip_prefix("0x").unwrap_or(s);
    if trimmed.len() != 64 {
        return Err(CliError::SeedWrongLength {
            got_chars: trimmed.len(),
        });
    }
    let mut out = [0u8; 32];
    let bytes = trimmed.as_bytes();
    for i in 0..32 {
        let hi = hex_nibble(bytes[2 * i] as char)?;
        let lo = hex_nibble(bytes[2 * i + 1] as char)?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nibble(c: char) -> Result<u8, CliError> {
    match c {
        '0'..='9' => Ok(c as u8 - b'0'),
        'a'..='f' => Ok(10 + c as u8 - b'a'),
        'A'..='F' => Ok(10 + c as u8 - b'A'),
        _ => Err(CliError::SeedBadHex(c)),
    }
}

/// Run the full pipeline against a pluggable I/O.
pub fn run_cli<IO: CliIo>(argv: &[String], io: &mut IO) -> Result<(), CliError> {
    let args = parse_args(argv)?;
    let raw = io.read_input(&args.input)?;
    let honest: ConcurrentPairSamples =
        serde_json::from_str(&raw).map_err(|e| CliError::JsonParse(format!("{e}")))?;
    let report = run_sweep(
        &honest,
        &SweepGrid::doctrine(),
        GateThresholds::doctrine(),
        args.seed,
    )
    .map_err(|e| CliError::Sweep(format!("{e}")))?;
    let md = render_sweep_report(&report, RenderOptions::default());
    io.write_output(&args.output, &md)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// In-memory I/O. `inputs` keyed by path string; writes accumulate
    /// in `outputs`.
    struct MemIo {
        inputs: HashMap<String, String>,
        outputs: HashMap<String, String>,
    }

    impl MemIo {
        fn new() -> Self {
            Self {
                inputs: HashMap::new(),
                outputs: HashMap::new(),
            }
        }
    }

    impl CliIo for MemIo {
        fn read_input(&mut self, path: &str) -> Result<String, CliError> {
            self.inputs
                .get(path)
                .cloned()
                .ok_or_else(|| CliError::Io(format!("missing input {path}")))
        }
        fn write_output(&mut self, path: &str, content: &str) -> Result<(), CliError> {
            self.outputs.insert(path.to_string(), content.to_string());
            Ok(())
        }
    }

    fn balanced_honest_json(n: usize) -> String {
        let bal: Vec<i8> = (0..n).map(|i| if i % 2 == 0 { 1 } else { -1 }).collect();
        let s = ConcurrentPairSamples {
            samples_ab: bal.clone(),
            samples_ab_prime: bal.clone(),
            samples_a_prime_b: bal.clone(),
            samples_a_prime_b_prime: bal,
        };
        serde_json::to_string(&s).unwrap()
    }

    // ── argv parsing ─────────────────────────────────────────────

    #[test]
    fn parse_minimal_args() {
        let argv = vec!["--input".into(), "h.json".into()];
        let a = parse_args(&argv).unwrap();
        assert_eq!(a.input, "h.json");
        assert_eq!(a.output, "-");
        assert_eq!(a.seed, [0u8; 32]);
    }

    #[test]
    fn parse_full_args() {
        let argv = vec![
            "--input".into(),
            "h.json".into(),
            "--output".into(),
            "out.md".into(),
            "--seed".into(),
            "0x".to_string() + &"ab".repeat(32),
        ];
        let a = parse_args(&argv).unwrap();
        assert_eq!(a.input, "h.json");
        assert_eq!(a.output, "out.md");
        assert_eq!(a.seed, [0xabu8; 32]);
    }

    #[test]
    fn parse_seed_without_0x_prefix() {
        let argv = vec![
            "--input".into(),
            "h.json".into(),
            "--seed".into(),
            "ff".repeat(32),
        ];
        let a = parse_args(&argv).unwrap();
        assert_eq!(a.seed, [0xffu8; 32]);
    }

    #[test]
    fn parse_rejects_missing_input() {
        let argv: Vec<String> = vec![];
        let err = parse_args(&argv).unwrap_err();
        assert!(matches!(err, CliError::MissingArgument("--input")));
    }

    #[test]
    fn parse_rejects_flag_without_value() {
        let argv = vec!["--input".into()];
        let err = parse_args(&argv).unwrap_err();
        assert!(matches!(err, CliError::FlagMissingValue { .. }));
    }

    #[test]
    fn parse_rejects_unknown_flag() {
        let argv = vec!["--input".into(), "x".into(), "--bogus".into()];
        let err = parse_args(&argv).unwrap_err();
        assert!(matches!(err, CliError::UnknownArgument(_)));
    }

    #[test]
    fn parse_rejects_bad_seed_length() {
        let argv = vec!["--input".into(), "x".into(), "--seed".into(), "abcd".into()];
        let err = parse_args(&argv).unwrap_err();
        assert!(matches!(err, CliError::SeedWrongLength { got_chars: 4 }));
    }

    #[test]
    fn parse_rejects_bad_seed_hex() {
        let argv = vec![
            "--input".into(),
            "x".into(),
            "--seed".into(),
            "z".repeat(64),
        ];
        let err = parse_args(&argv).unwrap_err();
        assert!(matches!(err, CliError::SeedBadHex('z')));
    }

    // ── pipeline ─────────────────────────────────────────────────

    #[test]
    fn pipeline_writes_markdown_to_named_output() {
        let mut io = MemIo::new();
        io.inputs.insert("h.json".into(), balanced_honest_json(64));
        run_cli(
            &vec![
                "--input".into(),
                "h.json".into(),
                "--output".into(),
                "r.md".into(),
            ],
            &mut io,
        )
        .unwrap();
        let md = io.outputs.get("r.md").unwrap();
        assert!(md.starts_with("# Causal-CHSH Gate Result"));
        assert!(md.contains("## Coordinated-subset"));
        assert!(md.contains("## Biased-coin"));
        assert!(md.contains("## PR-box approximation"));
    }

    #[test]
    fn pipeline_writes_to_stdout_dash() {
        let mut io = MemIo::new();
        io.inputs.insert("-".into(), balanced_honest_json(32));
        run_cli(&vec!["--input".into(), "-".into()], &mut io).unwrap();
        // Default output = "-"
        assert!(io.outputs.contains_key("-"));
    }

    #[test]
    fn pipeline_uses_seed_for_determinism() {
        // Same seed → same output; different seeds → output differs
        // somewhere (exact bytes will differ in stochastic cells).
        let mut io_a = MemIo::new();
        let mut io_b = MemIo::new();
        let mut io_c = MemIo::new();
        let json = balanced_honest_json(2_000);
        io_a.inputs.insert("h.json".into(), json.clone());
        io_b.inputs.insert("h.json".into(), json.clone());
        io_c.inputs.insert("h.json".into(), json);

        let argv_seed_one = vec![
            "--input".into(),
            "h.json".into(),
            "--output".into(),
            "out.md".into(),
            "--seed".into(),
            "01".repeat(32),
        ];
        let argv_seed_two = vec![
            "--input".into(),
            "h.json".into(),
            "--output".into(),
            "out.md".into(),
            "--seed".into(),
            "02".repeat(32),
        ];

        run_cli(&argv_seed_one, &mut io_a).unwrap();
        run_cli(&argv_seed_one, &mut io_b).unwrap();
        run_cli(&argv_seed_two, &mut io_c).unwrap();

        let a = io_a.outputs.get("out.md").unwrap().clone();
        let b = io_b.outputs.get("out.md").unwrap().clone();
        let c = io_c.outputs.get("out.md").unwrap().clone();

        assert_eq!(a, b, "same seed must produce identical output");
        assert_ne!(a, c, "different seeds must produce different output");
    }

    #[test]
    fn pipeline_surfaces_json_parse_error() {
        let mut io = MemIo::new();
        io.inputs.insert("h.json".into(), "not-json".into());
        let err = run_cli(&vec!["--input".into(), "h.json".into()], &mut io).unwrap_err();
        assert!(matches!(err, CliError::JsonParse(_)));
    }

    #[test]
    fn pipeline_surfaces_sweep_error_on_empty_bucket() {
        let bad = ConcurrentPairSamples {
            samples_ab: vec![],
            samples_ab_prime: vec![1],
            samples_a_prime_b: vec![1],
            samples_a_prime_b_prime: vec![-1],
        };
        let mut io = MemIo::new();
        io.inputs
            .insert("h.json".into(), serde_json::to_string(&bad).unwrap());
        let err = run_cli(&vec!["--input".into(), "h.json".into()], &mut io).unwrap_err();
        assert!(matches!(err, CliError::Sweep(_)));
    }

    #[test]
    fn pipeline_surfaces_input_io_error() {
        let mut io = MemIo::new();
        let err = run_cli(&vec!["--input".into(), "missing.json".into()], &mut io).unwrap_err();
        assert!(matches!(err, CliError::Io(_)));
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Lane O.6 ships a stdlib-only CLI driver that wires
        // the full Causal-CHSH stack: read JSON honest-sample from
        // a path or stdin, run the doctrine sweep, render the
        // GATE_RESULT.md, write to a path or stdout. Same seed ->
        // same output. JSON / sweep / I/O errors surface as typed
        // CliError variants; no panics on bad input."
        let mut io = MemIo::new();
        io.inputs.insert("h.json".into(), balanced_honest_json(64));
        run_cli(
            &vec![
                "--input".into(),
                "h.json".into(),
                "--output".into(),
                "out.md".into(),
                "--seed".into(),
                "deadbeef".repeat(8),
            ],
            &mut io,
        )
        .unwrap();
        let md = io.outputs.get("out.md").unwrap();
        assert!(md.contains("Causal-CHSH Gate Result"));
        assert!(md.contains("detection floor:"));
        assert!(md.contains("discrimination ceiling:"));
    }
}
