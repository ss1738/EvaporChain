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
}
