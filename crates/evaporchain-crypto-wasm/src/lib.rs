//! WASM bridge for ML-DSA (Dilithium3) post-quantum cryptography.
//!
//! Exposes key generation, signing, and verification to the browser extension
//! via wasm-bindgen. Uses `pqc_dilithium` in mode3 (NIST security level 3)
//! which is the same `pqc_dilithium` crate used by the EvaporChain node,
//! ensuring byte-level compatibility between browser and node signatures.
//!
//! Key sizes (Dilithium3 / ML-DSA-65):
//! - Public key:  1952 bytes
//! - Secret key:  4000 bytes
//! - Signature:   3293 bytes
//!
//! # Security — Browser Isolation Requirements
//!
//! Secret keys are exposed to JavaScript as `Uint8Array` via `ml_dsa_keygen()`.
//! WASM linear memory is readable by any JS in the same origin. This is safe
//! **only** under the browser extension's isolated execution context:
//!
//! 1. The extension runs in its own origin (`chrome-extension://<id>`), isolated
//!    from web page JS by the Same-Origin Policy.
//! 2. Content scripts run in an isolated world — page JS cannot read extension
//!    memory or call extension APIs.
//! 3. The `manifest.json` `content_security_policy` must forbid `unsafe-eval`
//!    and restrict script sources to `self` only.
//! 4. Secret keys should be stored in `chrome.storage.session` (memory-only,
//!    cleared on browser close) — NOT `chrome.storage.local` (persisted to disk).
//! 5. After signing, the caller should zero the `Uint8Array` holding the secret
//!    key (`secretKey.fill(0)`) to minimize the exposure window.
//!
//! **Do NOT use this module in regular web pages** — any same-origin script
//! could read the WASM linear memory and extract secret keys.

use pqc_dilithium::Keypair;
use sha2::{Digest, Sha256};
use wasm_bindgen::prelude::*;
use zeroize::Zeroize;

// Compile-time layout sanity checks — `reconstruct_keypair` below
// relies on `Keypair`'s in-memory layout being exactly
// `{ public: [u8; PUBLICKEYBYTES], secret: [u8; SECRETKEYBYTES] }`
// with `public` at byte offset 0 and no padding. The upstream
// `pqc_dilithium 0.2.0` struct (api.rs:5-8) is `#[derive(Copy, Clone,
// ...)]` over two fixed-size byte arrays. Critically, the struct is
// **not `#[repr(C)]`**, so under Rust's default `repr(Rust)` the
// compiler is technically free to reorder fields and add padding.
// In practice today every Rust compiler lays out plain-byte-array
// structs in declaration order with no padding, but we pin that
// behaviour explicitly with two checks rather than rely on it
// implicitly.
//
// (1) Size invariant — rules out padding. With two `[u8; N]` fields
//     of total declared size `PK + SK`, any padding would push
//     `size_of` above that bound.
// (2) `public` at offset 0 — rules out a hypothetical future
//     compiler / upstream-rewrite that swapped the field order.
//     If `secret` came first, our `copy_nonoverlapping(.., kp_ptr.add(PK), SK)`
//     would clobber the `public` slot and the unrelated trailing
//     bytes — silent memory corruption.
//
// We can NOT check `offset_of!(Keypair, secret)` from this crate
// because `secret` is a private field upstream — the privacy check
// fires in 1.77+. The `(1) + (2)` pair is sufficient: with no padding
// and `public` at offset 0, the only place `secret` can be is at
// offset `PUBLICKEYBYTES`.
const _ASSERT_KEYPAIR_SIZE: () = {
    if std::mem::size_of::<Keypair>()
        != pqc_dilithium::PUBLICKEYBYTES + pqc_dilithium::SECRETKEYBYTES
    {
        panic!("pqc_dilithium::Keypair size drifted — reconstruct_keypair's offset math is no longer valid; pin or audit before continuing");
    }
};

const _ASSERT_KEYPAIR_PUBLIC_AT_ORIGIN: () = {
    if std::mem::offset_of!(Keypair, public) != 0 {
        panic!("pqc_dilithium::Keypair field order drifted — `public` is no longer at offset 0; reconstruct_keypair would clobber the wrong slot");
    }
};

