//! Coverage tests for `HttpTransport` URL building (pure-logic
//! coverage; network-IO paths via `ureq` belong to the e2e harness
//! at tests/e2e_http.rs).
//!
//! Adds tests for:
//!
//!   - `HttpTransport::new` default state
//!   - `DEFAULT_*` const pin (no doctrine drift)
//!   - `header_url` / `latest_header_url` / `state_proof_url` /
//!     `nova_attestation_url` / `nova_vk_bytes_url` placeholder
//!     substitution
//!   - Hex encoding (lowercase, zero-padded, full-length)
//!   - `with_paths` override propagation
//!   - `with_bearer_token` builder semantics
//!   - Edge cases: u64::MAX height, all-zero key, all-ff key,
//!     base URL with trailing slash

use evaporchain_light_client_http::{
    HttpTransport, DEFAULT_HEADER_PATH, DEFAULT_LATEST_HEADER_PATH,
    DEFAULT_NOVA_ATTESTATION_PATH, DEFAULT_NOVA_VK_BYTES_PATH, DEFAULT_STATE_PROOF_PATH,
};

// =================================================================
// Default path constants
// =================================================================

#[test]
fn default_path_constants_pin_chain_api_shape() {
    // These match the chain's /api/... surface; drift would silently
    // break light-client probes against vanilla devnets.
    assert_eq!(DEFAULT_HEADER_PATH, "/api/light_header/{height}");
    assert_eq!(DEFAULT_LATEST_HEADER_PATH, "/api/light_header/latest");
    assert_eq!(DEFAULT_STATE_PROOF_PATH, "/api/state/proof/{key_hex}");
    assert_eq!(DEFAULT_NOVA_ATTESTATION_PATH, "/api/lambda_fold/nova");
    assert_eq!(DEFAULT_NOVA_VK_BYTES_PATH, "/api/lambda_fold/nova/vk_bytes");
}

// =================================================================
// header_url / latest_header_url
// =================================================================

#[test]
fn header_url_substitutes_height_placeholder() {
    let t = HttpTransport::new("http://localhost:8080");
    assert_eq!(
        t.header_url(42),
        "http://localhost:8080/api/light_header/42"
    );
}

#[test]
fn header_url_handles_u64_max_height() {
    let t = HttpTransport::new("http://localhost:8080");
    let url = t.header_url(u64::MAX);
    assert!(url.ends_with(&u64::MAX.to_string()));
    assert!(url.starts_with("http://localhost:8080/api/light_header/"));
}

#[test]
fn header_url_height_zero_is_valid() {
    let t = HttpTransport::new("http://localhost:8080");
    assert_eq!(
        t.header_url(0),
        "http://localhost:8080/api/light_header/0"
    );
}

#[test]
fn latest_header_url_concatenates_base_and_path() {
    let t = HttpTransport::new("http://localhost:8080");
    assert_eq!(
        t.latest_header_url(),
        "http://localhost:8080/api/light_header/latest"
    );
}

// =================================================================
// state_proof_url + hex encoding invariants
// =================================================================

#[test]
fn state_proof_url_encodes_all_zero_key_as_64_zeros() {
    let t = HttpTransport::new("http://h");
    let key = [0u8; 32];
    let url = t.state_proof_url(&key);
    assert_eq!(url, format!("http://h/api/state/proof/{}", "0".repeat(64)));
}

#[test]
fn state_proof_url_encodes_all_ff_key_as_64_f_chars() {
    let t = HttpTransport::new("http://h");
    let key = [0xffu8; 32];
    let url = t.state_proof_url(&key);
    assert!(url.ends_with(&"f".repeat(64)));
}

#[test]
fn state_proof_url_uses_lowercase_hex() {
    let t = HttpTransport::new("http://h");
    let key = [0xABu8; 32];
    let url = t.state_proof_url(&key);
    // Pin lowercase — SDK expects deterministic case for caching.
    assert!(url.contains("ab"), "got: {url}");
    assert!(!url.contains("AB"), "must be lowercase; got: {url}");
}

