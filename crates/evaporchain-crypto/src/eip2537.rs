/// EIP-2537 point encoding helpers.
///
/// EIP-2537 pads each 48-byte BLS12-381 field element to 64 bytes
/// (16 zero bytes on the left). G1 → 128 bytes; G2 → 256 bytes.
///
/// Both helpers accept the raw ZCash-serialised uncompressed bytes
/// produced by blst's `serialize()`:
///   G1: `[X || Y]`             (each 48 bytes, 96 total)
///   G2: `[X.c1 || X.c0 || Y.c1 || Y.c0]`  (each 48 bytes, 192 total)
///
/// Mapping to EIP-2537 layout:
///   G2[0..64]   = X.c0 (right-justified in 64 bytes)
///   G2[64..128] = X.c1
///   G2[128..192]= Y.c0
///   G2[192..256]= Y.c1
///
/// Encode a raw uncompressed G1 point (96 bytes) into EIP-2537 format (128 bytes).
///
/// Returns `None` if `raw` is not exactly 96 bytes.
pub fn g1_raw_to_eip2537(raw: &[u8]) -> Option<[u8; 128]> {
    if raw.len() != 96 {
        return None;
    }
    let mut out = [0u8; 128];
    out[16..64].copy_from_slice(&raw[0..48]);  // X
    out[80..128].copy_from_slice(&raw[48..96]); // Y
    Some(out)
}

/// Encode a raw uncompressed G2 point (192 bytes, ZCash order) into EIP-2537
/// format (256 bytes).
///
/// ZCash/blst layout: `[X.c1 || X.c0 || Y.c1 || Y.c0]`
/// EIP-2537 layout:   `[X.c0 (pad64) || X.c1 (pad64) || Y.c0 (pad64) || Y.c1 (pad64)]`
///
/// Returns `None` if `raw` is not exactly 192 bytes.
pub fn g2_raw_to_eip2537(raw: &[u8]) -> Option<[u8; 256]> {
    if raw.len() != 192 {
        return None;
    }
    let x_c1 = &raw[0..48];
    let x_c0 = &raw[48..96];
    let y_c1 = &raw[96..144];
    let y_c0 = &raw[144..192];
    let mut out = [0u8; 256];
    out[16..64].copy_from_slice(x_c0);
    out[80..128].copy_from_slice(x_c1);
    out[144..192].copy_from_slice(y_c0);
    out[208..256].copy_from_slice(y_c1);
    Some(out)
}

/// Decompress a BLS12-381 G1 public key (48 bytes compressed) and encode
/// it in EIP-2537 format (128 bytes).
///
/// Only available with the `bls-native` feature (requires blst).
/// Returns `None` on invalid input.
#[cfg(feature = "bls-native")]
pub fn g1_compressed_to_eip2537(compressed: &[u8]) -> Option<[u8; 128]> {
    let pk = blst::min_pk::PublicKey::from_bytes(compressed).ok()?;
    let raw = pk.serialize();
    g1_raw_to_eip2537(&raw)
}