// Legacy name kept for any out-of-tree code that referenced it; both
// new asserts above were previously bundled under this single name.
#[allow(dead_code)]
const _ASSERT_KEYPAIR_LAYOUT: () = {
    let _ = _ASSERT_KEYPAIR_SIZE;
    let _ = _ASSERT_KEYPAIR_PUBLIC_AT_ORIGIN;
};

/// Generate a new ML-DSA keypair.
///
/// Returns a JS object `{ publicKey: Uint8Array, secretKey: Uint8Array }`.
#[wasm_bindgen(js_name = "mlDsaKeygen")]
pub fn ml_dsa_keygen() -> Result<JsValue, JsValue> {
    let kp = Keypair::generate();

    let obj = js_sys::Object::new();
    js_sys::Reflect::set(
        &obj,
        &JsValue::from_str("publicKey"),
        &js_sys::Uint8Array::from(&kp.public[..]).into(),
    )?;
    js_sys::Reflect::set(
        &obj,
        &JsValue::from_str("secretKey"),
        &js_sys::Uint8Array::from(kp.expose_secret()).into(),
    )?;

    Ok(obj.into())
}

/// Sign a message with an ML-DSA secret key.
///
/// Accepts the full secret key bytes and message.
/// Returns the raw signature bytes (3293 bytes for Dilithium3).
///
/// The reconstructed `Keypair` is wrapped in a [`ZeroizingKeypair`]
/// guard so the secret-key bytes inside the `Copy` struct's stack
/// slot are explicitly overwritten with zeros before the binding
/// drops, narrowing the in-memory exposure window beyond what the
/// JS-side `secretKey.fill(0)` discipline can achieve alone. The
/// wrapper's `Drop` runs on every exit path — normal return, `?`
/// short-circuit, or panic unwind — so signature material does not
/// linger in any code path.
#[wasm_bindgen(js_name = "mlDsaSign")]
pub fn ml_dsa_sign(secret_key: &[u8], message: &[u8]) -> Result<Vec<u8>, JsValue> {
    let kp = reconstruct_keypair(secret_key).map_err(|e| JsValue::from_str(&e))?;
    let sig = kp.sign(message);
    // `kp` drops here; ZeroizingKeypair::drop zeroes the underlying
    // Keypair byte-slot on every exit path including panic unwinds.
    Ok(sig.to_vec())
}

/// Verify an ML-DSA signature.
///
/// Returns `true` if the signature is valid for the given message and public key.
#[wasm_bindgen(js_name = "mlDsaVerify")]
pub fn ml_dsa_verify(message: &[u8], signature: &[u8], public_key: &[u8]) -> bool {
    pqc_dilithium::verify(signature, message, public_key).is_ok()
}

/// Derive an EvaporChain address from a public key.
///
/// address = "0x" + hex(SHA-256(publicKey))
#[wasm_bindgen(js_name = "deriveAddress")]
pub fn derive_address(public_key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(public_key);
    let hash = hasher.finalize();
    let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
    format!("0x{}", hex)
}

