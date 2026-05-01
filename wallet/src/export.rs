//! Universal Export — formatted receipts, CSV history, JSON state dumps, account summaries.
//!
//! Produces audit-ready outputs in multiple formats. Every piece of wallet
//! state can be exported for accountants, regulators, or personal records.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum ExportError {
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("no data to export")]
    NoData,
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        ExportError::Io(e.to_string())
    }
}
impl From<serde_json::Error> for ExportError {
    fn from(e: serde_json::Error) -> Self {
        ExportError::Json(e.to_string())
    }
}

/// Export format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Json,
    Text,
}

impl ExportFormat {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "csv" => Some(ExportFormat::Csv),
            "json" => Some(ExportFormat::Json),
            "text" | "txt" => Some(ExportFormat::Text),
            _ => None,
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
            ExportFormat::Text => "txt",
        }
    }
}

/// A transaction receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub tx_hash: String,
    pub tx_type: String,
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub fee: u64,
    pub timestamp: String,
    pub block_height: Option<u64>,
    pub status: String,
    pub notes: String,
}

impl Receipt {
    /// Format as a human-readable text receipt.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str("═══════════════════════════════════════════\n");
        s.push_str("         EVAPORCHAIN TRANSACTION RECEIPT    \n");
        s.push_str("═══════════════════════════════════════════\n");
        s.push_str(&format!("  TX Hash:    {}\n", self.tx_hash));
        s.push_str(&format!("  Type:       {}\n", self.tx_type));
        s.push_str(&format!("  From:       {}\n", self.from));
        s.push_str(&format!("  To:         {}\n", self.to));
        s.push_str(&format!("  Amount:     {} EVAP\n", self.amount));
        s.push_str(&format!("  Fee:        {} EVAP\n", self.fee));
        s.push_str(&format!("  Date:       {}\n", &self.timestamp[..19]));
        if let Some(h) = self.block_height {
            s.push_str(&format!("  Block:      {}\n", h));
        }
        s.push_str(&format!("  Status:     {}\n", self.status));
        if !self.notes.is_empty() {
            s.push_str(&format!("  Notes:      {}\n", self.notes));
        }
        s.push_str("═══════════════════════════════════════════\n");
        s
    }

    /// Format as CSV row.
    pub fn to_csv_row(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{}\n",
            self.tx_hash,
            self.tx_type,
            self.from,
            self.to,
            self.amount,
            self.fee,
            &self.timestamp[..19],
            self.block_height.map_or("".into(), |h| h.to_string()),
            self.status
        )
    }
}

/// Account summary for export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountSummary {
    pub address: String,
    pub name: String,
    pub balance: u64,
    pub total_sent: u64,
    pub total_received: u64,
    pub total_energy_spent: u64,
    pub total_gas_spent: u64,
    pub object_count: usize,
    pub nft_count: usize,
    pub token_count: usize,
    pub created_at: String,
    pub exported_at: String,
}

impl AccountSummary {
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str("═══════════════════════════════════════════\n");
        s.push_str("           EVAPORCHAIN ACCOUNT SUMMARY      \n");
        s.push_str("═══════════════════════════════════════════\n");
        s.push_str(&format!("  Name:       {}\n", self.name));
        s.push_str(&format!("  Address:    {}\n", self.address));
        s.push_str(&format!("  Balance:    {} EVAP\n", self.balance));
        s.push_str(&format!("  Total Sent: {} EVAP\n", self.total_sent));
        s.push_str(&format!("  Total Recv: {} EVAP\n", self.total_received));
        s.push_str(&format!(
            "  Energy:     {} EVAP spent\n",
            self.total_energy_spent
        ));
        s.push_str(&format!("  Gas Fees:   {} EVAP\n", self.total_gas_spent));
        s.push_str(&format!("  Objects:    {}\n", self.object_count));
        s.push_str(&format!("  NFTs:       {}\n", self.nft_count));
        s.push_str(&format!("  Tokens:     {}\n", self.token_count));
        s.push_str(&format!("  Created:    {}\n", self.created_at));
        s.push_str(&format!("  Exported:   {}\n", &self.exported_at[..19]));
        s.push_str("═══════════════════════════════════════════\n");
        s
    }
}

/// A row in the history export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRow {
    pub timestamp: String,
    pub tx_type: String,
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub fee: u64,
    pub status: String,
    pub reference: String,
}

// ──────────────────────────── Exporter ───────────────────────────────────

/// The export engine.
#[derive(Debug, Clone)]
pub struct Exporter;

