//! Address derivation and formatting.
//!
//! EvaporChain addresses are 32 bytes derived from ML-DSA public keys:
//!   address = BLAKE3(public_key_bytes)
//!
//! Display format: `0x` + hex(address), e.g. `0xa1b2c3...`

use evaporchain_types::AccountAddress;

/// Derive a 32-byte address from an ML-DSA public key.
pub fn derive_address(public_key: &[u8]) -> AccountAddress {
    *blake3::hash(public_key).as_bytes()
}

/// Format an address as a hex string with 0x prefix.
pub fn format_address(addr: &AccountAddress) -> String {
    format!("0x{}", hex::encode(addr))
}

/// Format an address as a short hex string (first 8 bytes).
pub fn short_address(addr: &AccountAddress) -> String {
    format!(
        "0x{}..{}",
        hex::encode(&addr[..4]),
        hex::encode(&addr[28..])
    )
}

/// Parse a hex address string (with or without 0x prefix) into 32 bytes.
pub fn parse_address(s: &str) -> Result<AccountAddress, String> {
    let hex_str = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(hex_str).map_err(|e| format!("invalid hex: {}", e))?;
    if bytes.len() != 32 {
        return Err(format!("address must be 32 bytes, got {}", bytes.len()));
    }
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&bytes);
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_address_deterministic() {
        let pk = vec![42u8; 1952]; // ML-DSA public key size
        let a1 = derive_address(&pk);
        let a2 = derive_address(&pk);
        assert_eq!(a1, a2);
        assert_ne!(a1, [0u8; 32]);
    }

    #[test]
    fn test_different_keys_different_addresses() {
        let pk1 = vec![1u8; 1952];
        let pk2 = vec![2u8; 1952];
        assert_ne!(derive_address(&pk1), derive_address(&pk2));
    }

    #[test]
    fn test_format_address() {
        let mut addr = [0u8; 32];
        addr[0] = 0xAB;
        let s = format_address(&addr);
        assert!(s.starts_with("0x"));
        assert_eq!(s.len(), 66); // 2 + 64
    }

    #[test]
    fn test_short_address() {
        let mut addr = [0u8; 32];
        addr[0] = 0xAB;
        addr[31] = 0xCD;
        let s = short_address(&addr);
        assert!(s.starts_with("0xab"));
        assert!(s.contains(".."));
    }

    #[test]
    fn test_parse_address_with_prefix() {
        let addr = [42u8; 32];
        let s = format_address(&addr);
        let parsed = parse_address(&s).unwrap();
        assert_eq!(parsed, addr);
    }

    #[test]
    fn test_parse_address_without_prefix() {
        let addr = [42u8; 32];
        let s = hex::encode(addr);
        let parsed = parse_address(&s).unwrap();
        assert_eq!(parsed, addr);
    }

    #[test]
    fn test_parse_address_invalid_length() {
        assert!(parse_address("0xabcd").is_err());
    }

    #[test]
    fn test_parse_address_invalid_hex() {
        assert!(parse_address("0xGGGG").is_err());
    }
}