/// Reconstruct a `Keypair` from raw secret-key bytes.
///
/// `pqc_dilithium 0.2.0`'s public API has no constructor that
/// takes secret bytes — `Keypair::generate` returns a fresh
/// keypair and the `secret` field is private. The closest public
/// surface is `crypto_sign_signature(sig, msg, sk)` from
/// `pqc_dilithium::sign`, but that's only re-exported under the
/// `cfg(dilithium_kat)` flag, not available in the default build.
///
/// We therefore use a layout-dependent unsafe path, hardened with
/// **three** layout guards plus an explicit-zeroize Drop discipline at
/// the call site:
///
/// 1. **Compile-time size assertion** at module top —
///    `_ASSERT_KEYPAIR_SIZE` pins
///    `size_of::<Keypair>() == PUBLICKEYBYTES + SECRETKEYBYTES`.
///    Rules out any future upstream change that adds padding (silent
///    memory corruption → build failure).
/// 2. **Compile-time field-offset assertion** at module top —
///    `_ASSERT_KEYPAIR_PUBLIC_AT_ORIGIN` pins
///    `offset_of!(Keypair, public) == 0`. Rules out a hypothetical
///    upstream reorder that puts `secret` first; together with (1)
///    this implies `secret` is at offset `PUBLICKEYBYTES`.
/// 3. **Length-check** — secret-key byte slice must be exactly
///    `SECRETKEYBYTES`.
///
/// The `pqc_dilithium 0.2.0` `Keypair` struct is `#[derive(Copy, ...)]`
/// over two `[u8; N]` fields with NO `#[repr(C)]`. Rust's default
/// `repr(Rust)` is technically free to reorder fields, but in
/// practice every Rust compiler lays plain-byte-array structs out in
/// declaration order with no padding. Guards (1) and (2) pin that
/// implicit behaviour explicitly — any compiler upgrade or upstream
/// rewrite that broke either invariant trips the build before this
/// unsafe block runs.
///
/// **Why not call the upstream `crypto_sign_signature` directly?**
/// That function (in `pqc_dilithium::sign`) takes an `sk: &[u8]`
/// argument and would let us avoid the layout-dependent overwrite
/// entirely. But it's only `pub use`'d from the crate root under
/// `#[cfg(dilithium_kat)]` — a build-time cfg flag, not a Cargo
/// feature — so we can't enable it from downstream without
/// global `RUSTFLAGS` (which would affect every other crate too) or
/// vendoring/forking `pqc_dilithium`. Both alternatives were
/// considered and rejected as more invasive than the hardened
/// unsafe block.
///
/// The reconstructed keypair's `public` field is whatever the
/// throwaway `Keypair::generate()` produced — it does NOT match
/// the caller's secret. This is fine for our use case
/// (`ml_dsa_sign`) because Dilithium signing only reads from the
/// secret slot; the public slot is touched only by `verify`,
/// which the caller does separately with the correct public key.
/// A self-verify probe inside this function would always fail
/// (mismatched throwaway public + caller secret) and was
/// therefore removed in 2026-05-06's CRITICAL-1 hardening pass —
/// it would force the caller to supply a public key, defeating
/// the API simplicity that motivates this helper in the first
/// place.
///
/// Defence in depth: callers `mlDsaSign` invokes
/// [`zeroize_keypair`] on the returned keypair immediately after
/// `kp.sign` returns, so the secret bytes do not linger in the
/// stack slot beyond the sign call.
fn reconstruct_keypair(sk_bytes: &[u8]) -> Result<ZeroizingKeypair, String> {
    use pqc_dilithium::SECRETKEYBYTES;
    if sk_bytes.len() != SECRETKEYBYTES {
        return Err(format!(
            "Invalid secret key length: expected {}, got {}",
            SECRETKEYBYTES,
            sk_bytes.len()
        ));
    }

    // Generate a throwaway keypair as the layout host. Dilithium
    // signing only reads from the secret slot; overwriting the
    // secret slot with caller bytes is the layout-dependent step
    // guarded by the compile-time size assertion at module top.
    let mut kp = Keypair::generate();

    // SAFETY: layout invariants
    //   (a) `size_of::<Keypair>() == PK + SK` — _ASSERT_KEYPAIR_SIZE
    //       (rules out padding)
    //   (b) `offset_of!(Keypair, public) == 0` —
    //       _ASSERT_KEYPAIR_PUBLIC_AT_ORIGIN (rules out field swap)
    //   (a) + (b) together imply `secret` lives at byte offset PK
    //       and the write below lands in the `secret` slot.
    //
    // The caller-supplied bytes are exactly `SECRETKEYBYTES` per the
    // length check immediately above. `copy_nonoverlapping` is sound
    // for distinct allocations (caller-supplied slice vs our stack
    // `kp`); the source and destination cannot overlap because `kp`
    // is freshly stack-allocated and `sk_bytes` is the caller's
    // borrow.
    unsafe {
        let kp_ptr = &mut kp as *mut Keypair as *mut u8;
        let sk_offset = pqc_dilithium::PUBLICKEYBYTES;
        std::ptr::copy_nonoverlapping(sk_bytes.as_ptr(), kp_ptr.add(sk_offset), SECRETKEYBYTES);
    }

    Ok(ZeroizingKeypair(kp))
}

