use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum PnlError {
    #[error("insufficient balance: need {needed}, have {available}")]
    InsufficientBalance { needed: u64, available: u64 },
    #[error("token not found: {0}")]
    TokenNotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CostBasisMethod {
    Fifo,
    Lifo,
    Hifo,
    AvgCost,
}

impl Default for CostBasisMethod {
    fn default() -> Self {
        CostBasisMethod::Fifo
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LotStatus {
    Open,
    PartiallySold,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradeType {
    Buy,
    Sell,
    Swap,
    Airdrop,
    Stake,
    Unstake,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeLot {
    pub id: String,
    pub token: String,
    pub trade_type: TradeType,
    pub amount: u64,
    pub remaining: u64,
    pub cost_per_unit: f64,
    pub total_cost: f64,
    pub acquired_at: String,
    pub status: LotStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaleRecord {
    pub id: String,
    pub token: String,
    pub amount: u64,
    pub sale_price_per_unit: f64,
    pub cost_basis_per_unit: f64,
    pub realized_pnl: f64,
    pub method: CostBasisMethod,
    pub sold_at: String,
    pub lot_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPnl {
    pub token: String,
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
    pub total_cost_basis: f64,
    pub current_value: f64,
    pub total_bought: u64,
    pub total_sold: u64,
    pub avg_buy_price: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PnlStats {
    pub total_tokens_tracked: usize,
    pub total_lots: usize,
    pub open_lots: usize,
    pub total_sales: usize,
    pub total_realized_pnl: f64,
    pub total_unrealized_pnl: f64,
    pub best_trade: f64,
    pub worst_trade: f64,
}

// ---------------------------------------------------------------------------
// Main Store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PnlTracker {
    pub lots: HashMap<String, TradeLot>,
    pub sales: Vec<SaleRecord>,
    pub default_method: CostBasisMethod,
}

impl PnlTracker {
    // -- constructor --------------------------------------------------------

    pub fn new() -> Self {
        Self::default()
    }

    // -- record_buy ---------------------------------------------------------

    pub fn record_buy(
        &mut self,
        token: &str,
        amount: u64,
        price_per_unit: f64,
        trade_type: TradeType,
    ) -> String {
        let id = format!("lot-{}", uuid_v4());
        let lot = TradeLot {
            id: id.clone(),
            token: token.to_string(),
            trade_type,
            amount,
            remaining: amount,
            cost_per_unit: price_per_unit,
            total_cost: price_per_unit * amount as f64,
            acquired_at: Utc::now().to_rfc3339(),
            status: LotStatus::Open,
        };
        self.lots.insert(id.clone(), lot);
        id
    }

    // -- record_sale --------------------------------------------------------

    pub fn record_sale(
        &mut self,
        token: &str,
        amount: u64,
        sale_price: f64,
        method: Option<CostBasisMethod>,
    ) -> Result<SaleRecord, PnlError> {
        let method = method.unwrap_or_else(|| self.default_method.clone());

        // Gather open lots for this token
        let mut open_lot_ids: Vec<String> = self
            .lots
            .values()
            .filter(|l| l.token == token && l.remaining > 0)
            .map(|l| l.id.clone())
            .collect();

        let available: u64 = open_lot_ids
            .iter()
            .map(|id| self.lots[id].remaining)
            .sum();

        if available < amount {
            return Err(PnlError::InsufficientBalance {
                needed: amount,
                available,
            });
        }

        if open_lot_ids.is_empty() {
            return Err(PnlError::TokenNotFound(token.to_string()));
        }

        // Sort lots according to method
        match method {
            CostBasisMethod::Fifo => {
                open_lot_ids.sort_by(|a, b| {
                    self.lots[a].acquired_at.cmp(&self.lots[b].acquired_at)
                });
            }
            CostBasisMethod::Lifo => {
                open_lot_ids.sort_by(|a, b| {
                    self.lots[b].acquired_at.cmp(&self.lots[a].acquired_at)
                });
            }
            CostBasisMethod::Hifo => {
                open_lot_ids.sort_by(|a, b| {
                    self.lots[b]
                        .cost_per_unit
                        .partial_cmp(&self.lots[a].cost_per_unit)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            CostBasisMethod::AvgCost => {
                // For avg cost we consume proportionally, but simplify by
                // using the weighted average cost across all open lots.
            }
        }

        let mut remaining_to_sell = amount;
        let mut total_cost_basis = 0.0_f64;
        let mut used_lot_ids: Vec<String> = Vec::new();

        if method == CostBasisMethod::AvgCost {
            // Weighted average cost
            let total_remaining: u64 = open_lot_ids
                .iter()
                .map(|id| self.lots[id].remaining)
                .sum();
            let total_cost: f64 = open_lot_ids
                .iter()
                .map(|id| {
                    let lot = &self.lots[id];
                    lot.cost_per_unit * lot.remaining as f64
                })
                .sum();
            let avg = total_cost / total_remaining as f64;
            total_cost_basis = avg * amount as f64;

            // Deduct from lots in order
            for lot_id in &open_lot_ids {
                if remaining_to_sell == 0 {
                    break;
                }
                let lot = self.lots.get_mut(lot_id).unwrap();
                let deduct = remaining_to_sell.min(lot.remaining);
                lot.remaining -= deduct;
                remaining_to_sell -= deduct;
                used_lot_ids.push(lot_id.clone());
                lot.status = if lot.remaining == 0 {
                    LotStatus::Closed
                } else {
                    LotStatus::PartiallySold
                };
            }
        } else {
            for lot_id in &open_lot_ids {
                if remaining_to_sell == 0 {
                    break;
                }
                let lot = self.lots.get_mut(lot_id).unwrap();
                let deduct = remaining_to_sell.min(lot.remaining);
                total_cost_basis += lot.cost_per_unit * deduct as f64;
                lot.remaining -= deduct;
                remaining_to_sell -= deduct;
                used_lot_ids.push(lot_id.clone());
                lot.status = if lot.remaining == 0 {
                    LotStatus::Closed
                } else {
                    LotStatus::PartiallySold
                };
            }
        }

        let cost_basis_per_unit = total_cost_basis / amount as f64;
        let realized_pnl = (sale_price - cost_basis_per_unit) * amount as f64;

        let record = SaleRecord {
            id: format!("sale-{}", uuid_v4()),
            token: token.to_string(),
            amount,
            sale_price_per_unit: sale_price,
            cost_basis_per_unit,
            realized_pnl,
            method,
            sold_at: Utc::now().to_rfc3339(),
            lot_ids: used_lot_ids,
        };

        self.sales.push(record.clone());
        Ok(record)
    }

    // -- unrealized_pnl -----------------------------------------------------

    pub fn unrealized_pnl(&self, token: &str, current_price: f64) -> f64 {
        self.lots
            .values()
            .filter(|l| l.token == token && l.remaining > 0)
            .map(|l| (current_price - l.cost_per_unit) * l.remaining as f64)
            .sum()
    }

    // -- token_pnl ----------------------------------------------------------

    pub fn token_pnl(&self, token: &str, current_price: f64) -> TokenPnl {
        let realized: f64 = self
            .sales
            .iter()
            .filter(|s| s.token == token)
            .map(|s| s.realized_pnl)
            .sum();

        let unrealized = self.unrealized_pnl(token, current_price);
        let total_cost_basis = self.cost_basis(token);
        let total_remaining: u64 = self
            .lots
            .values()
            .filter(|l| l.token == token && l.remaining > 0)
            .map(|l| l.remaining)
            .sum();
        let current_value = current_price * total_remaining as f64;

        let total_bought: u64 = self
            .lots
            .values()
            .filter(|l| l.token == token)
            .map(|l| l.amount)
            .sum();

        let total_sold: u64 = self
            .sales
            .iter()
            .filter(|s| s.token == token)
            .map(|s| s.amount)
            .sum();

        let avg_buy = self.avg_cost(token);

        TokenPnl {
            token: token.to_string(),
            realized_pnl: realized,
            unrealized_pnl: unrealized,
            total_cost_basis,
            current_value,
            total_bought,
            total_sold,
            avg_buy_price: avg_buy,
        }
    }

    // -- all_token_pnl ------------------------------------------------------

    pub fn all_token_pnl(&self, prices: &HashMap<String, f64>) -> Vec<TokenPnl> {
        let mut tokens: std::collections::HashSet<String> = std::collections::HashSet::new();
        for lot in self.lots.values() {
            tokens.insert(lot.token.clone());
        }
        tokens
            .into_iter()
            .map(|t| {
                let price = prices.get(&t).copied().unwrap_or(0.0);
                self.token_pnl(&t, price)
            })
            .collect()
    }

    // -- open_lots ----------------------------------------------------------

    pub fn open_lots(&self, token: &str) -> Vec<&TradeLot> {
        self.lots
            .values()
            .filter(|l| l.token == token && l.remaining > 0)
            .collect()
    }

    // -- cost_basis ---------------------------------------------------------

    pub fn cost_basis(&self, token: &str) -> f64 {
        self.lots
            .values()
            .filter(|l| l.token == token && l.remaining > 0)
            .map(|l| l.cost_per_unit * l.remaining as f64)
            .sum()
    }

    // -- avg_cost -----------------------------------------------------------

    pub fn avg_cost(&self, token: &str) -> f64 {
        let total_remaining: u64 = self
            .lots
            .values()
            .filter(|l| l.token == token && l.remaining > 0)
            .map(|l| l.remaining)
            .sum();

        if total_remaining == 0 {
            return 0.0;
        }

        self.cost_basis(token) / total_remaining as f64
    }

    // -- sale_history -------------------------------------------------------

    pub fn sale_history(&self, token: &str) -> Vec<&SaleRecord> {
        self.sales.iter().filter(|s| s.token == token).collect()
    }

    // -- total_realized_pnl -------------------------------------------------

    pub fn total_realized_pnl(&self) -> f64 {
        self.sales.iter().map(|s| s.realized_pnl).sum()
    }

    // -- stats --------------------------------------------------------------

    pub fn stats(&self, prices: &HashMap<String, f64>) -> PnlStats {
        let all_pnl = self.all_token_pnl(prices);
        let total_unrealized: f64 = all_pnl.iter().map(|t| t.unrealized_pnl).sum();

        let open_lots = self.lots.values().filter(|l| l.remaining > 0).count();
        let total_lots = self.lots.len();

        let best_trade = self
            .sales
            .iter()
            .map(|s| s.realized_pnl)
            .fold(f64::NEG_INFINITY, f64::max);
        let worst_trade = self
            .sales
            .iter()
            .map(|s| s.realized_pnl)
            .fold(f64::INFINITY, f64::min);

        let mut tokens: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for lot in self.lots.values() {
            tokens.insert(&lot.token);
        }

        PnlStats {
            total_tokens_tracked: tokens.len(),
            total_lots,
            open_lots,
            total_sales: self.sales.len(),
            total_realized_pnl: self.total_realized_pnl(),
            total_unrealized_pnl: total_unrealized,
            best_trade: if self.sales.is_empty() {
                0.0
            } else {
                best_trade
            },
            worst_trade: if self.sales.is_empty() {
                0.0
            } else {
                worst_trade
            },
        }
    }

    // -- persistence --------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self, PnlError> {
        let data = std::fs::read_to_string(path)?;
        let tracker: Self = serde_json::from_str(&data)?;
        Ok(tracker)
    }

    pub fn save(&self, path: &Path) -> Result<(), PnlError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        d.as_nanos() as u32,
        (d.as_nanos() >> 32) as u16,
        ((d.as_nanos() >> 48) as u16 & 0x0fff) | 0x4000,
        ((d.as_nanos() >> 60) as u16 & 0x3fff) | 0x8000,
        (d.as_nanos() >> 64) as u64 ^ std::process::id() as u64,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::process::id;

    fn temp_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("pnl_test_{}_{}", id(), name))
    }

    #[test]
    fn test_new_tracker_is_empty() {
        let t = PnlTracker::new();
        assert!(t.lots.is_empty());
        assert!(t.sales.is_empty());
    }

    #[test]
    fn test_default_method_is_fifo() {
        let t = PnlTracker::new();
        assert_eq!(t.default_method, CostBasisMethod::Fifo);
    }

    #[test]
    fn test_record_buy() {
        let mut t = PnlTracker::new();
        let id = t.record_buy("EVAP", 100, 2.0, TradeType::Buy);
        assert!(id.starts_with("lot-"));
        assert_eq!(t.lots.len(), 1);
        let lot = t.lots.get(&id).unwrap();
        assert_eq!(lot.amount, 100);
        assert_eq!(lot.remaining, 100);
        assert_eq!(lot.cost_per_unit, 2.0);
        assert_eq!(lot.total_cost, 200.0);
        assert_eq!(lot.status, LotStatus::Open);
    }

    #[test]
    fn test_record_multiple_buys() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 2.0, TradeType::Buy);
        t.record_buy("EVAP", 50, 3.0, TradeType::Buy);
        t.record_buy("SOL", 10, 150.0, TradeType::Swap);
        assert_eq!(t.lots.len(), 3);
    }

    #[test]
    fn test_record_sale_fifo() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 1.0, TradeType::Buy);
        std::thread::sleep(std::time::Duration::from_millis(10));
        t.record_buy("EVAP", 100, 3.0, TradeType::Buy);

        let sale = t
            .record_sale("EVAP", 50, 5.0, Some(CostBasisMethod::Fifo))
            .unwrap();
        assert_eq!(sale.amount, 50);
        assert_eq!(sale.cost_basis_per_unit, 1.0); // FIFO takes the $1 lot first
        assert_eq!(sale.realized_pnl, (5.0 - 1.0) * 50.0);
    }

    #[test]
    fn test_record_sale_lifo() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 1.0, TradeType::Buy);
        std::thread::sleep(std::time::Duration::from_millis(10));
        t.record_buy("EVAP", 100, 3.0, TradeType::Buy);

        let sale = t
            .record_sale("EVAP", 50, 5.0, Some(CostBasisMethod::Lifo))
            .unwrap();
        assert_eq!(sale.cost_basis_per_unit, 3.0); // LIFO takes the $3 lot first
        assert_eq!(sale.realized_pnl, (5.0 - 3.0) * 50.0);
    }

    #[test]
    fn test_record_sale_hifo() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 1.0, TradeType::Buy);
        t.record_buy("EVAP", 100, 5.0, TradeType::Buy);
        t.record_buy("EVAP", 100, 3.0, TradeType::Buy);

        let sale = t
            .record_sale("EVAP", 50, 6.0, Some(CostBasisMethod::Hifo))
            .unwrap();
        assert_eq!(sale.cost_basis_per_unit, 5.0); // HIFO takes the $5 lot first
    }

    #[test]
    fn test_record_sale_avg_cost() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 2.0, TradeType::Buy);
        t.record_buy("EVAP", 100, 4.0, TradeType::Buy);

        let sale = t
            .record_sale("EVAP", 100, 5.0, Some(CostBasisMethod::AvgCost))
            .unwrap();
        // avg cost = (100*2 + 100*4) / 200 = 3.0
        assert!((sale.cost_basis_per_unit - 3.0).abs() < 1e-9);
        assert!((sale.realized_pnl - (5.0 - 3.0) * 100.0).abs() < 1e-9);
    }

    #[test]
    fn test_sale_insufficient_balance() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 50, 2.0, TradeType::Buy);
        let result = t.record_sale("EVAP", 100, 5.0, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_sale_token_not_found() {
        let mut t = PnlTracker::new();
        let result = t.record_sale("NONEXIST", 10, 5.0, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_lot_status_after_partial_sale() {
        let mut t = PnlTracker::new();
        let lot_id = t.record_buy("EVAP", 100, 2.0, TradeType::Buy);
        t.record_sale("EVAP", 30, 5.0, None).unwrap();
        let lot = t.lots.get(&lot_id).unwrap();
        assert_eq!(lot.remaining, 70);
        assert_eq!(lot.status, LotStatus::PartiallySold);
    }

    #[test]
    fn test_lot_status_after_full_sale() {
        let mut t = PnlTracker::new();
        let lot_id = t.record_buy("EVAP", 100, 2.0, TradeType::Buy);
        t.record_sale("EVAP", 100, 5.0, None).unwrap();
        let lot = t.lots.get(&lot_id).unwrap();
        assert_eq!(lot.remaining, 0);
        assert_eq!(lot.status, LotStatus::Closed);
    }

    #[test]
    fn test_unrealized_pnl() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 2.0, TradeType::Buy);
        let pnl = t.unrealized_pnl("EVAP", 5.0);
        assert!((pnl - 300.0).abs() < 1e-9); // (5-2)*100
    }

    #[test]
    fn test_unrealized_pnl_negative() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 10.0, TradeType::Buy);
        let pnl = t.unrealized_pnl("EVAP", 5.0);
        assert!((pnl - (-500.0)).abs() < 1e-9);
    }

    #[test]
    fn test_open_lots() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 2.0, TradeType::Buy);
        t.record_buy("EVAP", 50, 3.0, TradeType::Buy);
        t.record_buy("SOL", 10, 100.0, TradeType::Buy);
        let open = t.open_lots("EVAP");
        assert_eq!(open.len(), 2);
    }

    #[test]
    fn test_cost_basis() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 2.0, TradeType::Buy);
        t.record_buy("EVAP", 50, 4.0, TradeType::Buy);
        assert!((t.cost_basis("EVAP") - 400.0).abs() < 1e-9); // 200 + 200
    }

    #[test]
    fn test_avg_cost() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 2.0, TradeType::Buy);
        t.record_buy("EVAP", 100, 4.0, TradeType::Buy);
        assert!((t.avg_cost("EVAP") - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_avg_cost_empty() {
        let t = PnlTracker::new();
        assert_eq!(t.avg_cost("EVAP"), 0.0);
    }

    #[test]
    fn test_sale_history() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 200, 2.0, TradeType::Buy);
        t.record_sale("EVAP", 50, 3.0, None).unwrap();
        t.record_sale("EVAP", 50, 4.0, None).unwrap();
        assert_eq!(t.sale_history("EVAP").len(), 2);
        assert_eq!(t.sale_history("SOL").len(), 0);
    }

    #[test]
    fn test_total_realized_pnl() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 200, 2.0, TradeType::Buy);
        t.record_sale("EVAP", 100, 5.0, None).unwrap();
        // realized = (5-2)*100 = 300
        assert!((t.total_realized_pnl() - 300.0).abs() < 1e-9);
    }

    #[test]
    fn test_token_pnl() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 200, 2.0, TradeType::Buy);
        t.record_sale("EVAP", 100, 5.0, None).unwrap();
        let pnl = t.token_pnl("EVAP", 6.0);
        assert_eq!(pnl.total_bought, 200);
        assert_eq!(pnl.total_sold, 100);
        assert!((pnl.realized_pnl - 300.0).abs() < 1e-9);
        assert!((pnl.unrealized_pnl - 400.0).abs() < 1e-9); // (6-2)*100
        assert!((pnl.current_value - 600.0).abs() < 1e-9);
    }

    #[test]
    fn test_stats() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 200, 2.0, TradeType::Buy);
        t.record_buy("SOL", 10, 100.0, TradeType::Buy);
        t.record_sale("EVAP", 100, 5.0, None).unwrap();

        let mut prices = HashMap::new();
        prices.insert("EVAP".to_string(), 6.0);
        prices.insert("SOL".to_string(), 120.0);

        let stats = t.stats(&prices);
        assert_eq!(stats.total_tokens_tracked, 2);
        assert_eq!(stats.total_lots, 2);
        assert_eq!(stats.total_sales, 1);
        assert!((stats.total_realized_pnl - 300.0).abs() < 1e-9);
    }

    #[test]
    fn test_save_and_load() {
        let path = temp_path("save_load.json");
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 2.0, TradeType::Buy);
        t.save(&path).unwrap();

        let loaded = PnlTracker::load(&path).unwrap();
        assert_eq!(loaded.lots.len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("nonexistent.json");
        let t = PnlTracker::load_or_default(&path);
        assert!(t.lots.is_empty());
    }

    #[test]
    fn test_all_token_pnl() {
        let mut t = PnlTracker::new();
        t.record_buy("EVAP", 100, 2.0, TradeType::Buy);
        t.record_buy("SOL", 10, 100.0, TradeType::Buy);

        let mut prices = HashMap::new();
        prices.insert("EVAP".to_string(), 5.0);
        prices.insert("SOL".to_string(), 120.0);

        let all = t.all_token_pnl(&prices);
        assert_eq!(all.len(), 2);
    }
}
