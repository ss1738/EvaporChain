//! `TemplateDescriptor` — metadata record for one deployable template.
//!
//! Renders as a "deploy this primitive" form in the wallet/dApp UI.
//! Carries:
//! - `class` — the stable id consumers address by
//! - `brand` — human-readable consumer name (e.g. "Singh-Sabi")
//! - `lane` — string tag for grouping by lane in the UI
//! - `default_params` — JSON `serde_json::Value` the deploy form
//!   pre-populates; users edit before submitting
//! - `notes` — short description for the form's tooltip
//! - `crate_name` — full path to the underlying Rust crate
//!   (so curious devs can `cargo doc --open -p X`)

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::class::TemplateClass;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DescriptorError {
    #[error("brand must be non-empty")]
    EmptyBrand,
    #[error("lane must be non-empty")]
    EmptyLane,
    #[error("crate_name must be non-empty")]
    EmptyCrate,
    #[error("class id {0:#010x} is outside the app-template range")]
    OutOfRange(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateDescriptor {
    pub class: TemplateClass,
    pub brand: String,
    pub lane: String,
    pub default_params: serde_json::Value,
    pub notes: String,
    pub crate_name: String,
}

impl TemplateDescriptor {
    pub fn new(
        class: TemplateClass,
        brand: impl Into<String>,
        lane: impl Into<String>,
        default_params: serde_json::Value,
        notes: impl Into<String>,
        crate_name: impl Into<String>,
    ) -> Result<Self, DescriptorError> {
        if !class.is_in_app_range() {
            return Err(DescriptorError::OutOfRange(class.0));
        }
        let brand = brand.into();
        if brand.is_empty() {
            return Err(DescriptorError::EmptyBrand);
        }
        let lane = lane.into();
        if lane.is_empty() {
            return Err(DescriptorError::EmptyLane);
        }
        let crate_name = crate_name.into();
        if crate_name.is_empty() {
            return Err(DescriptorError::EmptyCrate);
        }
        Ok(Self {
            class,
            brand,
            lane,
            default_params,
            notes: notes.into(),
            crate_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::class::SINGH_SABI;

    #[test]
    fn rejects_out_of_range_class() {
        let err = TemplateDescriptor::new(
            TemplateClass(0x9999_9999),
            "test",
            "lane",
            serde_json::json!({}),
            "notes",
            "crate",
        )
        .unwrap_err();
        assert!(matches!(err, DescriptorError::OutOfRange(_)));
    }

    #[test]
    fn rejects_empty_brand() {
        let err = TemplateDescriptor::new(
            SINGH_SABI,
            "",
            "lane",
            serde_json::json!({}),
            "notes",
            "crate",
        )
        .unwrap_err();
        assert_eq!(err, DescriptorError::EmptyBrand);
    }

    #[test]
    fn rejects_empty_lane() {
        let err = TemplateDescriptor::new(
            SINGH_SABI,
            "Singh-Sabi",
            "",
            serde_json::json!({}),
            "notes",
            "crate",
        )
        .unwrap_err();
        assert_eq!(err, DescriptorError::EmptyLane);
    }

    #[test]
    fn rejects_empty_crate() {
        let err = TemplateDescriptor::new(
            SINGH_SABI,
            "Singh-Sabi",
            "NFT",
            serde_json::json!({}),
            "notes",
            "",
        )
        .unwrap_err();
        assert_eq!(err, DescriptorError::EmptyCrate);
    }

    #[test]
    fn valid_descriptor_constructs() {
        let d = TemplateDescriptor::new(
            SINGH_SABI,
            "Singh-Sabi",
            "NFT",
            serde_json::json!({"initial_energy": 1000, "half_life": 100}),
            "wabi-sabi-anchored Patina NFT",
            "evaporchain-singh-sabi",
        )
        .unwrap();
        assert_eq!(d.class, SINGH_SABI);
        assert_eq!(d.brand, "Singh-Sabi");
    }

    #[test]
    fn round_trip_serde() {
        let d = TemplateDescriptor::new(
            SINGH_SABI,
            "Singh-Sabi",
            "NFT",
            serde_json::json!({"k": "v"}),
            "notes",
            "crate",
        )
        .unwrap();
        let s = serde_json::to_string(&d).unwrap();
        let back: TemplateDescriptor = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }
}
