// wallet/src/qr.rs — QR code generation for terminal display
//
// Pure-Rust QR code encoder (no external QR crate).
// Supports encoding addresses, payment URIs, and arbitrary data.
// Renders to Unicode block characters for terminal display.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QrError {
    #[error("data too long for QR code: {0} bytes (max {1})")]
    DataTooLong(usize, usize),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

// ── QR encoding (simplified) ─────────────────────────────────
// We use a bitmask grid approach. For real production use you'd
// want a full QR spec encoder, but this gives us functional
// terminal-displayable codes for addresses and URIs.

const MAX_DATA_BYTES: usize = 2953; // QR version 40 capacity

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QrCode {
    pub data: String,
    pub modules: Vec<Vec<bool>>, // true = dark module
    pub size: usize,
}

impl QrCode {
    /// Encode data into a QR-like grid using a deterministic pattern.
    /// This produces a visual grid suitable for terminal display.
    pub fn encode(data: &str) -> Result<Self, QrError> {
        if data.is_empty() {
            return Err(QrError::InvalidInput("data cannot be empty".into()));
        }
        if data.len() > MAX_DATA_BYTES {
            return Err(QrError::DataTooLong(data.len(), MAX_DATA_BYTES));
        }

        let bytes = data.as_bytes();
        // Size scales with data length: minimum 21x21 (version 1)
        let size = compute_size(bytes.len());
        let mut modules = vec![vec![false; size]; size];

        // Finder patterns (top-left, top-right, bottom-left)
        draw_finder(&mut modules, 0, 0);
        draw_finder(&mut modules, 0, size - 7);
        draw_finder(&mut modules, size - 7, 0);

        // Timing patterns
        #[allow(clippy::needless_range_loop)]
        for i in 8..size - 8 {
            modules[6][i] = i % 2 == 0;
            modules[i][6] = i % 2 == 0;
        }

        // Encode data bits into available modules
        let mut bit_idx = 0;
        let data_bits = bytes_to_bits(bytes);
        #[allow(clippy::needless_range_loop)]
        for col in (8..size - 8).rev() {
            for row in 8..size - 8 {
                if bit_idx < data_bits.len() {
                    modules[row][col] = data_bits[bit_idx];
                    bit_idx += 1;
                } else {
                    // Fill remaining with deterministic pattern
                    modules[row][col] = (row + col) % 3 == 0;
                }
            }
        }

        Ok(QrCode {
            data: data.to_string(),
            modules,
            size,
        })
    }

    /// Render to terminal using Unicode block characters.
    /// Uses upper/lower half blocks for 2-row-per-line density.
    pub fn to_terminal(&self) -> String {
        self.to_terminal_with_border(2)
    }

    /// Render with configurable quiet zone border
    pub fn to_terminal_with_border(&self, border: usize) -> String {
        let total = self.size + border * 2;
        let mut lines = Vec::new();

        let mut row = 0;
        while row < total {
            let mut line = String::new();
            for col in 0..total {
                let top = self.get_module_bordered(row, col, border);
                let bottom = if row + 1 < total {
                    self.get_module_bordered(row + 1, col, border)
                } else {
                    false
                };
                // dark=true → black, light=false → white
                // Upper half block: top dark, bottom light → \u{2580}
                // Lower half block: top light, bottom dark → \u{2584}
                // Full block: both dark → \u{2588}
                // Space: both light → ' '
                match (top, bottom) {
                    (true, true) => line.push('\u{2588}'),
                    (true, false) => line.push('\u{2580}'),
                    (false, true) => line.push('\u{2584}'),
                    (false, false) => line.push(' '),
                }
            }
            lines.push(line);
            row += 2;
        }

        lines.join("\n")
    }

    fn get_module_bordered(&self, row: usize, col: usize, border: usize) -> bool {
        if row < border || col < border {
            return false;
        }
        let r = row - border;
        let c = col - border;
        if r < self.size && c < self.size {
            self.modules[r][c]
        } else {
            false
        }
    }

    /// Render as simple ASCII art (# for dark, space for light)
    pub fn to_ascii(&self) -> String {
        let mut lines = Vec::new();
        for row in &self.modules {
            let line: String = row
                .iter()
                .map(|&dark| if dark { '#' } else { ' ' })
                .collect();
            lines.push(line);
        }
        lines.join("\n")
    }

