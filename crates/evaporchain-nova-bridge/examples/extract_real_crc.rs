use evaporchain_nova_bridge::{expected_crc_len, extract_compressed_round_constants};
fn main() {
    let crc =
        extract_compressed_round_constants("/tmp/neptune-bn256-standard.json").expect("extract");
    println!("crc len: {}", crc.len());
    println!(
        "expected (arity-24 standard): {}",
        expected_crc_len(8, 59, 25)
    );
    println!(
        "first 3 entries non-zero: {} {} {}",
        crc[0] != ark_bn254::Fr::from(0u64),
        crc[1] != ark_bn254::Fr::from(0u64),
        crc[2] != ark_bn254::Fr::from(0u64),
    );
}
