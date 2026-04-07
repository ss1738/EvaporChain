//! Input validation for wallet commands.
//!
//! Catches bad inputs early with user-friendly error messages.
//! All validators return `Result<T, ValidationError>` with actionable hints.

use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("invalid address: {reason}\n  Hint: addresses are 64 hex characters with 0x prefix, e.g. 0x1a2b3c...")]
    InvalidAddress { reason: String },

    #[error("invalid amount: {reason}\n  Hint: amount must be a positive integer (no decimals, no negatives)")]
    InvalidAmount { reason: String },

    #[error("invalid object ID: {reason}\n  Hint: object IDs are 64 hex characters with 0x prefix")]
    InvalidObjectId { reason: String },

    #[error("invalid energy: {reason}")]
    InvalidEnergy { reason: String },

    #[error("invalid half-life: {reason}")]
    InvalidHalfLife { reason: String },

    #[error("invalid nonce: {reason}")]
    InvalidNonce { reason: String },

    #[error("invalid threshold: {reason}\n  Hint: threshold must be between 0 and 100 (percentage)")]
    InvalidThreshold { reason: String },

    #[error("invalid name: {reason}")]
    InvalidName { reason: String },
}

// ──────────────────────────── Address Validation ─────────────────────────

/// Validate and normalize an address string.
/// Accepts 0x-prefixed or raw hex, must be exactly 32 bytes (64 hex chars).
pub fn validate_address(input: &str) -> Result<String, ValidationError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ValidationError::InvalidAddress {
            reason: "address cannot be empty".into(),
        });
    }

    let hex_str = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")).unwrap_or(trimmed);

    if hex_str.len() != 64 {
        return Err(ValidationError::InvalidAddress {
            reason: format!(
                "expected 64 hex characters, got {} ({})",
                hex_str.len(),
                if hex_str.len() < 64 { "too short" } else { "too long" }
            ),
        });
    }

    if !hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
        let bad_char = hex_str.chars().find(|c| !c.is_ascii_hexdigit()).unwrap();
        return Err(ValidationError::InvalidAddress {
            reason: format!("invalid character '{}' — only 0-9, a-f, A-F allowed", bad_char),
        });
    }

    // Normalize to lowercase with 0x prefix
    Ok(format!("0x{}", hex_str.to_lowercase()))
}

/// Validate that a string looks like an address OR is a contact name.
/// Contact names are alphanumeric + underscores, max 32 chars.
pub fn validate_recipient(input: &str) -> Result<String, ValidationError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ValidationError::InvalidAddress {
            reason: "recipient cannot be empty".into(),
        });
    }

    // If it starts with 0x or is all hex, validate as address
    if trimmed.starts_with("0x") || trimmed.starts_with("0X") || (trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit())) {
        return validate_address(trimmed);
    }

    // Otherwise treat as contact name
    validate_name(trimmed)?;
    Ok(trimmed.to_string())
}

// ──────────────────────────── Amount Validation ──────────────────────────

/// Validate a transfer amount.
pub fn validate_amount(amount: u64) -> Result<u64, ValidationError> {
    if amount == 0 {
        return Err(ValidationError::InvalidAmount {
            reason: "amount must be greater than zero".into(),
        });
    }
    Ok(amount)
}

/// Parse and validate an amount from string input.
pub fn parse_amount(input: &str) -> Result<u64, ValidationError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ValidationError::InvalidAmount {
            reason: "amount cannot be empty".into(),
        });
    }

    if trimmed.contains('.') {
        return Err(ValidationError::InvalidAmount {
            reason: "decimal amounts not supported — use whole units".into(),
        });
    }

    if trimmed.starts_with('-') {
        return Err(ValidationError::InvalidAmount {
            reason: "amount cannot be negative".into(),
        });
    }

    let value: u64 = trimmed.parse().map_err(|_| ValidationError::InvalidAmount {
        reason: format!("'{}' is not a valid integer", trimmed),
    })?;

    validate_amount(value)
}

// ──────────────────────────── Energy Validation ──────────────────────────

