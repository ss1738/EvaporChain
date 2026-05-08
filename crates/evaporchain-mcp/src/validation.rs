//! Input validation for MCP write-tools — closes the second part of
//! `AUDIT_2026_05_06.md` CRITICAL-2.
//!
//! The bearer-token + require-auth gate from commit `639c843`
//! authenticates the MCP-to-backend channel. This module hardens
//! the AGENT-to-MCP boundary: per-tool range checks, address
//! validation, and schema enforcement so a prompt-injected agent
//! cannot push arbitrary or malicious payloads through the MCP
//! into chain state.
//!
//! All validators are pure functions over `&serde_json::Value` and
//! return typed errors. Each handler in `tools.rs` calls the
//! relevant validator BEFORE constructing the backend POST body.
//! Failures bubble up as `Err(String)` from `handle_tool` and the
//! MCP responds to the agent with a JSON-RPC error — no chain
//! state is touched.
//!
//! # Bounds rationale
//!
//! - **Address / object_id**: 32-byte hex (64 chars, optional `0x`
//!   prefix). Anything else is malformed by chain wire-format.
//! - **Amount / energy / energy_deposit**: positive (`>= 1`) and
//!   bounded above by `MAX_TOKEN_AMOUNT = 2^60`. Real economic
//!   values fit comfortably; values above this are signs of either
//!   a bug or an attempt to break u64 arithmetic somewhere
//!   downstream (overflow protection is independently enforced in
//!   `evaporchain-execution`'s checked-arithmetic path, but
//!   filtering at the MCP edge means chain-side never sees it).
//! - **Nonce**: any non-negative u64 fits the chain's nonce space.
//! - **Half-life**: positive (`>= 1`) and bounded above by
//!   `MAX_HALF_LIFE_EPOCHS = 2^40`. Larger values would indicate a
//!   typo (years of half-life is implausible).

use serde_json::Value;

/// Maximum accepted token / energy amount. 2^60 is comfortably
/// above any real economic value and well below u64::MAX, leaving
/// headroom against arithmetic overflow downstream.
pub const MAX_TOKEN_AMOUNT: u64 = 1 << 60;

/// Maximum accepted half-life in epochs. 2^40 epochs is ~10^12
/// blocks, far above any plausible chain configuration.
pub const MAX_HALF_LIFE_EPOCHS: u64 = 1 << 40;

