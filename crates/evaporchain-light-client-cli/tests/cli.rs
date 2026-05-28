//! Coverage tests for the `evaporchain-light-client` operator CLI.
//!
//! This is a `[[bin]]`-only crate (no `lib.rs`), so coverage runs by
//! spawning the compiled binary via `std::process::Command` and
//! observing exit code + stdout/stderr. The `CARGO_BIN_EXE_<name>`
//! env var resolves to the test-time binary path.
//!
//! The 3 subcommands (`sync-latest`, `get-state`, `watch`) all need
//! network I/O to run end-to-end. This file covers the arg-parsing
//! + early-return paths that DON'T need network:
//!
//!   - `--help` / `--version` flags
//!   - Missing required args → clap exits with usage error
//!   - Conflicting args (`--key` vs `--account`) → clap rejects
//!   - Subcommand `--help` rendering
//!
//! End-to-end runs against a live node belong to the operator
//! runbook (`docs/runbooks/light-client-cli.md`).

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_evaporchain-light-client"))
}

// =================================================================
// Top-level help + version
// =================================================================

#[test]
fn top_level_help_prints_subcommand_list() {
    let out = bin().arg("--help").output().expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        out.status.success(),
        "--help must exit 0; got {:?}",
        out.status
    );
    // All three subcommands must appear in help output.
    assert!(
        combined.contains("sync-latest"),
        "help must list sync-latest"
    );
    assert!(combined.contains("get-state"), "help must list get-state");
    assert!(combined.contains("watch"), "help must list watch");
}

#[test]
fn top_level_version_prints_semver() {
    let out = bin().arg("--version").output().expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "--version must exit 0");
    // The version line includes the binary name + a version string.
    assert!(
        stdout.contains("evaporchain-light-client"),
        "--version must print binary name; got: {stdout}"
    );
}

#[test]
fn no_args_prints_usage_and_exits_nonzero() {
    let out = bin().output().expect("spawn");
    assert!(!out.status.success(), "no-args must exit non-zero");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Usage") || stderr.contains("subcommand"),
        "stderr must guide the operator; got: {stderr}"
    );
}

#[test]
fn unknown_subcommand_rejected() {
    let out = bin().arg("not-a-subcommand").output().expect("spawn");
    assert!(!out.status.success());
}

// =================================================================
// sync-latest subcommand
// =================================================================

#[test]
fn sync_latest_help_lists_node_and_genesis_height() {
    let out = bin()
        .args(["sync-latest", "--help"])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(combined.contains("--node"));
    assert!(combined.contains("genesis-height") || combined.contains("genesis_height"));
}

#[test]
fn sync_latest_missing_node_arg_rejected() {
    let out = bin().arg("sync-latest").output().expect("spawn");
    assert!(!out.status.success(), "missing required --node must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--node") || stderr.contains("required"),
        "error must mention --node; got: {stderr}"
    );
}

// =================================================================
// get-state subcommand
// =================================================================

#[test]
fn get_state_help_mentions_key_and_account_mutex() {
    let out = bin().args(["get-state", "--help"]).output().expect("spawn");
    assert!(out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(combined.contains("--key"));
    assert!(combined.contains("--account"));
    assert!(combined.contains("--node"));
}

#[test]
fn get_state_with_both_key_and_account_rejected() {
    // Clap's `conflicts_with` should reject this combination at parse
    // time (no network call).
    let out = bin()
        .args([
            "get-state",
            "--node",
            "http://localhost:8080",
            "--key",
            &"00".repeat(32),
            "--account",
            &"01".repeat(32),
        ])
        .output()
        .expect("spawn");
    assert!(
        !out.status.success(),
        "both --key and --account must conflict"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be used") || stderr.contains("conflicts"),
        "clap must report the conflict; got: {stderr}"
    );
}

#[test]
fn get_state_missing_node_arg_rejected() {
    let out = bin()
        .args(["get-state", "--key", &"00".repeat(32)])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
}

// =================================================================
// watch subcommand
// =================================================================

#[test]
fn watch_help_lists_poll_secs() {
    let out = bin().args(["watch", "--help"]).output().expect("spawn");
    assert!(out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(combined.contains("--node"));
    assert!(combined.contains("poll-secs") || combined.contains("poll_secs"));
}

#[test]
fn watch_missing_node_arg_rejected() {
    let out = bin().arg("watch").output().expect("spawn");
    assert!(!out.status.success());
}

// =================================================================
// Cross-cutting
// =================================================================

#[test]
fn help_short_form_h_works_as_long() {
    let out = bin().arg("-h").output().expect("spawn");
    assert!(out.status.success(), "short -h must work like --help");
}

#[test]
fn each_subcommand_supports_help_flag_uniformly() {
    for sub in ["sync-latest", "get-state", "watch"] {
        let out = bin().args([sub, "--help"]).output().expect("spawn");
        assert!(
            out.status.success(),
            "subcommand {sub} must accept --help; got status {:?}",
            out.status
        );
    }
}