/// RAII guard around `Keypair`: zeroes the entire struct's byte slot
/// on drop. The upstream `Keypair` derives `Copy` over plain byte
/// arrays with no `Drop` of its own, so without this wrapper the
/// secret key bytes sit unzeroed in the stack slot until something
/// else writes over them — and a panic unwind through `ml_dsa_sign`
/// would never zero them at all.
///
/// The wrapper exposes the inner `Keypair` via `Deref`/`DerefMut`
/// so call sites can use `kp.sign(msg)` unchanged.
///
/// SAFETY: drop's byte-wide overwrite relies on the
/// `_ASSERT_KEYPAIR_LAYOUT` compile-time check that
/// `size_of::<Keypair>() == PUBLICKEYBYTES + SECRETKEYBYTES`. A
/// future upstream change that adds padding or non-`u8` fields
/// would trip the build before reaching this code.
struct ZeroizingKeypair(Keypair);

impl std::ops::Deref for ZeroizingKeypair {
    type Target = Keypair;
    fn deref(&self) -> &Keypair {
        &self.0
    }
}

impl std::ops::DerefMut for ZeroizingKeypair {
    fn deref_mut(&mut self) -> &mut Keypair {
        &mut self.0
    }
}

impl Drop for ZeroizingKeypair {
    fn drop(&mut self) {
        zeroize_keypair(&mut self.0);
    }
}