/// Typed validation error for MCP tool inputs.
#[derive(Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// Field is missing from the args object.
    Missing(&'static str),
    /// Field value is not the expected JSON type.
    WrongType {
        field: &'static str,
        expected: &'static str,
    },
    /// Field value is out of the allowed range.
    OutOfRange {
        field: &'static str,
        min: u64,
        max: u64,
        got: u64,
    },
    /// Address-shaped string failed format validation (length,
    /// hex content, prefix).
    MalformedAddress {
        field: &'static str,
        reason: &'static str,
    },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::Missing(field) => {
                write!(f, "Missing required field: '{field}'")
            }
            ValidationError::WrongType { field, expected } => {
                write!(f, "Field '{field}' has wrong type; expected {expected}")
            }
            ValidationError::OutOfRange {
                field,
                min,
                max,
                got,
            } => write!(f, "Field '{field}' = {got} is out of range [{min}, {max}]"),
            ValidationError::MalformedAddress { field, reason } => {
                write!(f, "Field '{field}' is malformed: {reason}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

/// Validate a hex-encoded 32-byte address (or object_id). Accepts
/// either bare 64 hex chars or `0x` + 64 hex chars. Returns the
/// normalised string with the `0x` prefix.
pub fn validate_address_field(
    args: &Value,
    field: &'static str,
) -> Result<String, ValidationError> {
    let raw = args
        .get(field)
        .ok_or(ValidationError::Missing(field))?
        .as_str()
        .ok_or(ValidationError::WrongType {
            field,
            expected: "hex-string",
        })?;

    let stripped = raw.strip_prefix("0x").unwrap_or(raw);
    if stripped.len() != 64 {
        return Err(ValidationError::MalformedAddress {
            field,
            reason: "must be 64 hex chars (with or without 0x prefix)",
        });
    }
    if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ValidationError::MalformedAddress {
            field,
            reason: "non-hex character",
        });
    }
    Ok(format!("0x{stripped}"))
}

/// Validate a positive u64 amount with an inclusive upper bound.
/// Used for transfer amounts, create-object energy deposits,
/// refresh / resurrect deposits.
pub fn validate_amount_field(
    args: &Value,
    field: &'static str,
    max: u64,
) -> Result<u64, ValidationError> {
    let v = args.get(field).ok_or(ValidationError::Missing(field))?;
    let n = v.as_u64().ok_or(ValidationError::WrongType {
        field,
        expected: "non-negative integer",
    })?;
    if n < 1 || n > max {
        return Err(ValidationError::OutOfRange {
            field,
            min: 1,
            max,
            got: n,
        });
    }
    Ok(n)
}

/// Validate a nonce field. Accepts any non-negative u64 (zero is
/// the legitimate first-tx nonce for a fresh address).
pub fn validate_nonce_field(args: &Value, field: &'static str) -> Result<u64, ValidationError> {
    let v = args.get(field).ok_or(ValidationError::Missing(field))?;
    v.as_u64().ok_or(ValidationError::WrongType {
        field,
        expected: "non-negative integer",
    })
}

/// Validate a half-life field — positive u64 bounded by
/// [`MAX_HALF_LIFE_EPOCHS`].
pub fn validate_half_life_field(args: &Value, field: &'static str) -> Result<u64, ValidationError> {
    validate_amount_field(args, field, MAX_HALF_LIFE_EPOCHS)
}

/// Validate a 64-char hex transaction hash. Reuses address rules —
/// both are 32-byte BLAKE3 digests with identical wire encoding.
pub fn validate_tx_hash_field(
    args: &Value,
    field: &'static str,
) -> Result<String, ValidationError> {
    validate_address_field(args, field)
}

/// Validate a block height — any non-negative u64 (zero = genesis).
pub fn validate_block_height_field(
    args: &Value,
    field: &'static str,
) -> Result<u64, ValidationError> {
    let v = args.get(field).ok_or(ValidationError::Missing(field))?;
    v.as_u64().ok_or(ValidationError::WrongType {
        field,
        expected: "non-negative integer",
    })
}

/// Validate a hex object-id path component — less strict than a full
/// 32-byte address. Accepts 1–128 ASCII hex chars (with/without `0x`).
/// Prevents path-traversal injection (`/`, `..`, `?`, `#`, etc.).
pub fn validate_hex_id_field(
    args: &Value,
    field: &'static str,
) -> Result<String, ValidationError> {
    let raw = args
        .get(field)
        .ok_or(ValidationError::Missing(field))?
        .as_str()
        .ok_or(ValidationError::WrongType {
            field,
            expected: "hex-string",
        })?;

    let stripped = raw.strip_prefix("0x").unwrap_or(raw);
    if stripped.is_empty() || stripped.len() > 128 {
        return Err(ValidationError::MalformedAddress {
            field,
            reason: "must be 1–128 hex chars (with or without 0x prefix)",
        });
    }
    if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ValidationError::MalformedAddress {
            field,
            reason: "non-hex character",
        });
    }
    Ok(stripped.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn good_addr() -> String {
        format!("0x{}", "a".repeat(64))
    }

    // ── address validation ────────────────────────────────────────

    #[test]
    fn address_with_0x_prefix_accepted() {
        let args = json!({ "from": good_addr() });
        assert_eq!(validate_address_field(&args, "from").unwrap(), good_addr());
    }

    #[test]
    fn address_without_prefix_normalised_to_0x() {
        let raw = "a".repeat(64);
        let args = json!({ "from": raw });
        assert_eq!(validate_address_field(&args, "from").unwrap(), good_addr());
    }

    #[test]
    fn address_missing_field_rejected() {
        let args = json!({});
        assert_eq!(
            validate_address_field(&args, "from").unwrap_err(),
            ValidationError::Missing("from")
        );
    }

    #[test]
    fn address_wrong_type_rejected() {
        let args = json!({ "from": 42 });
        assert!(matches!(
            validate_address_field(&args, "from"),
            Err(ValidationError::WrongType { field: "from", .. })
        ));
    }

    #[test]
    fn address_wrong_length_rejected() {
        let args = json!({ "from": "0xabc" });
        assert!(matches!(
            validate_address_field(&args, "from"),
            Err(ValidationError::MalformedAddress { reason, .. })
                if reason.contains("64 hex chars")
        ));
    }

    #[test]
    fn address_non_hex_rejected() {
        let args = json!({ "from": format!("{}", "z".repeat(64)) });
        assert!(matches!(
            validate_address_field(&args, "from"),
            Err(ValidationError::MalformedAddress { reason, .. })
                if reason.contains("non-hex")
        ));
    }

    // ── amount validation ─────────────────────────────────────────

    #[test]
    fn amount_in_range_accepted() {
        let args = json!({ "amount": 100 });
        assert_eq!(
            validate_amount_field(&args, "amount", MAX_TOKEN_AMOUNT).unwrap(),
            100
        );
    }

    #[test]
    fn amount_zero_rejected() {
        let args = json!({ "amount": 0 });
        assert!(matches!(
            validate_amount_field(&args, "amount", MAX_TOKEN_AMOUNT),
            Err(ValidationError::OutOfRange { got: 0, min: 1, .. })
        ));
    }

    #[test]
    fn amount_over_max_rejected() {
        let args = json!({ "amount": MAX_TOKEN_AMOUNT + 1 });
        assert!(matches!(
            validate_amount_field(&args, "amount", MAX_TOKEN_AMOUNT),
            Err(ValidationError::OutOfRange { .. })
        ));
    }

    #[test]
    fn amount_negative_rejected() {
        let args = json!({ "amount": -5 });
        assert!(matches!(
            validate_amount_field(&args, "amount", MAX_TOKEN_AMOUNT),
            Err(ValidationError::WrongType { .. })
        ));
    }

    #[test]
    fn amount_string_rejected() {
        let args = json!({ "amount": "100" });
        assert!(matches!(
            validate_amount_field(&args, "amount", MAX_TOKEN_AMOUNT),
            Err(ValidationError::WrongType { .. })
        ));
    }

    // ── nonce validation ──────────────────────────────────────────

    #[test]
    fn nonce_zero_accepted() {
        let args = json!({ "nonce": 0 });
        assert_eq!(validate_nonce_field(&args, "nonce").unwrap(), 0);
    }

    #[test]
    fn nonce_max_u64_accepted() {
        let args = json!({ "nonce": u64::MAX });
        assert_eq!(validate_nonce_field(&args, "nonce").unwrap(), u64::MAX);
    }

    #[test]
    fn nonce_negative_rejected() {
        let args = json!({ "nonce": -1 });
        assert!(matches!(
            validate_nonce_field(&args, "nonce"),
            Err(ValidationError::WrongType { .. })
        ));
    }

    // ── half-life validation ──────────────────────────────────────

    #[test]
    fn half_life_positive_accepted() {
        let args = json!({ "half_life": 100 });
        assert_eq!(validate_half_life_field(&args, "half_life").unwrap(), 100);
    }

    #[test]
    fn half_life_at_max_accepted() {
        let args = json!({ "half_life": MAX_HALF_LIFE_EPOCHS });
        assert_eq!(
            validate_half_life_field(&args, "half_life").unwrap(),
            MAX_HALF_LIFE_EPOCHS
        );
    }

    #[test]
    fn half_life_zero_rejected() {
        let args = json!({ "half_life": 0 });
        assert!(matches!(
            validate_half_life_field(&args, "half_life"),
            Err(ValidationError::OutOfRange { .. })
        ));
    }

    #[test]
    fn half_life_over_max_rejected() {
        let args = json!({ "half_life": MAX_HALF_LIFE_EPOCHS + 1 });
        assert!(matches!(
            validate_half_life_field(&args, "half_life"),
            Err(ValidationError::OutOfRange { .. })
        ));
    }
}
