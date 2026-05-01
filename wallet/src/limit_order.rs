// wallet/src/limit_order.rs — On-chain limit order engine
//
// Place, fill, cancel, and expire limit orders with support for
// stop-loss, take-profit, and trailing-stop trigger types.
// Builds aggregated order books and tracks fill history.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ── Errors ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum LimitOrderError {
    #[error("order already exists: {0}")]
    OrderAlreadyExists(String),
    #[error("order not found: {0}")]
    OrderNotFound(String),
    #[error("order not cancellable: {0}")]
    OrderNotCancellable(String),
    #[error("order not fillable: {0}")]
    OrderNotFillable(String),
    #[error("fill exceeds remaining amount: {0}")]
    FillExceedsRemaining(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Enums ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OrderStatus {
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OrderType {
    Limit,
    StopLoss,
    TakeProfit,
    TrailingStop(f64),
}

// ── Structs ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitOrder {
    pub id: String,
    pub token_from: String,
    pub token_to: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub amount: u64,
    pub filled_amount: u64,
    pub price: f64,
    pub trigger_price: Option<f64>,
    pub status: OrderStatus,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub filled_at: Option<String>,
    pub fills: Vec<OrderFill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderFill {
    pub amount: u64,
    pub price: f64,
    pub timestamp: String,
    pub tx_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub token_pair: String,
    pub bids: Vec<(f64, u64)>,
    pub asks: Vec<(f64, u64)>,
    pub spread: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitOrderStats {
    pub total_orders: usize,
    pub open_orders: usize,
    pub filled_orders: usize,
    pub cancelled_orders: usize,
    pub expired_orders: usize,
    pub total_volume: u64,
    pub avg_fill_price: f64,
}

// ── Manager ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LimitOrderManager {
    pub orders: HashMap<String, LimitOrder>,
}

impl LimitOrderManager {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Place / Cancel ──────────────────────────────────────

    pub fn place_order(&mut self, order: LimitOrder) -> Result<(), LimitOrderError> {
        if self.orders.contains_key(&order.id) {
            return Err(LimitOrderError::OrderAlreadyExists(order.id));
        }
        self.orders.insert(order.id.clone(), order);
        Ok(())
    }

    pub fn cancel_order(&mut self, id: &str) -> Result<(), LimitOrderError> {
        let order = self
            .orders
            .get_mut(id)
            .ok_or_else(|| LimitOrderError::OrderNotFound(id.to_string()))?;
        match order.status {
            OrderStatus::Open | OrderStatus::PartiallyFilled => {
                order.status = OrderStatus::Cancelled;
                Ok(())
            }
            _ => Err(LimitOrderError::OrderNotCancellable(id.to_string())),
        }
    }

    // ── Query ───────────────────────────────────────────────

    pub fn get_order(&self, id: &str) -> Option<&LimitOrder> {
        self.orders.get(id)
    }

    // ── Fill ────────────────────────────────────────────────

    pub fn fill_order(
        &mut self,
        id: &str,
        fill_amount: u64,
        fill_price: f64,
        tx_hash: Option<String>,
    ) -> Result<(), LimitOrderError> {
        let order = self
            .orders
            .get_mut(id)
            .ok_or_else(|| LimitOrderError::OrderNotFound(id.to_string()))?;

        match order.status {
            OrderStatus::Open | OrderStatus::PartiallyFilled => {}
            _ => return Err(LimitOrderError::OrderNotFillable(id.to_string())),
        }

        let remaining = order.amount - order.filled_amount;
        if fill_amount > remaining {
            return Err(LimitOrderError::FillExceedsRemaining(id.to_string()));
        }

        let now = chrono::Utc::now().to_rfc3339();
        order.fills.push(OrderFill {
            amount: fill_amount,
            price: fill_price,
            timestamp: now.clone(),
            tx_hash,
        });
        order.filled_amount += fill_amount;

        if order.filled_amount >= order.amount {
            order.status = OrderStatus::Filled;
            order.filled_at = Some(now);
        } else {
            order.status = OrderStatus::PartiallyFilled;
        }
        Ok(())
    }

    // ── Triggers ────────────────────────────────────────────

    pub fn check_triggers(&self, current_price: f64, token_pair: &str) -> Vec<String> {
        self.orders
            .values()
            .filter(|o| {
                let pair = format!("{}/{}", o.token_from, o.token_to);
                pair == token_pair
                    && matches!(o.status, OrderStatus::Open | OrderStatus::PartiallyFilled)
            })
            .filter(|o| {
                if let Some(trigger) = o.trigger_price {
                    match o.order_type {
                        OrderType::StopLoss => current_price <= trigger,
                        OrderType::TakeProfit => current_price >= trigger,
                        OrderType::TrailingStop(_) => current_price <= trigger,
                        _ => false,
                    }
                } else {
                    false
                }
            })
            .map(|o| o.id.clone())
            .collect()
    }

    // ── Expiry ──────────────────────────────────────────────

    pub fn expire_orders(&mut self) -> Vec<String> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut expired = Vec::new();
        for order in self.orders.values_mut() {
            if matches!(
                order.status,
                OrderStatus::Open | OrderStatus::PartiallyFilled
            ) {
                if let Some(ref expires_at) = order.expires_at {
                    if *expires_at <= now {
                        order.status = OrderStatus::Expired;
                        expired.push(order.id.clone());
                    }
                }
            }
        }
        expired
    }

    // ── Listings ────────────────────────────────────────────

    pub fn open_orders(&self) -> Vec<&LimitOrder> {
        self.orders
            .values()
            .filter(|o| matches!(o.status, OrderStatus::Open | OrderStatus::PartiallyFilled))
            .collect()
    }

    pub fn orders_by_pair(&self, token_from: &str, token_to: &str) -> Vec<&LimitOrder> {
        self.orders
            .values()
            .filter(|o| o.token_from == token_from && o.token_to == token_to)
            .collect()
    }

    // ── Order Book ──────────────────────────────────────────

    pub fn build_order_book(&self, token_from: &str, token_to: &str) -> OrderBook {
        let open: Vec<&LimitOrder> = self
            .orders
            .values()
            .filter(|o| {
                o.token_from == token_from
                    && o.token_to == token_to
                    && matches!(o.status, OrderStatus::Open | OrderStatus::PartiallyFilled)
            })
            .collect();

        let mut bid_map: HashMap<u64, u64> = HashMap::new();
        let mut ask_map: HashMap<u64, u64> = HashMap::new();

        for o in &open {
            let key = (o.price * 1_000_000.0) as u64;
            let remaining = o.amount - o.filled_amount;
            match o.side {
                OrderSide::Buy => *bid_map.entry(key).or_insert(0) += remaining,
                OrderSide::Sell => *ask_map.entry(key).or_insert(0) += remaining,
            }
        }

        let mut bids: Vec<(f64, u64)> = bid_map
            .into_iter()
            .map(|(k, v)| (k as f64 / 1_000_000.0, v))
            .collect();
        bids.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut asks: Vec<(f64, u64)> = ask_map
            .into_iter()
            .map(|(k, v)| (k as f64 / 1_000_000.0, v))
            .collect();
        asks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let spread = match (bids.first(), asks.first()) {
            (Some(b), Some(a)) => a.0 - b.0,
            _ => 0.0,
        };

        OrderBook {
            token_pair: format!("{}/{}", token_from, token_to),
            bids,
            asks,
            spread,
        }
    }

    // ── History ─────────────────────────────────────────────

    pub fn order_history(&self) -> Vec<&LimitOrder> {
        let mut orders: Vec<&LimitOrder> = self.orders.values().collect();
        orders.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        orders
    }

    // ── Remaining ───────────────────────────────────────────

    pub fn remaining_amount(&self, id: &str) -> Result<u64, LimitOrderError> {
        let order = self
            .orders
            .get(id)
            .ok_or_else(|| LimitOrderError::OrderNotFound(id.to_string()))?;
        Ok(order.amount - order.filled_amount)
    }

    // ── Stats ───────────────────────────────────────────────

    pub fn stats(&self) -> LimitOrderStats {
        let total_orders = self.orders.len();
        let open_orders = self
            .orders
            .values()
            .filter(|o| o.status == OrderStatus::Open)
            .count();
        let filled_orders = self
            .orders
            .values()
            .filter(|o| o.status == OrderStatus::Filled)
            .count();
        let cancelled_orders = self
            .orders
            .values()
            .filter(|o| o.status == OrderStatus::Cancelled)
            .count();
        let expired_orders = self
            .orders
            .values()
            .filter(|o| o.status == OrderStatus::Expired)
            .count();

        let total_volume: u64 = self.orders.values().map(|o| o.filled_amount).sum();

        let (total_fill_value, total_fill_amount) = self
            .orders
            .values()
            .flat_map(|o| o.fills.iter())
            .fold((0.0_f64, 0_u64), |(val, amt), f| {
                (val + f.price * f.amount as f64, amt + f.amount)
            });

        let avg_fill_price = if total_fill_amount > 0 {
            total_fill_value / total_fill_amount as f64
        } else {
            0.0
        };

        LimitOrderStats {
            total_orders,
            open_orders,
            filled_orders,
            cancelled_orders,
            expired_orders,
            total_volume,
            avg_fill_price,
        }
    }

    // ── Persistence ─────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), LimitOrderError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| LimitOrderError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| LimitOrderError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, LimitOrderError> {
        let data = std::fs::read_to_string(path).map_err(|e| LimitOrderError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| LimitOrderError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_order(id: &str, side: OrderSide, price: f64, amount: u64) -> LimitOrder {
        LimitOrder {
            id: id.to_string(),
            token_from: "EVAP".to_string(),
            token_to: "USDC".to_string(),
            side,
            order_type: OrderType::Limit,
            amount,
            filled_amount: 0,
            price,
            trigger_price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            expires_at: None,
            filled_at: None,
            fills: vec![],
        }
    }

    fn make_stop_loss(id: &str, trigger: f64) -> LimitOrder {
        let mut o = make_order(id, OrderSide::Sell, trigger, 100);
        o.order_type = OrderType::StopLoss;
        o.trigger_price = Some(trigger);
        o
    }

    fn make_take_profit(id: &str, trigger: f64) -> LimitOrder {
        let mut o = make_order(id, OrderSide::Sell, trigger, 100);
        o.order_type = OrderType::TakeProfit;
        o.trigger_price = Some(trigger);
        o
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "evap_limit_order_test_{}_{}",
            std::process::id(),
            name
        ))
    }

    // 1. Place order
    #[test]
    fn test_place_order() {
        let mut mgr = LimitOrderManager::new();
        let order = make_order("o1", OrderSide::Buy, 1.5, 1000);
        assert!(mgr.place_order(order).is_ok());
        assert!(mgr.get_order("o1").is_some());
    }

    // 2. Place duplicate
    #[test]
    fn test_place_order_duplicate() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 1000))
            .unwrap();
        let result = mgr.place_order(make_order("o1", OrderSide::Sell, 2.0, 500));
        assert!(matches!(
            result.unwrap_err(),
            LimitOrderError::OrderAlreadyExists(_)
        ));
    }

    // 3. Cancel open order
    #[test]
    fn test_cancel_order() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 1000))
            .unwrap();
        assert!(mgr.cancel_order("o1").is_ok());
        assert_eq!(mgr.get_order("o1").unwrap().status, OrderStatus::Cancelled);
    }

    // 4. Cancel non-existent
    #[test]
    fn test_cancel_order_not_found() {
        let mut mgr = LimitOrderManager::new();
        assert!(matches!(
            mgr.cancel_order("no-such").unwrap_err(),
            LimitOrderError::OrderNotFound(_)
        ));
    }

    // 5. Cancel already filled
    #[test]
    fn test_cancel_filled_order() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 100))
            .unwrap();
        mgr.fill_order("o1", 100, 1.5, None).unwrap();
        assert!(matches!(
            mgr.cancel_order("o1").unwrap_err(),
            LimitOrderError::OrderNotCancellable(_)
        ));
    }

    // 6. Full fill
    #[test]
    fn test_fill_order_full() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 1000))
            .unwrap();
        mgr.fill_order("o1", 1000, 1.5, Some("tx123".to_string()))
            .unwrap();
        let order = mgr.get_order("o1").unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.filled_amount, 1000);
        assert!(order.filled_at.is_some());
        assert_eq!(order.fills.len(), 1);
        assert_eq!(order.fills[0].tx_hash, Some("tx123".to_string()));
    }

    // 7. Partial fill
    #[test]
    fn test_fill_order_partial() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 1000))
            .unwrap();
        mgr.fill_order("o1", 400, 1.5, None).unwrap();
        let order = mgr.get_order("o1").unwrap();
        assert_eq!(order.status, OrderStatus::PartiallyFilled);
        assert_eq!(order.filled_amount, 400);
        assert!(order.filled_at.is_none());
    }

    // 8. Fill exceeds remaining
    #[test]
    fn test_fill_exceeds_remaining() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 100))
            .unwrap();
        mgr.fill_order("o1", 80, 1.5, None).unwrap();
        assert!(matches!(
            mgr.fill_order("o1", 30, 1.5, None).unwrap_err(),
            LimitOrderError::FillExceedsRemaining(_)
        ));
    }

    // 9. Fill not found
    #[test]
    fn test_fill_order_not_found() {
        let mut mgr = LimitOrderManager::new();
        assert!(matches!(
            mgr.fill_order("nope", 10, 1.0, None).unwrap_err(),
            LimitOrderError::OrderNotFound(_)
        ));
    }

    // 10. Fill cancelled order
    #[test]
    fn test_fill_cancelled_order() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 100))
            .unwrap();
        mgr.cancel_order("o1").unwrap();
        assert!(matches!(
            mgr.fill_order("o1", 50, 1.5, None).unwrap_err(),
            LimitOrderError::OrderNotFillable(_)
        ));
    }

    // 11. Check stop-loss trigger
    #[test]
    fn test_check_triggers_stop_loss() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_stop_loss("sl1", 90.0)).unwrap();
        let triggered = mgr.check_triggers(85.0, "EVAP/USDC");
        assert_eq!(triggered, vec!["sl1".to_string()]);
    }

    // 12. Check take-profit trigger
    #[test]
    fn test_check_triggers_take_profit() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_take_profit("tp1", 150.0)).unwrap();
        let triggered = mgr.check_triggers(160.0, "EVAP/USDC");
        assert_eq!(triggered, vec!["tp1".to_string()]);
    }

    // 13. Trigger not hit
    #[test]
    fn test_check_triggers_not_hit() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_stop_loss("sl1", 90.0)).unwrap();
        let triggered = mgr.check_triggers(95.0, "EVAP/USDC");
        assert!(triggered.is_empty());
    }

    // 14. Expire orders
    #[test]
    fn test_expire_orders() {
        let mut mgr = LimitOrderManager::new();
        let mut o = make_order("o1", OrderSide::Buy, 1.5, 100);
        o.expires_at = Some("2000-01-01T00:00:00+00:00".to_string());
        mgr.place_order(o).unwrap();
        let expired = mgr.expire_orders();
        assert_eq!(expired, vec!["o1".to_string()]);
        assert_eq!(mgr.get_order("o1").unwrap().status, OrderStatus::Expired);
    }

    // 15. Open orders
    #[test]
    fn test_open_orders() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 100))
            .unwrap();
        mgr.place_order(make_order("o2", OrderSide::Sell, 2.0, 200))
            .unwrap();
        mgr.place_order(make_order("o3", OrderSide::Buy, 1.0, 50))
            .unwrap();
        mgr.cancel_order("o3").unwrap();
        assert_eq!(mgr.open_orders().len(), 2);
    }

    // 16. Orders by pair
    #[test]
    fn test_orders_by_pair() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 100))
            .unwrap();
        let mut other = make_order("o2", OrderSide::Sell, 2.0, 200);
        other.token_from = "BTC".to_string();
        other.token_to = "USDT".to_string();
        mgr.place_order(other).unwrap();
        assert_eq!(mgr.orders_by_pair("EVAP", "USDC").len(), 1);
        assert_eq!(mgr.orders_by_pair("BTC", "USDT").len(), 1);
    }

    // 17. Build order book
    #[test]
    fn test_build_order_book() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("b1", OrderSide::Buy, 1.4, 100))
            .unwrap();
        mgr.place_order(make_order("b2", OrderSide::Buy, 1.5, 200))
            .unwrap();
        mgr.place_order(make_order("a1", OrderSide::Sell, 1.6, 150))
            .unwrap();
        mgr.place_order(make_order("a2", OrderSide::Sell, 1.7, 300))
            .unwrap();

        let book = mgr.build_order_book("EVAP", "USDC");
        assert_eq!(book.token_pair, "EVAP/USDC");
        assert_eq!(book.bids.len(), 2);
        assert_eq!(book.asks.len(), 2);
        // Bids sorted descending
        assert!(book.bids[0].0 >= book.bids[1].0);
        // Asks sorted ascending
        assert!(book.asks[0].0 <= book.asks[1].0);
        // Spread = best ask - best bid
        let expected_spread = 1.6 - 1.5;
        assert!((book.spread - expected_spread).abs() < 0.001);
    }

    // 18. Order history sorted desc
    #[test]
    fn test_order_history() {
        let mut mgr = LimitOrderManager::new();
        let mut o1 = make_order("o1", OrderSide::Buy, 1.5, 100);
        o1.created_at = "2026-01-01T00:00:00+00:00".to_string();
        let mut o2 = make_order("o2", OrderSide::Sell, 2.0, 200);
        o2.created_at = "2026-02-01T00:00:00+00:00".to_string();
        mgr.place_order(o1).unwrap();
        mgr.place_order(o2).unwrap();

        let history = mgr.order_history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, "o2");
        assert_eq!(history[1].id, "o1");
    }

    // 19. Remaining amount
    #[test]
    fn test_remaining_amount() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 1000))
            .unwrap();
        mgr.fill_order("o1", 300, 1.5, None).unwrap();
        assert_eq!(mgr.remaining_amount("o1").unwrap(), 700);
    }

    // 20. Stats
    #[test]
    fn test_stats() {
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 100))
            .unwrap();
        mgr.place_order(make_order("o2", OrderSide::Sell, 2.0, 200))
            .unwrap();
        mgr.place_order(make_order("o3", OrderSide::Buy, 1.0, 50))
            .unwrap();
        mgr.fill_order("o1", 100, 1.5, None).unwrap();
        mgr.cancel_order("o3").unwrap();

        let stats = mgr.stats();
        assert_eq!(stats.total_orders, 3);
        assert_eq!(stats.open_orders, 1);
        assert_eq!(stats.filled_orders, 1);
        assert_eq!(stats.cancelled_orders, 1);
        assert_eq!(stats.total_volume, 100);
        assert!((stats.avg_fill_price - 1.5).abs() < 0.001);
    }

    // 21. Save and load
    #[test]
    fn test_save_and_load() {
        let path = temp_path("save_load.json");
        let mut mgr = LimitOrderManager::new();
        mgr.place_order(make_order("o1", OrderSide::Buy, 1.5, 1000))
            .unwrap();
        mgr.save(&path).unwrap();

        let loaded = LimitOrderManager::load(&path).unwrap();
        assert!(loaded.get_order("o1").is_some());
        assert_eq!(loaded.get_order("o1").unwrap().price, 1.5);

        let _ = std::fs::remove_file(&path);
    }

    // 22. Load or default
    #[test]
    fn test_load_or_default() {
        let path = temp_path("nonexistent.json");
        let _ = std::fs::remove_file(&path);
        let mgr = LimitOrderManager::load_or_default(&path);
        assert!(mgr.orders.is_empty());
    }
}
