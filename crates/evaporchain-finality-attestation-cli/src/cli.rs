//! Argv parser + pipeline.

use std::io::{Read, Write};

use evaporchain_finality_attestation::{
    build_attestation, verify_attestation, FinalityAttestation,
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subcommand {
    Build,
    Verify { root: [u8; 32] },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliArgs {
    pub sub: Subcommand,
    pub input: String,
    pub output: String,
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("missing required argument: {0}")]
    MissingArgument(&'static str),
    #[error("unknown argument: {0}")]
    UnknownArgument(String),
    #[error("flag {0} requires a value")]
    FlagMissingValue(String),
    #[error("unknown subcommand: {0} (expected `build` or `verify`)")]
    UnknownSubcommand(String),
    #[error("missing subcommand (expected `build` or `verify`)")]
    MissingSubcommand,
    #[error("hex value must be exactly 64 hex chars (got {got})")]
    HexWrongLength { got: usize },
    #[error("hex value contains non-hex character: {0}")]
    HexBadChar(char),
    #[error("io error: {0}")]
    Io(String),
    #[error("json parse error: {0}")]
    JsonParse(String),
    #[error("attestation error: {0}")]
    Attestation(String),
}

pub trait CliIo {
    fn read_input(&mut self, path: &str) -> Result<String, CliError>;
    fn write_output(&mut self, path: &str, content: &str) -> Result<(), CliError>;
}

pub fn parse_args(argv: &[String]) -> Result<CliArgs, CliError> {
    let mut iter = argv.iter();
    let sub_name = iter.next().ok_or(CliError::MissingSubcommand)?;
    let sub_kind = sub_name.as_str();

    let mut input = String::new();
    let mut output = "-".to_string();
    let mut root_hex: Option<String> = None;
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--input" => {
                let v = iter
                    .next()
                    .ok_or_else(|| CliError::FlagMissingValue("--input".into()))?;
                input = v.clone();
            }
            "--output" => {
                let v = iter
                    .next()
                    .ok_or_else(|| CliError::FlagMissingValue("--output".into()))?;
                output = v.clone();
            }
            "--root" => {
                let v = iter
                    .next()
                    .ok_or_else(|| CliError::FlagMissingValue("--root".into()))?;
                root_hex = Some(v.clone());
            }
            other => return Err(CliError::UnknownArgument(other.to_string())),
        }
    }
    if input.is_empty() {
        return Err(CliError::MissingArgument("--input"));
    }

    let sub = match sub_kind {
        "build" => Subcommand::Build,
        "verify" => {
            let raw = root_hex.ok_or(CliError::MissingArgument("--root"))?;
            Subcommand::Verify {
                root: parse_hex32(&raw)?,
            }
        }
        other => return Err(CliError::UnknownSubcommand(other.to_string())),
    };

    Ok(CliArgs { sub, input, output })
}

fn parse_hex32(s: &str) -> Result<[u8; 32], CliError> {
    let trimmed = s.strip_prefix("0x").unwrap_or(s);
    if trimmed.len() != 64 {
        return Err(CliError::HexWrongLength { got: trimmed.len() });
    }
    let bytes = trimmed.as_bytes();
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = (hex_nibble(bytes[2 * i] as char)? << 4) | hex_nibble(bytes[2 * i + 1] as char)?;
    }
    Ok(out)
}