/// Decompress a BLS12-381 G2 aggregate signature (96 bytes compressed) and
/// encode it in EIP-2537 format (256 bytes).
///
/// Only available with the `bls-native` feature (requires blst).
/// Returns `None` on invalid input.
#[cfg(feature = "bls-native")]
pub fn g2_compressed_to_eip2537(compressed: &[u8]) -> Option<[u8; 256]> {
    let sig = blst::min_pk::Signature::from_bytes(compressed).ok()?;
    let raw = sig.serialize();
    g2_raw_to_eip2537(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g1_raw_to_eip2537_rejects_wrong_len() {
        assert!(g1_raw_to_eip2537(&[0u8; 48]).is_none());
        assert!(g1_raw_to_eip2537(&[0u8; 128]).is_none());
    }

    #[test]
    fn g2_raw_to_eip2537_rejects_wrong_len() {
        assert!(g2_raw_to_eip2537(&[0u8; 96]).is_none());
        assert!(g2_raw_to_eip2537(&[0u8; 256]).is_none());
    }

    #[test]
    fn g1_raw_to_eip2537_pads_correctly() {
        let mut raw = [0u8; 96];
        raw[0] = 0xAB; // X first byte
        raw[48] = 0xCD; // Y first byte
        let enc = g1_raw_to_eip2537(&raw).unwrap();
        // X lands at [16..64]; bytes [0..16] must be zero padding
        assert_eq!(enc[0..16], [0u8; 16]);
        assert_eq!(enc[16], 0xAB);
        // Y lands at [80..128]; bytes [64..80] must be zero padding
        assert_eq!(enc[64..80], [0u8; 16]);
        assert_eq!(enc[80], 0xCD);
    }

    #[test]
    fn g2_raw_to_eip2537_routes_coordinates_correctly() {
        let mut raw = [0u8; 192];
        raw[0] = 0x11;   // X.c1 first byte → EIP2537 [80]
        raw[48] = 0x22;  // X.c0 first byte → EIP2537 [16]
        raw[96] = 0x33;  // Y.c1 first byte → EIP2537 [208]
        raw[144] = 0x44; // Y.c0 first byte → EIP2537 [144]
        let enc = g2_raw_to_eip2537(&raw).unwrap();
        assert_eq!(enc[16], 0x22,  "X.c0 at [16]");
        assert_eq!(enc[80], 0x11,  "X.c1 at [80]");
        assert_eq!(enc[144], 0x44, "Y.c0 at [144]");
        assert_eq!(enc[208], 0x33, "Y.c1 at [208]");
    }

    /// T1.20 — `g1_raw_to_eip2537` preserves the FULL 48 bytes of
    /// each coordinate, not just the first. Existing routing test
    /// only checked the first byte of X and Y; this pins that all
    /// 48 X-bytes land at [16..64] and all 48 Y-bytes land at
    /// [80..128], and the 16-byte zero padding is preserved on
    /// each side.
    #[test]
    fn t1_20_g1_raw_to_eip2537_preserves_full_coordinates() {
        let mut raw = [0u8; 96];
        // Distinctive byte patterns so a coordinate swap would trip.
        for i in 0..48 {
            raw[i] = 0xA0 | (i as u8 & 0x0F); // X: 0xA0..0xAF cycling
            raw[48 + i] = 0xB0 | (i as u8 & 0x0F); // Y: 0xB0..0xBF cycling
        }
        let enc = g1_raw_to_eip2537(&raw).unwrap();
        // Padding zones.
        assert_eq!(&enc[0..16], &[0u8; 16], "left padding before X");
        assert_eq!(&enc[64..80], &[0u8; 16], "left padding before Y");
        // X bytes preserved.
        assert_eq!(&enc[16..64], &raw[0..48]);
        // Y bytes preserved.
        assert_eq!(&enc[80..128], &raw[48..96]);
    }

    /// T1.20 — `g2_raw_to_eip2537` preserves all 4 × 48 coordinate
    /// bytes through the ZCash → EIP-2537 reordering. Existing test
    /// only checked one byte per coordinate.
    #[test]
    fn t1_20_g2_raw_to_eip2537_preserves_full_coordinates() {
        let mut raw = [0u8; 192];
        for i in 0..48 {
            raw[i] = 0xC0 | (i as u8 & 0x0F); // X.c1
            raw[48 + i] = 0xD0 | (i as u8 & 0x0F); // X.c0
            raw[96 + i] = 0xE0 | (i as u8 & 0x0F); // Y.c1
            raw[144 + i] = 0xF0 | (i as u8 & 0x0F); // Y.c0
        }
        let enc = g2_raw_to_eip2537(&raw).unwrap();
        // X.c0 at [16..64], padded by [0..16].
        assert_eq!(&enc[0..16], &[0u8; 16]);
        assert_eq!(&enc[16..64], &raw[48..96]);
        // X.c1 at [80..128], padded by [64..80].
        assert_eq!(&enc[64..80], &[0u8; 16]);
        assert_eq!(&enc[80..128], &raw[0..48]);
        // Y.c0 at [144..192], padded by [128..144].
        assert_eq!(&enc[128..144], &[0u8; 16]);
        assert_eq!(&enc[144..192], &raw[144..192]);
        // Y.c1 at [208..256], padded by [192..208].
        assert_eq!(&enc[192..208], &[0u8; 16]);
        assert_eq!(&enc[208..256], &raw[96..144]);
    }

    /// T1.20 — `g1_compressed_to_eip2537` round-trips a real BLS
    /// public key (compressed → blst::serialize → EIP-2537). This
    /// is the production path for L1 precompile call setup; it
    /// previously had zero test coverage despite shipping behind
    /// `bls-native`.
    #[cfg(feature = "bls-native")]
    #[test]
    fn t1_20_g1_compressed_smoke_via_blst() {
        // Generate a real BLS keypair via blst.
        let ikm = [0x42u8; 32];
        let sk = blst::min_pk::SecretKey::key_gen(&ikm, &[]).expect("keygen");
        let pk = sk.sk_to_pk();
        let compressed = pk.compress(); // 48 bytes
        let enc = g1_compressed_to_eip2537(&compressed).expect("compressed → eip2537");
        // EIP-2537 layout is 128 bytes with two 16-byte zero
        // paddings; the X coordinate (decompressed) must be present
        // at [16..64] and Y at [80..128].
        assert_eq!(&enc[0..16], &[0u8; 16]);
        assert_eq!(&enc[64..80], &[0u8; 16]);
        // X is not all-zero (the generator's serialize never is).
        assert!(enc[16..64].iter().any(|&b| b != 0));
        // Wrong-length compressed input must Err.
        assert!(g1_compressed_to_eip2537(&[0u8; 47]).is_none());
        assert!(g1_compressed_to_eip2537(&[0u8; 48]).is_none()); // valid len, invalid bytes
    }

    /// T1.20 — `g2_compressed_to_eip2537` round-trips a real G2
    /// signature. Same shape as the G1 smoke; pins the
    /// `bls-native` compressed-input path.
    #[cfg(feature = "bls-native")]
    #[test]
    fn t1_20_g2_compressed_smoke_via_blst() {
        let ikm = [0x37u8; 32];
        let sk = blst::min_pk::SecretKey::key_gen(&ikm, &[]).expect("keygen");
        let dst = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_";
        let sig = sk.sign(b"test message", dst, &[]);
        let compressed = sig.compress(); // 96 bytes
        let enc = g2_compressed_to_eip2537(&compressed).expect("compressed → eip2537");
        // Four padding zones at [0..16], [64..80], [128..144], [192..208].
        assert_eq!(&enc[0..16], &[0u8; 16]);
        assert_eq!(&enc[64..80], &[0u8; 16]);
        assert_eq!(&enc[128..144], &[0u8; 16]);
        assert_eq!(&enc[192..208], &[0u8; 16]);
        // Wrong-length compressed input rejected.
        assert!(g2_compressed_to_eip2537(&[0u8; 95]).is_none());
    }
}