impl Exporter {
    /// Export receipts to file.
    pub fn export_receipts<P: AsRef<Path>>(
        receipts: &[Receipt],
        path: P,
        format: ExportFormat,
    ) -> Result<usize, ExportError> {
        if receipts.is_empty() {
            return Err(ExportError::NoData);
        }
        let content = match format {
            ExportFormat::Json => serde_json::to_string_pretty(receipts)?,
            ExportFormat::Csv => {
                let mut csv =
                    String::from("tx_hash,type,from,to,amount,fee,timestamp,block,status\n");
                for r in receipts {
                    csv.push_str(&r.to_csv_row());
                }
                csv
            }
            ExportFormat::Text => {
                let mut text = String::new();
                for r in receipts {
                    text.push_str(&r.to_text());
                    text.push('\n');
                }
                text
            }
        };
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &content)?;
        Ok(receipts.len())
    }

    /// Export history rows to file.
    pub fn export_history<P: AsRef<Path>>(
        rows: &[HistoryRow],
        path: P,
        format: ExportFormat,
    ) -> Result<usize, ExportError> {
        if rows.is_empty() {
            return Err(ExportError::NoData);
        }
        let content = match format {
            ExportFormat::Json => serde_json::to_string_pretty(rows)?,
            ExportFormat::Csv => {
                let mut csv = String::from("timestamp,type,from,to,amount,fee,status,reference\n");
                for r in rows {
                    csv.push_str(&format!(
                        "{},{},{},{},{},{},{},{}\n",
                        r.timestamp,
                        r.tx_type,
                        r.from,
                        r.to,
                        r.amount,
                        r.fee,
                        r.status,
                        r.reference
                    ));
                }
                csv
            }
            ExportFormat::Text => {
                let mut text = String::new();
                for r in rows {
                    text.push_str(&format!(
                        "{} | {:12} | {} -> {} | {} EVAP | {}\n",
                        &r.timestamp[..19],
                        r.tx_type,
                        &r.from[..10.min(r.from.len())],
                        &r.to[..10.min(r.to.len())],
                        r.amount,
                        r.status
                    ));
                }
                text
            }
        };
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &content)?;
        Ok(rows.len())
    }

    /// Export account summary to file.
    pub fn export_summary<P: AsRef<Path>>(
        summary: &AccountSummary,
        path: P,
        format: ExportFormat,
    ) -> Result<(), ExportError> {
        let content = match format {
            ExportFormat::Json => serde_json::to_string_pretty(summary)?,
            ExportFormat::Csv => {
                format!(
                    "address,name,balance,total_sent,total_received,energy_spent,gas_spent,objects,nfts,tokens\n{},{},{},{},{},{},{},{},{},{}\n",
                    summary.address, summary.name, summary.balance,
                    summary.total_sent, summary.total_received,
                    summary.total_energy_spent, summary.total_gas_spent,
                    summary.object_count, summary.nft_count, summary.token_count
                )
            }
            ExportFormat::Text => summary.to_text(),
        };
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &content)?;
        Ok(())
    }

    /// Export a full wallet state dump (JSON only).
    pub fn export_state_dump<P: AsRef<Path>>(
        data: &HashMap<String, serde_json::Value>,
        path: P,
    ) -> Result<(), ExportError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(data)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Detect format from file extension.
    pub fn detect_format(path: &str) -> ExportFormat {
        if path.ends_with(".csv") {
            ExportFormat::Csv
        } else if path.ends_with(".json") {
            ExportFormat::Json
        } else {
            ExportFormat::Text
        }
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_receipt() -> Receipt {
        Receipt {
            tx_hash: "0xabc123".into(),
            tx_type: "transfer".into(),
            from: "0xalice".into(),
            to: "0xbob".into(),
            amount: 1000,
            fee: 21,
            timestamp: "2025-06-15T10:30:00Z".into(),
            block_height: Some(42000),
            status: "confirmed".into(),
            notes: "Monthly rent".into(),
        }
    }

    fn make_history() -> Vec<HistoryRow> {
        vec![
            HistoryRow {
                timestamp: "2025-06-15T10:30:00Z".into(),
                tx_type: "transfer".into(),
                from: "0xalice".into(),
                to: "0xbob".into(),
                amount: 1000,
                fee: 21,
                status: "confirmed".into(),
                reference: "tx_001".into(),
            },
            HistoryRow {
                timestamp: "2025-06-16T14:00:00Z".into(),
                tx_type: "refresh".into(),
                from: "0xalice".into(),
                to: "obj_42".into(),
                amount: 500,
                fee: 30,
                status: "confirmed".into(),
                reference: "tx_002".into(),
            },
        ]
    }

    fn make_summary() -> AccountSummary {
        AccountSummary {
            address: "0xalice123".into(),
            name: "alice".into(),
            balance: 50000,
            total_sent: 10000,
            total_received: 60000,
            total_energy_spent: 2000,
            total_gas_spent: 500,
            object_count: 5,
            nft_count: 3,
            token_count: 2,
            created_at: "2025-01-01T00:00:00Z".into(),
            exported_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn test_receipt_to_text() {
        let r = make_receipt();
        let text = r.to_text();
        assert!(text.contains("0xabc123"));
        assert!(text.contains("1000 EVAP"));
        assert!(text.contains("Monthly rent"));
        assert!(text.contains("42000"));
    }

    #[test]
    fn test_receipt_to_csv_row() {
        let r = make_receipt();
        let csv = r.to_csv_row();
        assert!(csv.contains("0xabc123"));
        assert!(csv.contains("1000"));
        assert!(csv.contains("confirmed"));
    }

    #[test]
    fn test_summary_to_text() {
        let s = make_summary();
        let text = s.to_text();
        assert!(text.contains("alice"));
        assert!(text.contains("50000 EVAP"));
        assert!(text.contains("Objects:    5"));
    }

    #[test]
    fn test_export_receipts_json() {
        let dir = std::env::temp_dir().join("evap_export_test_json");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("receipts.json");

        let receipts = vec![make_receipt()];
        let count = Exporter::export_receipts(&receipts, &path, ExportFormat::Json).unwrap();
        assert_eq!(count, 1);
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("0xabc123"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_export_receipts_csv() {
        let dir = std::env::temp_dir().join("evap_export_test_csv");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("receipts.csv");

        let receipts = vec![make_receipt()];
        let count = Exporter::export_receipts(&receipts, &path, ExportFormat::Csv).unwrap();
        assert_eq!(count, 1);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("tx_hash,"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_export_receipts_text() {
        let dir = std::env::temp_dir().join("evap_export_test_txt");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("receipts.txt");

        let receipts = vec![make_receipt()];
        Exporter::export_receipts(&receipts, &path, ExportFormat::Text).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("TRANSACTION RECEIPT"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_export_receipts_empty() {
        let dir = std::env::temp_dir().join("evap_export_empty");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("empty.json");
        let err = Exporter::export_receipts(&[], &path, ExportFormat::Json);
        assert!(err.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_export_history_csv() {
        let dir = std::env::temp_dir().join("evap_export_hist");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history.csv");

        let rows = make_history();
        let count = Exporter::export_history(&rows, &path, ExportFormat::Csv).unwrap();
        assert_eq!(count, 2);
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3); // header + 2

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_export_history_text() {
        let dir = std::env::temp_dir().join("evap_export_hist_txt");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history.txt");

        let rows = make_history();
        Exporter::export_history(&rows, &path, ExportFormat::Text).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("transfer"));
        assert!(content.contains("refresh"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_export_summary_json() {
        let dir = std::env::temp_dir().join("evap_export_sum");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("summary.json");

        let summary = make_summary();
        Exporter::export_summary(&summary, &path, ExportFormat::Json).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("alice"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_export_summary_csv() {
        let dir = std::env::temp_dir().join("evap_export_sum_csv");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("summary.csv");

        let summary = make_summary();
        Exporter::export_summary(&summary, &path, ExportFormat::Csv).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("address,"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_export_state_dump() {
        let dir = std::env::temp_dir().join("evap_export_dump");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("dump.json");

        let mut data = HashMap::new();
        data.insert("version".into(), serde_json::json!("1.0.0"));
        data.insert("accounts".into(), serde_json::json!(["alice", "bob"]));
        data.insert("config".into(), serde_json::json!({"node": "localhost"}));

        Exporter::export_state_dump(&data, &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("1.0.0"));
        assert!(content.contains("alice"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_detect_format() {
        assert_eq!(Exporter::detect_format("output.csv"), ExportFormat::Csv);
        assert_eq!(Exporter::detect_format("output.json"), ExportFormat::Json);
        assert_eq!(Exporter::detect_format("output.txt"), ExportFormat::Text);
        assert_eq!(Exporter::detect_format("output"), ExportFormat::Text);
    }

    #[test]
    fn test_export_format_from_str() {
        assert_eq!(ExportFormat::from_str("csv"), Some(ExportFormat::Csv));
        assert_eq!(ExportFormat::from_str("JSON"), Some(ExportFormat::Json));
        assert_eq!(ExportFormat::from_str("txt"), Some(ExportFormat::Text));
        assert_eq!(ExportFormat::from_str("pdf"), None);
    }

    #[test]
    fn test_export_format_extension() {
        assert_eq!(ExportFormat::Csv.extension(), "csv");
        assert_eq!(ExportFormat::Json.extension(), "json");
        assert_eq!(ExportFormat::Text.extension(), "txt");
    }

    #[test]
    fn test_receipt_without_block() {
        let mut r = make_receipt();
        r.block_height = None;
        r.notes = String::new();
        let text = r.to_text();
        assert!(!text.contains("Block:"));
        assert!(!text.contains("Notes:"));
    }

    #[test]
    fn test_multiple_receipts_export() {
        let dir = std::env::temp_dir().join("evap_export_multi");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("multi.txt");

        let r1 = make_receipt();
        let mut r2 = make_receipt();
        r2.tx_hash = "0xdef456".into();
        r2.amount = 2000;
        let receipts = vec![r1, r2];

        let count = Exporter::export_receipts(&receipts, &path, ExportFormat::Text).unwrap();
        assert_eq!(count, 2);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("0xabc123"));
        assert!(content.contains("0xdef456"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
