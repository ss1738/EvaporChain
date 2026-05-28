//! End-to-end HTTP integration test for the Light Client SDK.
//!
//! Spins up a tiny stdlib-only HTTP server in a thread, points
//! [`HttpTransport`] at it, and drives the SDK's state-query path
//! through the full HTTP + JSON round-trip + chain-authoritative
//! [`EnergyVerkleTrie::verify`] cryptographic check.
//!
//! ## What this test covers
//!
//! 1. URL template substitution for the state-proof path.
//! 2. HTTP GET request from the SDK's `ureq`-backed transport.
//! 3. JSON deserialisation of [`EnergyVerkleProof`] on the wire.
//! 4. SDK's `verify_state` Pasta-curve Pedersen commitment check
//!    against a trusted `state_root`.
//!
//! ## What this test does NOT cover
//!
//! - BFT verification path (would require valid BLS-aggregate sigs
//!   over a real validator set). Handled by unit tests in the
//!   parent crate's `client::tests` module with the in-process
//!   [`crate::test_fixtures`].
//! - Nova-IVC verification (would require running the actual SNARK
//!   prover). Handled by unit tests in the parent crate's
//!   `nova::tests` and end-to-end in the chain's own test suite.
//!
//! Sufficient e2e coverage for the HTTP transport's plumbing — the
//! BFT and Nova layers have their own dedicated test paths.

use evaporchain_consensus::light_client::LightBlockHeader;
use evaporchain_consensus::validator_set::ValidatorSet;
use evaporchain_crypto::energy_verkle::EnergyVerkleTrie;
use evaporchain_light_client::LightClient;
use evaporchain_light_client_http::HttpTransport;
use evaporchain_types::CommitCertificate;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

/// Simple route map: path → JSON body. The test server reads the
/// HTTP request line, looks up the path, and returns the body. Any
/// other path returns 404.
type RouteMap = Mutex<std::collections::HashMap<String, String>>;

/// Spawn a tiny HTTP server on a random localhost port. Returns
/// the base URL ("http://127.0.0.1:NNNN") and a handle to the
/// route map (so callers can register more routes after the
/// server starts). The server stays alive for the test's
/// lifetime via the JoinHandle being detached.
fn start_test_server(routes: std::collections::HashMap<String, String>) -> String {
    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test server should bind to a random port");
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);
    let routes: &'static RouteMap = Box::leak(Box::new(Mutex::new(routes)));

    thread::spawn(move || loop {
        let (mut stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));

        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            continue;
        }

        // Drain headers (until empty line) so the client doesn't
        // see a hang.
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                break;
            }
            if line == "\r\n" || line.is_empty() {
                break;
            }
        }

        // Parse `GET /path HTTP/1.1`.
        let parts: Vec<&str> = request_line.split_whitespace().collect();
        let path = parts.get(1).copied().unwrap_or("/");

        let response = match routes.lock().unwrap().get(path) {
            Some(body) => format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            None => {
                let body = "{\"error\":\"not found\"}";
                format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                )
            }
        };
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    });

    base_url
}

/// Build a synthetic LightBlockHeader anchored at the given
/// state_root. We don't drive BFT verification here, so the
/// commit_certificate / validator_set fields are stubbed —
/// `verify_state` only consumes `current_state_root()`.
fn synthetic_header_with_root(state_root: [u8; 32]) -> LightBlockHeader {
    LightBlockHeader {
        height: 1,
        epoch: 0,
        block_hash: [0xaa; 32],
        parent_hash: [0u8; 32],
        state_root,
        timestamp: 1_700_000_000,
        validator_set: ValidatorSet::default(),
        commit_certificate: CommitCertificate {
            height: 1,
            round: 0,
            block_hash: [0xaa; 32],
            aggregate_signature: Vec::new(),
            signer_ids: Vec::new(),
        },
    }
}

