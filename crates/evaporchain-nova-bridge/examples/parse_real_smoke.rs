use evaporchain_nova_bridge::parse_dump;
fn main() {
    let s = parse_dump("/tmp/neptune-bn256-standard.json").expect("parse");
    println!("rf={} rp={} mds_m_dims={:?} crc_len={}", s.full_rounds, s.partial_rounds, s.mds_m_dims, s.crc_len);
    println!("mds_m[0][0] = {}", s.mds_m00_hex);
    println!("crc[0]      = {}", s.crc_0_hex);
}