#[test]
fn state_proof_url_pattern_a_through_f_round_trips() {
    let t = HttpTransport::new("http://h");
    // Walk all nibble values 0..16 across 32 bytes.
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = (i as u8) | ((i as u8) << 4);
    }
    let url = t.state_proof_url(&key);
    // Recover the hex part.
    let prefix = "http://h/api/state/proof/";
    assert!(url.starts_with(prefix));
    let hex = &url[prefix.len()..];
    assert_eq!(hex.len(), 64, "32 bytes → 64 hex chars");
    // Pin a few known bytes manually: i=10 → 0xAA → "aa", i=15 → 0xFF → "ff".
    assert!(hex.contains("aa"));
    assert!(hex.contains("ff"));
}

// =================================================================
// nova_* URLs
// =================================================================

#[test]
fn nova_attestation_url_is_fixed_path() {
    let t = HttpTransport::new("http://h");
    assert_eq!(t.nova_attestation_url(), "http://h/api/lambda_fold/nova");
}

#[test]
fn nova_vk_bytes_url_is_fixed_path() {
    let t = HttpTransport::new("http://h");
    assert_eq!(
        t.nova_vk_bytes_url(),
        "http://h/api/lambda_fold/nova/vk_bytes"
    );
}

// =================================================================
// with_paths overrides
// =================================================================

#[test]
fn with_paths_overrides_all_five_templates() {
    let t = HttpTransport::new("http://h").with_paths(
        "/h/{height}",
        "/l",
        "/sp/{key_hex}",
        "/na",
        "/vk",
    );
    assert_eq!(t.header_url(7), "http://h/h/7");
    assert_eq!(t.latest_header_url(), "http://h/l");
    assert_eq!(t.state_proof_url(&[0u8; 32]), format!("http://h/sp/{}", "0".repeat(64)));
    assert_eq!(t.nova_attestation_url(), "http://h/na");
    assert_eq!(t.nova_vk_bytes_url(), "http://h/vk");
}

#[test]
fn with_paths_template_without_placeholder_is_literal() {
    // If the operator hard-codes a height into the template, the
    // {height} substitution is a no-op (no placeholder to replace).
    // This is permissive: caller takes responsibility.
    let t = HttpTransport::new("http://h").with_paths(
        "/fixed-height-1",
        "/l",
        "/sp/{key_hex}",
        "/na",
        "/vk",
    );
    assert_eq!(t.header_url(99), "http://h/fixed-height-1");
}

// =================================================================
// with_bearer_token + chaining
// =================================================================

#[test]
fn with_bearer_token_is_builder_style() {
    // Chains. We can't observe the token from outside, but the
    // chained methods all return Self so the builder pattern works.
    let _t = HttpTransport::new("http://h")
        .with_bearer_token("tok")
        .with_paths("/h/{height}", "/l", "/sp/{key_hex}", "/na", "/vk");
}

#[test]
fn with_bearer_token_does_not_change_url_building() {
    // Bearer token affects request headers, not URL paths.
    let t_no_tok = HttpTransport::new("http://h");
    let t_tok = HttpTransport::new("http://h").with_bearer_token("secret");
    assert_eq!(t_no_tok.header_url(1), t_tok.header_url(1));
    assert_eq!(
        t_no_tok.state_proof_url(&[0u8; 32]),
        t_tok.state_proof_url(&[0u8; 32])
    );
}

// =================================================================
// Base URL edge cases
// =================================================================

#[test]
fn base_url_with_trailing_slash_double_slashes() {
    // The transport doesn't strip trailing slashes — operator is
    // responsible for passing the correct base. Pin the observable
    // behavior so it doesn't silently change.
    let t = HttpTransport::new("http://h/");
    assert_eq!(t.header_url(1), "http://h//api/light_header/1");
}

#[test]
fn base_url_https_works() {
    let t = HttpTransport::new("https://node.example.com:9443");
    assert!(t.header_url(1).starts_with("https://"));
    assert!(t.latest_header_url().starts_with("https://"));
}

#[test]
fn empty_base_url_does_not_panic() {
    // Pin defensive behavior — empty base produces a path-only URL,
    // which most HTTP clients reject at request time, not at URL
    // construction time. Builder must not panic.
    let t = HttpTransport::new("");
    let _ = t.header_url(1);
    let _ = t.latest_header_url();
    let _ = t.state_proof_url(&[0u8; 32]);
}