/// Validate energy deposit amount.
pub fn validate_energy(energy: u64) -> Result<u64, ValidationError> {
    if energy == 0 {
        return Err(ValidationError::InvalidEnergy {
            reason: "energy deposit must be greater than zero".into(),
        });
    }
    if energy > 1_000_000_000 {
        return Err(ValidationError::InvalidEnergy {
            reason: format!("energy {} exceeds maximum (1,000,000,000)", energy),
        });
    }
    Ok(energy)
}

/// Validate half-life value.
pub fn validate_half_life(half_life: u64) -> Result<u64, ValidationError> {
    if half_life == 0 {
        return Err(ValidationError::InvalidHalfLife {
            reason: "half-life must be greater than zero (objects would evaporate instantly)".into(),
        });
    }
    if half_life > 1_000_000 {
        return Err(ValidationError::InvalidHalfLife {
            reason: format!("half-life {} is unreasonably large (max 1,000,000 epochs)", half_life),
        });
    }
    Ok(half_life)
}

// ──────────────────────────── Threshold Validation ───────────────────────

/// Validate a percentage threshold (0-100).
pub fn validate_threshold(pct: f64) -> Result<f64, ValidationError> {
    if pct < 0.0 || pct > 100.0 {
        return Err(ValidationError::InvalidThreshold {
            reason: format!("{} is out of range (must be 0-100)", pct),
        });
    }
    if pct.is_nan() || pct.is_infinite() {
        return Err(ValidationError::InvalidThreshold {
            reason: "threshold must be a finite number".into(),
        });
    }
    Ok(pct)
}

// ──────────────────────────── Name Validation ────────────────────────────

/// Validate an account or contact name.
pub fn validate_name(name: &str) -> Result<&str, ValidationError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ValidationError::InvalidName {
            reason: "name cannot be empty".into(),
        });
    }
    if trimmed.len() > 32 {
        return Err(ValidationError::InvalidName {
            reason: format!("name too long ({} chars, max 32)", trimmed.len()),
        });
    }
    if !trimmed.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return Err(ValidationError::InvalidName {
            reason: "name can only contain letters, numbers, underscores, and hyphens".into(),
        });
    }
    if !trimmed.chars().next().unwrap().is_alphabetic() {
        return Err(ValidationError::InvalidName {
            reason: "name must start with a letter".into(),
        });
    }
    Ok(trimmed)
}

// ──────────────────────────── Password Validation ────────────────────────

