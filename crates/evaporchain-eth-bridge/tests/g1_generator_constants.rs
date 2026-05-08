//! Emit the BLS12-381 G1 generator and its negation in EIP-2537
//! uncompressed encoding so the Solidity verifier can hardcode
//! `NEG_G1_GEN_UNCOMP` for the BLS pairing check.

use bls12_381::{G1Affine, G1Projective};
use group::Group;

/// EIP-2537 uncompressed encoding of a `G1Affine`. 128 bytes:
/// [pad16 || x_be_48 || pad16 || y_be_48].
fn g1_to_eip2537(p: G1Affine) -> [u8; 128] {
    // bls12_381 0.8 uncompressed: [x_be_48 || y_be_48], 96 bytes.
    let raw = p.to_uncompressed(); // [u8; 96]
    let x = &raw[0..48];
    let y = &raw[48..96];
    let mut out = [0u8; 128];
    out[16..64].copy_from_slice(x);
    out[80..128].copy_from_slice(y);
    out
}

#[test]
fn dump_g1_generator_eip2537() {
    let g = G1Projective::generator();
    let g_aff: G1Affine = g.into();
    let g_enc = g1_to_eip2537(g_aff);

    let neg_g_aff: G1Affine = (-g).into();
    let neg_g_enc = g1_to_eip2537(neg_g_aff);

    println!("\nG1_GEN_UNCOMP = 0x{}", hex::encode(g_enc));
    println!("NEG_G1_GEN_UNCOMP = 0x{}\n", hex::encode(neg_g_enc));

    // Sanity: the two share x, differ in y.
    assert_eq!(&g_enc[0..64], &neg_g_enc[0..64]);
    assert_ne!(&g_enc[64..128], &neg_g_enc[64..128]);
}