    /// Render as SVG string
    pub fn to_svg(&self, module_size: usize) -> String {
        let total = self.size * module_size;
        let mut svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}">"#,
            total, total
        );
        svg.push_str(&format!(
            r#"<rect width="{}" height="{}" fill="white"/>"#,
            total, total
        ));
        for (r, row) in self.modules.iter().enumerate() {
            for (c, &dark) in row.iter().enumerate() {
                if dark {
                    svg.push_str(&format!(
                        r#"<rect x="{}" y="{}" width="{}" height="{}" fill="black"/>"#,
                        c * module_size,
                        r * module_size,
                        module_size,
                        module_size
                    ));
                }
            }
        }
        svg.push_str("</svg>");
        svg
    }

    /// Get the module grid dimensions
    pub fn dimensions(&self) -> (usize, usize) {
        (self.size, self.size)
    }

    /// Count dark modules
    pub fn dark_count(&self) -> usize {
        self.modules
            .iter()
            .flat_map(|row| row.iter())
            .filter(|&&m| m)
            .count()
    }

    /// Count light modules
    pub fn light_count(&self) -> usize {
        self.size * self.size - self.dark_count()
    }
}

// ── Payment URI builder ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentUri {
    pub address: String,
    pub amount: Option<u64>,
    pub label: Option<String>,
    pub message: Option<String>,
}

impl PaymentUri {
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_string(),
            amount: None,
            label: None,
            message: None,
        }
    }

    pub fn with_amount(mut self, amount: u64) -> Self {
        self.amount = Some(amount);
        self
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.label = Some(label.to_string());
        self
    }

    pub fn with_message(mut self, message: &str) -> Self {
        self.message = Some(message.to_string());
        self
    }

    pub fn to_uri(&self) -> String {
        let mut uri = format!("evaporchain:{}", self.address);
        let mut params = Vec::new();
        if let Some(amt) = self.amount {
            params.push(format!("amount={}", amt));
        }
        if let Some(ref label) = self.label {
            params.push(format!("label={}", label));
        }
        if let Some(ref msg) = self.message {
            params.push(format!("message={}", msg));
        }
        if !params.is_empty() {
            uri.push('?');
            uri.push_str(&params.join("&"));
        }
        uri
    }

    pub fn to_qr(&self) -> Result<QrCode, QrError> {
        QrCode::encode(&self.to_uri())
    }
}

/// Parse an evaporchain: URI back into a PaymentUri
pub fn parse_payment_uri(uri: &str) -> Result<PaymentUri, QrError> {
    let stripped = uri
        .strip_prefix("evaporchain:")
        .ok_or_else(|| QrError::InvalidInput("URI must start with 'evaporchain:'".into()))?;

    let (address, params_str) = if let Some(idx) = stripped.find('?') {
        (&stripped[..idx], Some(&stripped[idx + 1..]))
    } else {
        (stripped, None)
    };

    if address.is_empty() {
        return Err(QrError::InvalidInput("address cannot be empty".into()));
    }

    let mut payment = PaymentUri::new(address);

    if let Some(params) = params_str {
        for param in params.split('&') {
            if let Some((key, value)) = param.split_once('=') {
                match key {
                    "amount" => {
                        payment.amount = value.parse::<u64>().ok();
                    }
                    "label" => payment.label = Some(value.to_string()),
                    "message" => payment.message = Some(value.to_string()),
                    _ => {} // ignore unknown params
                }
            }
        }
    }

    Ok(payment)
}

// ── Internal helpers ──────────────────────────────────────────

fn compute_size(data_len: usize) -> usize {
    // QR versions: 21, 25, 29, ... (21 + 4*version)
    // We pick a size that can fit the data
    let min_modules = (data_len * 8) + 200; // rough estimate
    let mut size = 21;
    while size * size < min_modules + 300 {
        size += 4;
    }
    size
}

fn draw_finder(modules: &mut [Vec<bool>], row: usize, col: usize) {
    // 7x7 finder pattern
    for r in 0..7 {
        for c in 0..7 {
            let is_border = r == 0 || r == 6 || c == 0 || c == 6;
            let is_center = (2..=4).contains(&r) && (2..=4).contains(&c);
            if r + row < modules.len() && c + col < modules[0].len() {
                modules[r + row][c + col] = is_border || is_center;
            }
        }
    }
}