/// Validate password strength (minimum requirements).
pub fn validate_password(password: &str) -> Result<(), ValidationError> {
    if password.len() < 8 {
        return Err(ValidationError::InvalidName {
            reason: format!("password too short ({} chars, minimum 8)", password.len()),
        });
    }
    Ok(())
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Address tests ──

    #[test]
    fn test_valid_address_with_prefix() {
        let addr = format!("0x{}", "ab".repeat(32));
        let result = validate_address(&addr).unwrap();
        assert_eq!(result, addr);
    }

    #[test]
    fn test_valid_address_without_prefix() {
        let hex = "ab".repeat(32);
        let result = validate_address(&hex).unwrap();
        assert_eq!(result, format!("0x{}", hex));
    }

    #[test]
    fn test_valid_address_uppercase_normalized() {
        let addr = format!("0x{}", "AB".repeat(32));
        let result = validate_address(&addr).unwrap();
        assert_eq!(result, format!("0x{}", "ab".repeat(32)));
    }

    #[test]
    fn test_address_empty() {
        assert!(validate_address("").is_err());
    }

    #[test]
    fn test_address_too_short() {
        let err = validate_address("0xabcd").unwrap_err();
        assert!(err.to_string().contains("too short"));
    }

    #[test]
    fn test_address_too_long() {
        let addr = format!("0x{}", "ab".repeat(33));
        let err = validate_address(&addr).unwrap_err();
        assert!(err.to_string().contains("too long"));
    }

    #[test]
    fn test_address_invalid_chars() {
        let addr = format!("0x{}gg", "ab".repeat(31));
        let err = validate_address(&addr).unwrap_err();
        assert!(err.to_string().contains("invalid character"));
    }

    // ── Recipient tests ──

    #[test]
    fn test_recipient_address() {
        let addr = format!("0x{}", "ab".repeat(32));
        let result = validate_recipient(&addr).unwrap();
        assert_eq!(result, addr);
    }

    #[test]
    fn test_recipient_contact_name() {
        let result = validate_recipient("alice").unwrap();
        assert_eq!(result, "alice");
    }

    #[test]
    fn test_recipient_empty() {
        assert!(validate_recipient("").is_err());
    }

    // ── Amount tests ──

    #[test]
    fn test_valid_amount() {
        assert_eq!(validate_amount(1000).unwrap(), 1000);
    }

    #[test]
    fn test_zero_amount() {
        assert!(validate_amount(0).is_err());
    }

    #[test]
    fn test_parse_amount_valid() {
        assert_eq!(parse_amount("5000").unwrap(), 5000);
    }

    #[test]
    fn test_parse_amount_decimal() {
        let err = parse_amount("10.5").unwrap_err();
        assert!(err.to_string().contains("decimal"));
    }

    #[test]
    fn test_parse_amount_negative() {
        let err = parse_amount("-100").unwrap_err();
        assert!(err.to_string().contains("negative"));
    }

    #[test]
    fn test_parse_amount_empty() {
        assert!(parse_amount("").is_err());
    }

    #[test]
    fn test_parse_amount_garbage() {
        assert!(parse_amount("abc").is_err());
    }

    // ── Energy tests ──

    #[test]
    fn test_valid_energy() {
        assert_eq!(validate_energy(500).unwrap(), 500);
    }

    #[test]
    fn test_zero_energy() {
        assert!(validate_energy(0).is_err());
    }

    #[test]
    fn test_excessive_energy() {
        assert!(validate_energy(2_000_000_000).is_err());
    }

    // ── Half-life tests ──

    #[test]
    fn test_valid_half_life() {
        assert_eq!(validate_half_life(100).unwrap(), 100);
    }

    #[test]
    fn test_zero_half_life() {
        let err = validate_half_life(0).unwrap_err();
        assert!(err.to_string().contains("instantly"));
    }

    #[test]
    fn test_excessive_half_life() {
        assert!(validate_half_life(10_000_000).is_err());
    }

    // ── Threshold tests ──

    #[test]
    fn test_valid_threshold() {
        assert!((validate_threshold(25.0).unwrap() - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_threshold_zero() {
        assert!(validate_threshold(0.0).is_ok());
    }

    #[test]
    fn test_threshold_hundred() {
        assert!(validate_threshold(100.0).is_ok());
    }

    #[test]
    fn test_threshold_negative() {
        assert!(validate_threshold(-1.0).is_err());
    }

    #[test]
    fn test_threshold_over_hundred() {
        assert!(validate_threshold(101.0).is_err());
    }

    #[test]
    fn test_threshold_nan() {
        assert!(validate_threshold(f64::NAN).is_err());
    }

    // ── Name tests ──

    #[test]
    fn test_valid_name() {
        assert_eq!(validate_name("alice").unwrap(), "alice");
    }

    #[test]
    fn test_name_with_numbers() {
        assert_eq!(validate_name("wallet2").unwrap(), "wallet2");
    }

    #[test]
    fn test_name_with_underscores() {
        assert_eq!(validate_name("my_wallet").unwrap(), "my_wallet");
    }

    #[test]
    fn test_name_with_hyphens() {
        assert_eq!(validate_name("my-wallet").unwrap(), "my-wallet");
    }

    #[test]
    fn test_name_empty() {
        assert!(validate_name("").is_err());
    }

    #[test]
    fn test_name_too_long() {
        let long = "a".repeat(33);
        assert!(validate_name(&long).is_err());
    }

    #[test]
    fn test_name_special_chars() {
        assert!(validate_name("alice@bob").is_err());
    }

    #[test]
    fn test_name_starts_with_number() {
        assert!(validate_name("2fast").is_err());
    }

    // ── Password tests ──

    #[test]
    fn test_valid_password() {
        assert!(validate_password("securepass").is_ok());
    }

    #[test]
    fn test_short_password() {
        assert!(validate_password("abc").is_err());
    }
}