#[test]
fn e2e_state_query_round_trip_via_http() {
    // 1. Build an EnergyVerkleTrie locally with a known (key, value)
    //    pair. The trie's root becomes the trusted state_root in
    //    the synthetic LightClient.
    let mut trie = EnergyVerkleTrie::new();
    let key = [7u8; 32];
    let value = [42u8; 32];
    trie.insert(
        key, value, /* energy */ 0, /* half_life */ 0, /* epoch */ 0,
    );
    let state_root = trie.root();

    // 2. Generate a real EnergyVerkleProof and serialize it to
    //    JSON — this is exactly what the chain's
    //    /api/state/proof/:key_hex endpoint will produce.
    let proof = trie.prove(&key);
    let proof_json = serde_json::to_string(&proof).expect("proof must serialize");

    // 3. Build the route map. The path matches what
    //    HttpTransport::default_paths produces:
    //    /api/state/proof/{hex(key)}
    let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();
    let mut routes = std::collections::HashMap::new();
    routes.insert(format!("/api/state/proof/{}", key_hex), proof_json);

    // 4. Start the test HTTP server and point HttpTransport at it.
    let base_url = start_test_server(routes);
    let transport = HttpTransport::new(base_url);

    // 5. Construct the LightClient with the trie's root as the
    //    trusted state_root.
    let header = synthetic_header_with_root(state_root);
    let lc = LightClient::new(header, 1_700_000_000, "", /* vk_bytes */ None);

    // 6. Drive the SDK's HTTP-backed state-query: fetch + verify
    //    in one call. Asserts the proof verifies cryptographically
    //    against the trusted state_root.
    let returned = lc
        .fetch_and_verify_state(&transport, &key, Some(value))
        .expect("e2e state-query round-trip must succeed");
    assert_eq!(returned, Some(value));
}

#[test]
fn e2e_state_query_404_on_missing_proof() {
    // Empty route map → server returns 404 for all paths.
    let routes = std::collections::HashMap::new();
    let base_url = start_test_server(routes);
    let transport = HttpTransport::new(base_url);

    // Genesis with arbitrary state_root — doesn't matter, the
    // request never gets to the verify_state step.
    let header = synthetic_header_with_root([0xff; 32]);
    let lc = LightClient::new(header, 1_700_000_000, "", None);

    let key = [9u8; 32];
    let result = lc.fetch_and_verify_state(&transport, &key, Some([0u8; 32]));
    assert!(result.is_err(), "404 from server must surface as an error");
}

#[test]
fn e2e_state_query_value_mismatch_rejects() {
    // Same setup as the round-trip test, but the caller asks for
    // the WRONG expected value. The proof verifies cryptographically
    // (the trie really maps key→value), but the value match fails.
    let mut trie = EnergyVerkleTrie::new();
    let key = [3u8; 32];
    let stored = [11u8; 32];
    let wrong = [22u8; 32];
    trie.insert(key, stored, 0, 0, 0);
    let state_root = trie.root();
    let proof = trie.prove(&key);
    let proof_json = serde_json::to_string(&proof).unwrap();

    let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();
    let mut routes = std::collections::HashMap::new();
    routes.insert(format!("/api/state/proof/{}", key_hex), proof_json);

    let base_url = start_test_server(routes);
    let transport = HttpTransport::new(base_url);

    let header = synthetic_header_with_root(state_root);
    let lc = LightClient::new(header, 1_700_000_000, "", None);

    let result = lc.fetch_and_verify_state(&transport, &key, Some(wrong));
    assert!(
        result.is_err(),
        "value-mismatch must be rejected even when proof verifies"
    );
}

#[test]
fn e2e_url_building_against_real_listener() {
    // Sanity check that HttpTransport's URL-builder methods
    // produce paths the test server can actually respond to.
    let routes = std::collections::HashMap::new();
    let base_url = start_test_server(routes);
    let transport = HttpTransport::new(&base_url);

    // The URL builders should use the same base + paths the
    // server expects.
    assert_eq!(
        transport.header_url(42),
        format!("{}/api/light_header/42", base_url)
    );
    assert_eq!(
        transport.latest_header_url(),
        format!("{}/api/light_header/latest", base_url)
    );
    let key = [0xabu8; 32];
    let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();
    assert_eq!(
        transport.state_proof_url(&key),
        format!("{}/api/state/proof/{}", base_url, key_hex)
    );
}