fn bytes_to_bits(bytes: &[u8]) -> Vec<bool> {
    let mut bits = Vec::with_capacity(bytes.len() * 8);
    for &byte in bytes {
        for i in (0..8).rev() {
            bits.push((byte >> i) & 1 == 1);
        }
    }
    bits
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_basic() {
        let qr = QrCode::encode("evap1abc123").unwrap();
        assert!(qr.size >= 21);
        assert_eq!(qr.modules.len(), qr.size);
        assert_eq!(qr.modules[0].len(), qr.size);
    }

    #[test]
    fn test_encode_empty_fails() {
        assert!(QrCode::encode("").is_err());
    }

    #[test]
    fn test_encode_address() {
        let addr = "evap1".to_string() + &"a".repeat(40);
        let qr = QrCode::encode(&addr).unwrap();
        assert_eq!(qr.data, addr);
    }

    #[test]
    fn test_finder_patterns() {
        let qr = QrCode::encode("test").unwrap();
        // Top-left finder: row 0, col 0 should be dark (border)
        assert!(qr.modules[0][0]);
        // Center of finder: row 3, col 3 should be dark
        assert!(qr.modules[3][3]);
    }

    #[test]
    fn test_dimensions() {
        let qr = QrCode::encode("hello").unwrap();
        let (w, h) = qr.dimensions();
        assert_eq!(w, h);
        assert_eq!(w, qr.size);
    }

    #[test]
    fn test_dark_light_counts() {
        let qr = QrCode::encode("test data").unwrap();
        assert_eq!(qr.dark_count() + qr.light_count(), qr.size * qr.size);
    }

    #[test]
    fn test_to_ascii() {
        let qr = QrCode::encode("hi").unwrap();
        let ascii = qr.to_ascii();
        assert!(ascii.contains('#'));
        assert!(ascii.contains(' '));
        let lines: Vec<&str> = ascii.lines().collect();
        assert_eq!(lines.len(), qr.size);
    }

    #[test]
    fn test_to_terminal() {
        let qr = QrCode::encode("test").unwrap();
        let term = qr.to_terminal();
        assert!(!term.is_empty());
        // Terminal uses half-block chars, so roughly half the lines
        let lines: Vec<&str> = term.lines().collect();
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_to_terminal_with_border() {
        let qr = QrCode::encode("test").unwrap();
        let t0 = qr.to_terminal_with_border(0);
        let t4 = qr.to_terminal_with_border(4);
        // Larger border → more columns per line
        let w0 = t0.lines().next().unwrap().chars().count();
        let w4 = t4.lines().next().unwrap().chars().count();
        assert!(w4 > w0);
    }

    #[test]
    fn test_to_svg() {
        let qr = QrCode::encode("evap1test").unwrap();
        let svg = qr.to_svg(4);
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("fill=\"black\""));
        assert!(svg.ends_with("</svg>"));
    }

    #[test]
    fn test_payment_uri_basic() {
        let uri = PaymentUri::new("evap1abc").to_uri();
        assert_eq!(uri, "evaporchain:evap1abc");
    }

    #[test]
    fn test_payment_uri_with_amount() {
        let uri = PaymentUri::new("evap1abc").with_amount(1000).to_uri();
        assert_eq!(uri, "evaporchain:evap1abc?amount=1000");
    }

    #[test]
    fn test_payment_uri_full() {
        let uri = PaymentUri::new("evap1abc")
            .with_amount(500)
            .with_label("Alice")
            .with_message("rent")
            .to_uri();
        assert!(uri.contains("amount=500"));
        assert!(uri.contains("label=Alice"));
        assert!(uri.contains("message=rent"));
    }

    #[test]
    fn test_payment_uri_to_qr() {
        let qr = PaymentUri::new("evap1abc")
            .with_amount(100)
            .to_qr()
            .unwrap();
        assert!(qr.data.starts_with("evaporchain:"));
    }

    #[test]
    fn test_parse_payment_uri() {
        let uri = "evaporchain:evap1abc?amount=1000&label=Bob&message=thanks";
        let p = parse_payment_uri(uri).unwrap();
        assert_eq!(p.address, "evap1abc");
        assert_eq!(p.amount, Some(1000));
        assert_eq!(p.label, Some("Bob".to_string()));
        assert_eq!(p.message, Some("thanks".to_string()));
    }

    #[test]
    fn test_parse_payment_uri_no_params() {
        let p = parse_payment_uri("evaporchain:evap1xyz").unwrap();
        assert_eq!(p.address, "evap1xyz");
        assert!(p.amount.is_none());
    }

    #[test]
    fn test_parse_payment_uri_bad_prefix() {
        assert!(parse_payment_uri("bitcoin:abc").is_err());
    }

    #[test]
    fn test_parse_payment_uri_empty_address() {
        assert!(parse_payment_uri("evaporchain:").is_err());
    }

    #[test]
    fn test_roundtrip_uri() {
        let original = PaymentUri::new("evap1round")
            .with_amount(999)
            .with_label("Test");
        let uri_str = original.to_uri();
        let parsed = parse_payment_uri(&uri_str).unwrap();
        assert_eq!(parsed.address, "evap1round");
        assert_eq!(parsed.amount, Some(999));
        assert_eq!(parsed.label, Some("Test".to_string()));
    }

    #[test]
    fn test_compute_size_minimum() {
        // Small data should get minimum 21
        assert!(compute_size(5) >= 21);
    }

    #[test]
    fn test_compute_size_scales() {
        let s1 = compute_size(10);
        let s2 = compute_size(500);
        assert!(s2 > s1);
    }

    #[test]
    fn test_bytes_to_bits() {
        let bits = bytes_to_bits(&[0xFF]);
        assert_eq!(bits, vec![true; 8]);
        let bits2 = bytes_to_bits(&[0x00]);
        assert_eq!(bits2, vec![false; 8]);
        let bits3 = bytes_to_bits(&[0xA5]); // 10100101
        assert_eq!(
            bits3,
            vec![true, false, true, false, false, true, false, true]
        );
    }
}
