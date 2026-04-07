//! Build-time metadata captured by build.rs.

/// Package version from Cargo.toml.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Git commit hash (short).
pub const GIT_HASH: &str = env!("GIT_HASH");

/// Git dirty flag ("-dirty" or "").
pub const GIT_DIRTY: &str = env!("GIT_DIRTY");

/// Build timestamp.
pub const BUILD_DATE: &str = env!("BUILD_DATE");

/// Target triple.
pub const BUILD_TARGET: &str = env!("BUILD_TARGET");

/// Rustc version.
pub const RUSTC_VERSION: &str = env!("RUSTC_VERSION");

/// Full version string for display.
pub fn full_version() -> String {
    format!(
        "evaporchain-wallet v{} ({}{}) built {} for {}",
        VERSION, GIT_HASH, GIT_DIRTY, BUILD_DATE, BUILD_TARGET
    )
}

/// Structured version info for JSON output.
#[derive(serde::Serialize)]
pub struct VersionInfo {
    pub version: &'static str,
    pub git_hash: &'static str,
    pub git_dirty: bool,
    pub build_date: &'static str,
    pub target: &'static str,
    pub rustc: &'static str,
    pub signature_scheme: &'static str,
}

impl VersionInfo {
    pub fn current() -> Self {
        Self {
            version: VERSION,
            git_hash: GIT_HASH,
            git_dirty: !GIT_DIRTY.is_empty(),
            build_date: BUILD_DATE,
            target: BUILD_TARGET,
            rustc: RUSTC_VERSION,
            signature_scheme: "ML-DSA-65 (FIPS 204)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_not_empty() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn test_full_version_contains_version() {
        let full = full_version();
        assert!(full.contains(VERSION));
    }

    #[test]
    fn test_version_info_serializable() {
        let info = VersionInfo::current();
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("ML-DSA"));
        assert!(json.contains(VERSION));
    }

    #[test]
    fn test_build_date_not_empty() {
        assert!(!BUILD_DATE.is_empty());
    }

    #[test]
    fn test_build_target_not_empty() {
        assert!(!BUILD_TARGET.is_empty());
    }
}