/// Zero the in-memory bytes of a `Keypair`. Backing implementation
/// for `ZeroizingKeypair`'s `Drop`; also kept as a standalone helper
/// for tests that want to observe the post-zeroize byte state
/// directly. SAFETY: relies on the same compile-time layout
/// assertion as the unsafe write in `reconstruct_keypair` —
/// `Keypair` is exactly `PUBLICKEYBYTES + SECRETKEYBYTES` contiguous
/// bytes with no `Drop` of its own.
fn zeroize_keypair(kp: &mut Keypair) {
    let total = pqc_dilithium::PUBLICKEYBYTES + pqc_dilithium::SECRETKEYBYTES;
    // SAFETY: layout invariants pinned by `_ASSERT_KEYPAIR_SIZE` and
    // `_ASSERT_KEYPAIR_PUBLIC_AT_ORIGIN` at module top. With `public`
    // at offset 0, no padding, and `Keypair` consisting entirely of
    // two `[u8; N]` fields, the whole struct is `total` contiguous
    // bytes that we can re-borrow as a `&mut [u8]` slice. `Keypair`
    // has no `Drop` so byte-wide overwrite cannot violate any
    // invariant; it's plain-old-data.
    let bytes: &mut [u8] =
        unsafe { std::slice::from_raw_parts_mut(kp as *mut Keypair as *mut u8, total) };
    bytes.zeroize();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keygen_sign_verify() {
        let kp = Keypair::generate();
        let msg = b"hello evaporchain";
        let sig = kp.sign(msg);
        assert!(pqc_dilithium::verify(&sig, msg, &kp.public).is_ok());
    }

    #[test]
    fn test_wrong_message_rejects() {
        let kp = Keypair::generate();
        let sig = kp.sign(b"message A");
        assert!(pqc_dilithium::verify(&sig, b"message B", &kp.public).is_err());
    }

    #[test]
    fn test_reconstruct_and_sign() {
        let kp = Keypair::generate();
        let sk = kp.expose_secret().to_vec();
        let msg = b"reconstruct test";

        let kp2 = reconstruct_keypair(&sk).unwrap();
        let sig = kp2.sign(msg);
        assert!(pqc_dilithium::verify(&sig, msg, &kp.public).is_ok());
    }

    #[test]
    fn test_address_derivation() {
        let kp = Keypair::generate();
        let addr = derive_address(&kp.public);
        assert!(addr.starts_with("0x"));
        assert_eq!(addr.len(), 66); // "0x" + 64 hex chars
    }

    #[test]
    fn test_invalid_sk_length() {
        assert!(reconstruct_keypair(&[0u8; 10]).is_err());
    }

    /// CRITICAL-1 finish — empirically pin the layout invariants the
    /// `unsafe` block in `reconstruct_keypair` depends on. The
    /// `_ASSERT_KEYPAIR_SIZE` and `_ASSERT_KEYPAIR_PUBLIC_AT_ORIGIN`
    /// constants at module top catch this at compile time; this test
    /// catches the same regression at run time AND demonstrates the
    /// observable behaviour for anyone reading the test suite.
    ///
    /// If pqc_dilithium ever bumps to a version that reorders fields
    /// or adds padding, both the compile-time assert AND this test
    /// fire — making the layout dependency loudly visible from two
    /// different signal sources.
    #[test]
    fn keypair_layout_invariants_hold_at_runtime() {
        // Invariant 1: total struct size matches PK + SK exactly
        // (no padding, no extra fields).
        assert_eq!(
            std::mem::size_of::<Keypair>(),
            pqc_dilithium::PUBLICKEYBYTES + pqc_dilithium::SECRETKEYBYTES,
            "Keypair size must equal PK + SK — no padding, no extras"
        );

        // Invariant 2: `public` is at offset 0. We can't directly
        // check `offset_of!(Keypair, secret)` because `secret` is a
        // private field upstream (1.77+ privacy check fires), but
        // invariants (1) + (2) together imply `secret` is at offset
        // PUBLICKEYBYTES.
        assert_eq!(
            std::mem::offset_of!(Keypair, public),
            0,
            "Keypair.public must be at offset 0 — `reconstruct_keypair`'s \
             `kp_ptr.add(PUBLICKEYBYTES)` write assumes this and would \
             clobber the wrong slot if `secret` came first"
        );

        // Invariant 3 (observable): after generating a real Keypair,
        // reading the first PUBLICKEYBYTES bytes via raw pointer
        // produces exactly the `public` field's bytes. This proves
        // the layout assumption is correct in practice, not just per
        // the type system.
        let kp = Keypair::generate();
        let raw_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                &kp as *const Keypair as *const u8,
                std::mem::size_of::<Keypair>(),
            )
        };
        assert_eq!(
            &raw_bytes[..pqc_dilithium::PUBLICKEYBYTES],
            &kp.public[..],
            "first PK bytes of the struct must equal Keypair.public — \
             empirical confirmation of offset 0"
        );
    }

    /// AUDIT_2026_05_06.md CRITICAL-1 hardening — round-trip
    /// behaviour: a legitimate generated SK reconstructs cleanly
    /// and produces signatures that verify against the original
    /// keypair's public key. (The reconstructed keypair's own
    /// `public` field is the throwaway's, not the caller's; the
    /// caller is expected to track the matching public key out
    /// of band, which is the API contract.)
    #[test]
    fn reconstruct_round_trip_signs_and_verifies_with_original_pubkey() {
        let original = Keypair::generate();
        let sk_bytes = original.expose_secret().to_vec();
        let rebuilt = reconstruct_keypair(&sk_bytes).expect("legitimate SK must reconstruct");
        let msg = b"reconstruct round trip - CRITICAL-1 regression guard";
        let sig = rebuilt.sign(msg);
        assert!(
            pqc_dilithium::verify(&sig, msg, &original.public).is_ok(),
            "signature from reconstructed kp must verify against original pubkey"
        );
    }

    /// CRITICAL-1 hardening — wrong-length SK is the only
    /// fast-fail path the helper enforces. Garbage of the correct
    /// length produces a keypair whose signatures won't match
    /// any real public key, but the helper itself doesn't know
    /// the caller's intended pubkey, so this is the right
    /// boundary for the helper and the wrong layer to detect
    /// garbage at.
    #[test]
    fn reconstruct_rejects_short_sk() {
        let result = reconstruct_keypair(&[0u8; 10]);
        assert!(result.is_err());
        if let Err(msg) = result {
            assert!(msg.contains("Invalid secret key length"), "got: {msg}");
        }
    }

    /// CRITICAL-1 hardening — `zeroize_keypair` actually clears the
    /// underlying byte slot. Confirms the helper does what the
    /// `ml_dsa_sign` Drop-equivalent path needs.
    #[test]
    fn zeroize_keypair_actually_zeros_bytes() {
        let mut kp = Keypair::generate();
        zeroize_keypair(&mut kp);
        // Public field is one of the two contiguous byte arrays;
        // zeroize_keypair zeroes the whole struct, so kp.public
        // should be all zeros.
        assert!(
            kp.public.iter().all(|&b| b == 0),
            "zeroize_keypair must zero the public field too"
        );
        // Secret field is private but we can sign with it — a
        // zeroed secret produces a deterministic-looking but
        // useless signature; the meaningful check is that the
        // public bytes are zero (any future layout change would
        // make this assertion catch a regression).
    }

    /// CRITICAL-1 hardening — `ZeroizingKeypair`'s `Drop` zeroes the
    /// underlying `Keypair` byte slot on every exit path. The audit's
    /// concern was that the inline `zeroize_keypair()` call in
    /// `ml_dsa_sign` wouldn't fire on a panic unwind through `sign`,
    /// leaving secret bytes in the stack slot. The wrapper closes
    /// that gap: drop runs unconditionally regardless of how the
    /// scope exits.
    #[test]
    fn zeroizing_keypair_drop_zeroes_underlying_bytes() {
        let original_pub: [u8; pqc_dilithium::PUBLICKEYBYTES] = {
            let zk = reconstruct_keypair(Keypair::generate().expose_secret())
                .expect("legit SK reconstructs");
            zk.public
        };
        // After zk drops above, its stack slot should be reused or
        // overwritten — but more importantly, the Drop impl
        // ran. Witness Drop directly via a ManuallyDrop-style
        // observation: build one, run drop_in_place explicitly, and
        // verify the inner Keypair's bytes are zero.
        let mut zk = reconstruct_keypair(Keypair::generate().expose_secret())
            .expect("legit SK reconstructs");
        // Borrow and drop in place to observe the post-drop state.
        // SAFETY: zk is not used after the explicit drop below.
        unsafe {
            std::ptr::drop_in_place(&mut zk as *mut ZeroizingKeypair);
        }
        // After Drop ran, the inner Keypair's public slot is all zero.
        assert!(
            zk.0.public.iter().all(|&b| b == 0),
            "ZeroizingKeypair::drop must zero the inner Keypair's public slot \
             (the secret slot is private but the same Drop covers both)"
        );
        // Prevent the compiler from running Drop again at scope end —
        // it's already been run via drop_in_place.
        std::mem::forget(zk);
        // Sanity: the snapshot we took before any drop ran was non-zero
        // (i.e. `Keypair::generate()` produces real public-key entropy)
        // — so the `all-zeroes` assertion above is meaningful.
        assert!(
            !original_pub.iter().all(|&b| b == 0),
            "Keypair::generate() must produce non-zero public bytes; if this \
             fires, the test setup is degenerate and the post-drop check is \
             trivially true"
        );
    }

    /// CRITICAL-1 hardening — Drop is panic-safe: a panic inside the
    /// `ml_dsa_sign` body still triggers ZeroizingKeypair's Drop on
    /// stack unwind. We can't easily intercept the keypair's bytes
    /// post-unwind from outside `catch_unwind`, but we can verify
    /// the unwind path runs Drop by counting drops via a flag.
    #[test]
    fn zeroizing_keypair_drop_runs_on_panic_unwind() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static DROPPED: AtomicBool = AtomicBool::new(false);

        // Local newtype with a Drop that flips DROPPED — proves the
        // unwind path runs scope drops in general. (Testing
        // ZeroizingKeypair's Drop fires under unwind requires
        // upstream Keypair instrumentation we don't have, so we
        // assert the language guarantee instead, which is what the
        // audit fix actually relies on.)
        struct PanicWitness;
        impl Drop for PanicWitness {
            fn drop(&mut self) {
                DROPPED.store(true, Ordering::SeqCst);
            }
        }

        let result = std::panic::catch_unwind(|| {
            let _guard = PanicWitness;
            // Induce a panic from inside scope.
            panic!("induced panic for unwind test");
        });
        assert!(result.is_err(), "panic must propagate to catch_unwind");
        assert!(
            DROPPED.load(Ordering::SeqCst),
            "drop must run during stack unwind — this is the language \
             guarantee that ZeroizingKeypair relies on"
        );
    }

    /// CRITICAL-1 hardening — compile-time layout assertion is
    /// already evaluated at compile time; this test just witnesses
    /// the runtime equivalence so the assertion's invariant is
    /// readable from the test report.
    #[test]
    fn keypair_layout_matches_byte_arrays() {
        use pqc_dilithium::{PUBLICKEYBYTES, SECRETKEYBYTES};
        assert_eq!(
            std::mem::size_of::<Keypair>(),
            PUBLICKEYBYTES + SECRETKEYBYTES,
            "Keypair must be exactly two contiguous byte arrays — \
             reconstruct_keypair's offset math depends on it"
        );
    }
}
