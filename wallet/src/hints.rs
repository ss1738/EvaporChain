//! Actionable error hints for common wallet failures.
//!
//! Wraps raw error messages with user-friendly suggestions
//! so users know exactly how to fix the problem.

// ──────────────────────────── Hint Engine ─────────────────────────────────

/// Analyze an error message and return a helpful hint.
pub fn hint_for_error(error: &str) -> Option<&'static str> {
    let lower = error.to_lowercase();

    // Connection errors
    if lower.contains("connection refused") || lower.contains("connect error") {
        return Some("Is the node running? Check with: wallet status\n  Or set a different node: wallet config set node_url http://your-node:3000");
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return Some("The node is not responding. It may be syncing or overloaded.\n  Try again in a few seconds, or check node health: wallet status");
    }
    if lower.contains("dns") || lower.contains("resolve") {
        return Some(
            "Could not resolve the node hostname.\n  Check your node URL: wallet config show",
        );
    }

    // Auth / account errors
    if lower.contains("wrong password")
        || lower.contains("decryption failed")
        || lower.contains("mac mismatch")
    {
        return Some("Wrong password. Double-check your password and try again.\n  If forgotten, recover using: wallet seed recover <backup-file> <name>");
    }
    if lower.contains("no active account") || lower.contains("account not found") {
        return Some("No active account set. Create or switch:\n  wallet account create <name>\n  wallet account switch <name>");
    }
    if lower.contains("key not found") || lower.contains("account '") {
        return Some(
            "That account doesn't exist in your keystore.\n  List accounts: wallet account list",
        );
    }

    // Balance errors
    if lower.contains("insufficient balance") || lower.contains("insufficient funds") {
        return Some("Not enough EVAP to cover this transaction.\n  Check balance: wallet account balance\n  Get testnet tokens: wallet faucet");
    }

    // Nonce errors
    if lower.contains("nonce too low") || lower.contains("nonce mismatch") {
        return Some("Nonce is stale — another transaction was confirmed first.\n  The wallet will auto-retry with the correct nonce. Try again.");
    }

    // Object errors
    if lower.contains("object not found") || lower.contains("object does not exist") {
        return Some("That object doesn't exist on-chain. It may have evaporated.\n  Check ghosts: wallet ghost list\n  View all objects: wallet objects");
    }
    if lower.contains("already evaporated") || lower.contains("ghost") {
        return Some("This object has evaporated (energy reached zero).\n  Check resurrection cost: wallet ghost cost <half-life>\n  View ghost records: wallet ghost list");
    }
    if lower.contains("grace period") {
        return Some("Object is in grace period — refresh it before it evaporates!\n  wallet refresh <object-id> <energy>");
    }

    // NFT errors
    if lower.contains("nft not found") {
        return Some("That NFT doesn't exist. List available NFTs: wallet nft list");
    }
    if lower.contains("not owner") || lower.contains("unauthorized") {
        return Some("You don't own this asset. Only the owner can perform this action.");
    }

    // Network / API errors
    if lower.contains("502") || lower.contains("503") || lower.contains("504") {
        return Some("The node returned a server error. This is usually temporary.\n  Try again in a few seconds.");
    }
    if lower.contains("429") || lower.contains("rate limit") {
        return Some("Too many requests. The wallet will automatically retry.\n  If this persists, increase the poll interval in auto-refresh.");
    }

    // File errors
    if lower.contains("no such file")
        || lower.contains("file not found")
        || lower.contains("notfound")
    {
        return Some("File not found. Check the file path and try again.");
    }
    if lower.contains("permission denied") {
        return Some("Permission denied. Check file permissions or run with appropriate access.");
    }

    // Validation errors (these already have hints from validation.rs, but catch leftovers)
    if lower.contains("invalid hex") {
        return Some("Address must be valid hex. Example: 0x1a2b3c4d...\n  Use: wallet account list to see your addresses");
    }

    None
}

/// Format an error with a hint for display.
pub fn format_error_with_hint(error: &str) -> String {
    if let Some(hint) = hint_for_error(error) {
        format!("{}\n\n  💡 {}", error, hint)
    } else {
        error.to_string()
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_refused_hint() {
        let hint = hint_for_error("HTTP error: connection refused").unwrap();
        assert!(hint.contains("node running"));
    }

    #[test]
    fn test_timeout_hint() {
        let hint = hint_for_error("request timed out").unwrap();
        assert!(hint.contains("not responding"));
    }

    #[test]
    fn test_wrong_password_hint() {
        let hint = hint_for_error("decryption failed: mac mismatch").unwrap();
        assert!(hint.contains("Wrong password"));
    }

    #[test]
    fn test_insufficient_balance_hint() {
        let hint = hint_for_error("insufficient balance for transfer").unwrap();
        assert!(hint.contains("faucet"));
    }

    #[test]
    fn test_nonce_hint() {
        let hint = hint_for_error("nonce too low").unwrap();
        assert!(hint.contains("auto-retry"));
    }

    #[test]
    fn test_object_evaporated_hint() {
        let hint = hint_for_error("object has already evaporated").unwrap();
        assert!(hint.contains("resurrection"));
    }

    #[test]
    fn test_no_account_hint() {
        let hint = hint_for_error("no active account set").unwrap();
        assert!(hint.contains("account create"));
    }

    #[test]
    fn test_server_error_hint() {
        let hint = hint_for_error("502 Bad Gateway").unwrap();
        assert!(hint.contains("temporary"));
    }

    #[test]
    fn test_rate_limit_hint() {
        let hint = hint_for_error("429 Too Many Requests").unwrap();
        assert!(hint.contains("retry"));
    }

    #[test]
    fn test_no_hint_for_unknown() {
        assert!(hint_for_error("some random error xyz").is_none());
    }

    #[test]
    fn test_format_with_hint() {
        let formatted = format_error_with_hint("connection refused");
        assert!(formatted.contains("connection refused"));
        assert!(formatted.contains("node running"));
    }

    #[test]
    fn test_format_without_hint() {
        let formatted = format_error_with_hint("something unknown");
        assert_eq!(formatted, "something unknown");
    }

    #[test]
    fn test_grace_period_hint() {
        let hint = hint_for_error("object is in grace period").unwrap();
        assert!(hint.contains("refresh"));
    }

    #[test]
    fn test_permission_denied_hint() {
        let hint = hint_for_error("permission denied").unwrap();
        assert!(hint.contains("Permission"));
    }

    #[test]
    fn test_dns_hint() {
        let hint = hint_for_error("could not resolve dns").unwrap();
        assert!(hint.contains("hostname"));
    }
}