fn hex_nibble(c: char) -> Result<u8, CliError> {
    match c {
        '0'..='9' => Ok(c as u8 - b'0'),
        'a'..='f' => Ok(10 + c as u8 - b'a'),
        'A'..='F' => Ok(10 + c as u8 - b'A'),
        _ => Err(CliError::HexBadChar(c)),
    }
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

pub fn run_cli<IO: CliIo>(argv: &[String], io: &mut IO) -> Result<(), CliError> {
    let args = parse_args(argv)?;
    let raw = io.read_input(&args.input)?;
    let att: FinalityAttestation =
        serde_json::from_str(&raw).map_err(|e| CliError::JsonParse(format!("{e}")))?;

    match args.sub {
        Subcommand::Build => {
            let root =
                build_attestation(&att).map_err(|e| CliError::Attestation(format!("{e}")))?;
            io.write_output(&args.output, &hex_encode(&root))?;
            Ok(())
        }
        Subcommand::Verify { root } => {
            verify_attestation(&att, &root).map_err(|e| CliError::Attestation(format!("{e}")))?;
            io.write_output(&args.output, "ok")?;
            Ok(())
        }
    }
}

/// Production I/O.
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

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_finality_attestation::EvaporatedForkWitnessRef;
    use std::collections::HashMap;

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

    fn fwr(b: u8) -> EvaporatedForkWitnessRef {
        let mut fr = [0u8; 32];
        fr[0] = b;
        let mut w = [0u8; 32];
        w[0] = b ^ 0xff;
        EvaporatedForkWitnessRef {
            fork_root: fr,
            witness: w,
        }
    }

    fn good_attestation_json() -> String {
        let att = FinalityAttestation {
            block_hash: [1u8; 32],
            finalised_at_epoch: 1000,
            causal_root: [2u8; 32],
            bell_seed: [3u8; 32],
            evaporated_forks: vec![fwr(0x10), fwr(0x20)],
        };
        serde_json::to_string(&att).unwrap()
    }

    // ── argv parser ──────────────────────────────────────────────

    #[test]
    fn parse_build_minimal() {
        let argv = vec!["build".into(), "--input".into(), "a.json".into()];
        let a = parse_args(&argv).unwrap();
        assert_eq!(a.sub, Subcommand::Build);
        assert_eq!(a.input, "a.json");
        assert_eq!(a.output, "-");
    }

    #[test]
    fn parse_verify_with_root() {
        let argv = vec![
            "verify".into(),
            "--input".into(),
            "a.json".into(),
            "--root".into(),
            "0x".to_string() + &"ab".repeat(32),
        ];
        let a = parse_args(&argv).unwrap();
        match a.sub {
            Subcommand::Verify { root } => assert_eq!(root, [0xabu8; 32]),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_verify_without_0x() {
        let argv = vec![
            "verify".into(),
            "--input".into(),
            "a.json".into(),
            "--root".into(),
            "ab".repeat(32),
        ];
        let a = parse_args(&argv).unwrap();
        match a.sub {
            Subcommand::Verify { root } => assert_eq!(root, [0xabu8; 32]),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_rejects_missing_subcommand() {
        let argv: Vec<String> = vec![];
        assert!(matches!(
            parse_args(&argv).unwrap_err(),
            CliError::MissingSubcommand
        ));
    }

    #[test]
    fn parse_rejects_unknown_subcommand() {
        let argv = vec!["frobnicate".into(), "--input".into(), "a".into()];
        assert!(matches!(
            parse_args(&argv).unwrap_err(),
            CliError::UnknownSubcommand(_)
        ));
    }

    #[test]
    fn parse_rejects_verify_without_root() {
        let argv = vec!["verify".into(), "--input".into(), "a.json".into()];
        assert!(matches!(
            parse_args(&argv).unwrap_err(),
            CliError::MissingArgument("--root")
        ));
    }

    #[test]
    fn parse_rejects_bad_root_length() {
        let argv = vec![
            "verify".into(),
            "--input".into(),
            "a.json".into(),
            "--root".into(),
            "abcd".into(),
        ];
        assert!(matches!(
            parse_args(&argv).unwrap_err(),
            CliError::HexWrongLength { got: 4 }
        ));
    }

    #[test]
    fn parse_rejects_bad_root_hex() {
        let argv = vec![
            "verify".into(),
            "--input".into(),
            "a.json".into(),
            "--root".into(),
            "z".repeat(64),
        ];
        assert!(matches!(
            parse_args(&argv).unwrap_err(),
            CliError::HexBadChar('z')
        ));
    }

    // ── build pipeline ───────────────────────────────────────────

    #[test]
    fn build_writes_64_hex_chars() {
        let mut io = MemIo::new();
        io.inputs.insert("a.json".into(), good_attestation_json());
        run_cli(
            &vec!["build".into(), "--input".into(), "a.json".into()],
            &mut io,
        )
        .unwrap();
        let hex = io.outputs.get("-").expect("stdout default");
        assert_eq!(hex.len(), 64);
        for c in hex.chars() {
            assert!(c.is_ascii_hexdigit() && c.is_ascii_lowercase() || c.is_ascii_digit());
        }
    }

    #[test]
    fn build_to_named_output() {
        let mut io = MemIo::new();
        io.inputs.insert("a.json".into(), good_attestation_json());
        run_cli(
            &vec![
                "build".into(),
                "--input".into(),
                "a.json".into(),
                "--output".into(),
                "out.hex".into(),
            ],
            &mut io,
        )
        .unwrap();
        assert!(io.outputs.contains_key("out.hex"));
    }

    // ── verify pipeline ──────────────────────────────────────────

    #[test]
    fn build_then_verify_round_trip() {
        let mut io = MemIo::new();
        io.inputs.insert("a.json".into(), good_attestation_json());
        // Build
        run_cli(
            &vec![
                "build".into(),
                "--input".into(),
                "a.json".into(),
                "--output".into(),
                "root.hex".into(),
            ],
            &mut io,
        )
        .unwrap();
        let root_hex = io.outputs.remove("root.hex").unwrap();
        // Verify with the produced root
        run_cli(
            &vec![
                "verify".into(),
                "--input".into(),
                "a.json".into(),
                "--root".into(),
                root_hex,
            ],
            &mut io,
        )
        .unwrap();
        assert_eq!(io.outputs.get("-").unwrap(), "ok");
    }

    #[test]
    fn verify_with_wrong_root_fails() {
        let mut io = MemIo::new();
        io.inputs.insert("a.json".into(), good_attestation_json());
        let err = run_cli(
            &vec![
                "verify".into(),
                "--input".into(),
                "a.json".into(),
                "--root".into(),
                "0".repeat(64),
            ],
            &mut io,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::Attestation(_)));
    }

    // ── error surfaces ───────────────────────────────────────────

    #[test]
    fn json_parse_error_surfaces() {
        let mut io = MemIo::new();
        io.inputs.insert("a.json".into(), "not-json".into());
        let err = run_cli(
            &vec!["build".into(), "--input".into(), "a.json".into()],
            &mut io,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::JsonParse(_)));
    }

    #[test]
    fn attestation_unsorted_forks_surfaces() {
        let att = FinalityAttestation {
            block_hash: [1u8; 32],
            finalised_at_epoch: 0,
            causal_root: [2u8; 32],
            bell_seed: [3u8; 32],
            evaporated_forks: vec![fwr(0x20), fwr(0x10)], // unsorted
        };
        let mut io = MemIo::new();
        io.inputs
            .insert("a.json".into(), serde_json::to_string(&att).unwrap());
        let err = run_cli(
            &vec!["build".into(), "--input".into(), "a.json".into()],
            &mut io,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::Attestation(_)));
    }

    #[test]
    fn input_io_error_surfaces() {
        let mut io = MemIo::new();
        let err = run_cli(
            &vec!["build".into(), "--input".into(), "missing.json".into()],
            &mut io,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::Io(_)));
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "The CLI exposes the FinalityAttestation capstone via
        // two subcommands: `build` reads JSON and emits 32-byte hex
        // root; `verify` reads JSON + supplied root and returns
        // ok / typed error. Build → verify is round-trip across same
        // inputs. JSON, attestation, and I/O errors all surface as
        // typed CliError variants; no panics on bad input."

        let mut io = MemIo::new();
        io.inputs.insert("a.json".into(), good_attestation_json());

        // Build
        run_cli(
            &vec![
                "build".into(),
                "--input".into(),
                "a.json".into(),
                "--output".into(),
                "r.hex".into(),
            ],
            &mut io,
        )
        .unwrap();
        let root_hex = io.outputs.remove("r.hex").unwrap();

        // Verify (round-trip)
        run_cli(
            &vec![
                "verify".into(),
                "--input".into(),
                "a.json".into(),
                "--root".into(),
                root_hex,
            ],
            &mut io,
        )
        .unwrap();
        assert_eq!(io.outputs.get("-").unwrap(), "ok");

        // Wrong root → typed Attestation error.
        let err = run_cli(
            &vec![
                "verify".into(),
                "--input".into(),
                "a.json".into(),
                "--root".into(),
                "f".repeat(64),
            ],
            &mut io,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::Attestation(_)));
    }
}
