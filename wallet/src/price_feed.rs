//! Token price tracking and portfolio valuation system.
//!
//! Tracks live token prices, OHLC candles, price alerts, and portfolio
//! valuations. Data is persisted to a local JSON store.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum PriceFeedError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid price: {0}")]
    InvalidPrice(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ──────────────────────────────────────

/// Condition that triggers a price alert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PriceAlertCondition {
    Above(f64),
    Below(f64),
    ChangePercent(f64),
}

/// Status of a price alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertStatus {
    Active,
    Triggered,
    Expired,
}

/// Currency denomination for prices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Currency {
    Usd,
    Eur,
    Gbp,
    Btc,
    Eth,
    Custom(String),
}

// ──────────────────────────── PricePoint ─────────────────────────────────

/// A single price observation.
#[derive(Clone, Serialize, Deserialize)]
pub struct PricePoint {
    pub price: f64,
    pub volume: f64,
    pub timestamp: String,
}

impl PricePoint {
    pub fn new(price: f64, volume: f64) -> Self {
        Self {
            price,
            volume,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }
}

// ──────────────────────────── OhlcCandle ─────────────────────────────────

/// OHLC candlestick data.
#[derive(Clone, Serialize, Deserialize)]
pub struct OhlcCandle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub timestamp: String,
}

impl OhlcCandle {
    pub fn new(open: f64, high: f64, low: f64, close: f64, volume: f64) -> Self {
        Self {
            open,
            high,
            low,
            close,
            volume,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Price range within the candle.
    pub fn range(&self) -> f64 {
        self.high - self.low
    }

    /// Whether the candle closed at or above its open.
    pub fn is_bullish(&self) -> bool {
        self.close >= self.open
    }

    /// Absolute size of the candle body.
    pub fn body_size(&self) -> f64 {
        (self.close - self.open).abs()
    }
}

// ──────────────────────────── TokenPrice ─────────────────────────────────

/// Full price state for a single token.
#[derive(Clone, Serialize, Deserialize)]
pub struct TokenPrice {
    pub token_id: String,
    pub symbol: String,
    pub current_price: f64,
    pub currency: Currency,
    pub change_24h: f64,
    pub high_24h: f64,
    pub low_24h: f64,
    pub market_cap: Option<u64>,
    pub history: Vec<PricePoint>,
    pub candles: Vec<OhlcCandle>,
    pub last_updated: String,
}

impl TokenPrice {
    pub fn new(token_id: &str, symbol: &str, price: f64, currency: Currency) -> Self {
        Self {
            token_id: token_id.to_string(),
            symbol: symbol.to_string(),
            current_price: price,
            currency,
            change_24h: 0.0,
            high_24h: price,
            low_24h: price,
            market_cap: None,
            history: Vec::new(),
            candles: Vec::new(),
            last_updated: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Record a new price observation.
    pub fn update_price(&mut self, price: f64, volume: f64) {
        self.history.push(PricePoint::new(price, volume));
        self.current_price = price;

        // Compute 24h change from oldest price in history.
        if let Some(first) = self.history.first() {
            if first.price != 0.0 {
                self.change_24h = (price - first.price) / first.price * 100.0;
            } else {
                self.change_24h = 0.0;
            }
        } else {
            self.change_24h = 0.0;
        }

        // Update 24h high/low.
        if price > self.high_24h {
            self.high_24h = price;
        }
        if price < self.low_24h {
            self.low_24h = price;
        }

        // Prune history to max 500 entries.
        if self.history.len() > 500 {
            let excess = self.history.len() - 500;
            self.history.drain(..excess);
        }

        self.last_updated = chrono::Utc::now().to_rfc3339();
    }

    /// Add an OHLC candle, pruning to max 200.
    pub fn add_candle(&mut self, candle: OhlcCandle) {
        self.candles.push(candle);
        if self.candles.len() > 200 {
            let excess = self.candles.len() - 200;
            self.candles.drain(..excess);
        }
    }

    /// Average of all historical prices.
    pub fn average_price(&self) -> f64 {
        if self.history.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.history.iter().map(|p| p.price).sum();
        sum / self.history.len() as f64
    }

    /// Price at a given history index.
    pub fn price_at(&self, index: usize) -> Option<f64> {
        self.history.get(index).map(|p| p.price)
    }

    /// Standard deviation of historical prices.
    pub fn volatility(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.0;
        }
        let mean = self.average_price();
        let variance: f64 = self
            .history
            .iter()
            .map(|p| {
                let diff = p.price - mean;
                diff * diff
            })
            .sum::<f64>()
            / self.history.len() as f64;
        variance.sqrt()
    }

    /// Percent change from first history point to current price.
    pub fn trend(&self) -> f64 {
        match self.history.first() {
            Some(first) if first.price != 0.0 => {
                (self.current_price - first.price) / first.price * 100.0
            }
            _ => 0.0,
        }
    }
}

// ──────────────────────────── PriceAlert ─────────────────────────────────

/// An alert that fires when a price condition is met.
#[derive(Clone, Serialize, Deserialize)]
pub struct PriceAlert {
    pub id: String,
    pub token_id: String,
    pub condition: PriceAlertCondition,
    pub status: AlertStatus,
    pub created_at: String,
    pub triggered_at: Option<String>,
    pub message: String,
}

impl PriceAlert {
    pub fn new(id: &str, token_id: &str, condition: PriceAlertCondition) -> Self {
        Self {
            id: id.to_string(),
            token_id: token_id.to_string(),
            condition,
            status: AlertStatus::Active,
            created_at: chrono::Utc::now().to_rfc3339(),
            triggered_at: None,
            message: String::new(),
        }
    }

    /// Check the alert against current market data. Returns true if just triggered.
    pub fn check(&mut self, current_price: f64, change_24h: f64) -> bool {
        if self.status != AlertStatus::Active {
            return false;
        }

        let triggered = match self.condition {
            PriceAlertCondition::Above(threshold) => current_price > threshold,
            PriceAlertCondition::Below(threshold) => current_price < threshold,
            PriceAlertCondition::ChangePercent(threshold) => change_24h.abs() >= threshold.abs(),
        };

        if triggered {
            self.status = AlertStatus::Triggered;
            self.triggered_at = Some(chrono::Utc::now().to_rfc3339());
            self.message = match self.condition {
                PriceAlertCondition::Above(t) => {
                    format!("Price {} exceeded threshold {}", current_price, t)
                }
                PriceAlertCondition::Below(t) => {
                    format!("Price {} fell below threshold {}", current_price, t)
                }
                PriceAlertCondition::ChangePercent(t) => {
                    format!("24h change {:.2}% exceeded threshold {:.2}%", change_24h, t)
                }
            };
        }

        triggered
    }

    /// Whether the alert is still active.
    pub fn is_active(&self) -> bool {
        self.status == AlertStatus::Active
    }
}

// ──────────────────────────── PortfolioValuation ─────────────────────────

/// Snapshot of portfolio value at a point in time.
#[derive(Serialize, Deserialize)]
pub struct PortfolioValuation {
    pub total_value: f64,
    pub currency: Currency,
    /// (symbol, amount, value) for each holding.
    pub holdings: Vec<(String, f64, f64)>,
    pub computed_at: String,
}

// ──────────────────────────── FeedStats ──────────────────────────────────

/// Summary statistics for the feed.
#[derive(Serialize, Deserialize)]
pub struct FeedStats {
    pub total_tokens: usize,
    pub total_alerts: usize,
    pub active_alerts: usize,
    pub triggered_alerts: usize,
}

// ──────────────────────────── PriceFeed ──────────────────────────────────

/// Central price feed aggregator.
#[derive(Default, Serialize, Deserialize)]
pub struct PriceFeed {
    pub prices: HashMap<String, TokenPrice>,
    pub alerts: Vec<PriceAlert>,
    #[serde(default = "default_max_alerts")]
    pub max_alerts: usize,
}

fn default_max_alerts() -> usize {
    100
}

impl PriceFeed {
    pub fn new() -> Self {
        Self {
            prices: HashMap::new(),
            alerts: Vec::new(),
            max_alerts: 100,
        }
    }

    /// Update the price of an already-registered token.
    pub fn update_price(
        &mut self,
        token_id: &str,
        price: f64,
        volume: f64,
    ) -> Result<(), PriceFeedError> {
        let token = self
            .prices
            .get_mut(token_id)
            .ok_or_else(|| PriceFeedError::NotFound(token_id.to_string()))?;
        token.update_price(price, volume);
        Ok(())
    }

    /// Register (or replace) a token in the feed.
    pub fn register_token(&mut self, token: TokenPrice) {
        self.prices.insert(token.token_id.clone(), token);
    }

    pub fn get_price(&self, token_id: &str) -> Option<&TokenPrice> {
        self.prices.get(token_id)
    }

    pub fn get_price_mut(&mut self, token_id: &str) -> Option<&mut TokenPrice> {
        self.prices.get_mut(token_id)
    }

    pub fn list_tokens(&self) -> Vec<&TokenPrice> {
        self.prices.values().collect()
    }

    /// Add a price alert, pruning oldest if over max.
    pub fn add_alert(&mut self, alert: PriceAlert) {
        self.alerts.push(alert);
        if self.alerts.len() > self.max_alerts {
            self.alerts.remove(0);
        }
    }

    /// Check all active alerts and return IDs of newly triggered ones.
    pub fn check_alerts(&mut self) -> Vec<String> {
        // Collect price data first to avoid double borrow.
        let price_data: HashMap<String, (f64, f64)> = self
            .prices
            .iter()
            .map(|(id, tp)| (id.clone(), (tp.current_price, tp.change_24h)))
            .collect();

        let mut triggered = Vec::new();
        for alert in &mut self.alerts {
            if alert.status != AlertStatus::Active {
                continue;
            }
            if let Some(&(price, change)) = price_data.get(&alert.token_id) {
                if alert.check(price, change) {
                    triggered.push(alert.id.clone());
                }
            }
        }
        triggered
    }

    pub fn active_alerts(&self) -> Vec<&PriceAlert> {
        self.alerts
            .iter()
            .filter(|a| a.status == AlertStatus::Active)
            .collect()
    }

    pub fn triggered_alerts(&self) -> Vec<&PriceAlert> {
        self.alerts
            .iter()
            .filter(|a| a.status == AlertStatus::Triggered)
            .collect()
    }

    /// Remove an alert by ID. Returns true if found.
    pub fn remove_alert(&mut self, id: &str) -> bool {
        let before = self.alerts.len();
        self.alerts.retain(|a| a.id != id);
        self.alerts.len() < before
    }

    /// Compute portfolio valuation from (token_id, amount) pairs.
    pub fn valuate_portfolio(&self, holdings: &[(String, f64)]) -> PortfolioValuation {
        let mut total_value = 0.0;
        let mut items = Vec::new();
        let mut currency = Currency::Usd;

        for (token_id, amount) in holdings {
            if let Some(tp) = self.prices.get(token_id) {
                let value = tp.current_price * amount;
                total_value += value;
                items.push((tp.symbol.clone(), *amount, value));
                currency = tp.currency.clone();
            } else {
                items.push((token_id.clone(), *amount, 0.0));
            }
        }

        PortfolioValuation {
            total_value,
            currency,
            holdings: items,
            computed_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Top N gainers by 24h change.
    pub fn top_gainers(&self, n: usize) -> Vec<&TokenPrice> {
        let mut tokens: Vec<&TokenPrice> = self.prices.values().collect();
        tokens.sort_by(|a, b| b.change_24h.partial_cmp(&a.change_24h).unwrap());
        tokens.truncate(n);
        tokens
    }

    /// Top N losers by 24h change.
    pub fn top_losers(&self, n: usize) -> Vec<&TokenPrice> {
        let mut tokens: Vec<&TokenPrice> = self.prices.values().collect();
        tokens.sort_by(|a, b| a.change_24h.partial_cmp(&b.change_24h).unwrap());
        tokens.truncate(n);
        tokens
    }

    /// Summary statistics.
    pub fn stats(&self) -> FeedStats {
        FeedStats {
            total_tokens: self.prices.len(),
            total_alerts: self.alerts.len(),
            active_alerts: self.alerts.iter().filter(|a| a.status == AlertStatus::Active).count(),
            triggered_alerts: self
                .alerts
                .iter()
                .filter(|a| a.status == AlertStatus::Triggered)
                .count(),
        }
    }

    // ──────────────── Persistence ────────────────

    pub fn save(&self, path: &Path) -> Result<(), PriceFeedError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, PriceFeedError> {
        let data = std::fs::read_to_string(path)?;
        let feed: PriceFeed = serde_json::from_str(&data)?;
        Ok(feed)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("price_feed_test_{}", std::process::id()))
    }

    fn make_feed_with_tokens() -> PriceFeed {
        let mut feed = PriceFeed::new();
        let mut t1 = TokenPrice::new("evap", "EVAP", 10.0, Currency::Usd);
        t1.update_price(12.0, 1000.0);
        let mut t2 = TokenPrice::new("btc", "BTC", 50000.0, Currency::Usd);
        t2.update_price(48000.0, 5000.0);
        let mut t3 = TokenPrice::new("eth", "ETH", 3000.0, Currency::Usd);
        t3.update_price(3200.0, 2000.0);
        feed.register_token(t1);
        feed.register_token(t2);
        feed.register_token(t3);
        feed
    }

    #[test]
    fn test_register_and_get_token() {
        let mut feed = PriceFeed::new();
        let token = TokenPrice::new("evap", "EVAP", 10.0, Currency::Usd);
        feed.register_token(token);
        let tp = feed.get_price("evap").unwrap();
        assert_eq!(tp.symbol, "EVAP");
        assert_eq!(tp.current_price, 10.0);
    }

    #[test]
    fn test_update_price() {
        let mut feed = PriceFeed::new();
        feed.register_token(TokenPrice::new("evap", "EVAP", 10.0, Currency::Usd));
        feed.update_price("evap", 12.0, 500.0).unwrap();
        let tp = feed.get_price("evap").unwrap();
        assert_eq!(tp.current_price, 12.0);
        assert_eq!(tp.history.len(), 1);
    }

    #[test]
    fn test_update_price_not_found() {
        let mut feed = PriceFeed::new();
        let err = feed.update_price("nope", 1.0, 1.0).unwrap_err();
        assert!(matches!(err, PriceFeedError::NotFound(_)));
    }

    #[test]
    fn test_price_history_capped() {
        let mut token = TokenPrice::new("evap", "EVAP", 1.0, Currency::Usd);
        for i in 0..600 {
            token.update_price(i as f64, 1.0);
        }
        assert!(token.history.len() <= 500);
    }

    #[test]
    fn test_average_price() {
        let mut token = TokenPrice::new("evap", "EVAP", 10.0, Currency::Usd);
        assert_eq!(token.average_price(), 0.0);
        token.update_price(10.0, 1.0);
        token.update_price(20.0, 1.0);
        assert!((token.average_price() - 15.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_volatility() {
        let mut token = TokenPrice::new("evap", "EVAP", 10.0, Currency::Usd);
        assert_eq!(token.volatility(), 0.0);
        token.update_price(10.0, 1.0);
        token.update_price(20.0, 1.0);
        assert!(token.volatility() > 0.0);
    }

    #[test]
    fn test_trend() {
        let mut token = TokenPrice::new("evap", "EVAP", 10.0, Currency::Usd);
        assert_eq!(token.trend(), 0.0);
        token.update_price(10.0, 1.0);
        token.update_price(15.0, 1.0);
        // first history price is 10, current is 15 → 50%
        assert!((token.trend() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ohlc_candle() {
        let candle = OhlcCandle::new(100.0, 120.0, 90.0, 110.0, 5000.0);
        assert_eq!(candle.range(), 30.0);
        assert_eq!(candle.body_size(), 10.0);
    }

    #[test]
    fn test_ohlc_bullish_bearish() {
        let bull = OhlcCandle::new(100.0, 120.0, 95.0, 115.0, 1000.0);
        assert!(bull.is_bullish());
        let bear = OhlcCandle::new(100.0, 105.0, 85.0, 90.0, 1000.0);
        assert!(!bear.is_bullish());
        // equal open/close counts as bullish
        let flat = OhlcCandle::new(100.0, 105.0, 95.0, 100.0, 1000.0);
        assert!(flat.is_bullish());
    }

    #[test]
    fn test_alert_above_triggered() {
        let mut alert = PriceAlert::new("a1", "evap", PriceAlertCondition::Above(15.0));
        assert!(!alert.check(14.0, 0.0));
        assert!(alert.is_active());
        assert!(alert.check(16.0, 0.0));
        assert_eq!(alert.status, AlertStatus::Triggered);
        assert!(alert.triggered_at.is_some());
        assert!(!alert.message.is_empty());
    }

    #[test]
    fn test_alert_below_triggered() {
        let mut alert = PriceAlert::new("a2", "evap", PriceAlertCondition::Below(5.0));
        assert!(!alert.check(6.0, 0.0));
        assert!(alert.check(4.0, 0.0));
        assert_eq!(alert.status, AlertStatus::Triggered);
    }

    #[test]
    fn test_alert_change_percent() {
        let mut alert = PriceAlert::new("a3", "evap", PriceAlertCondition::ChangePercent(10.0));
        assert!(!alert.check(100.0, 5.0));
        assert!(alert.check(100.0, 12.0));
        assert_eq!(alert.status, AlertStatus::Triggered);
    }

    #[test]
    fn test_alert_not_triggered() {
        let mut alert = PriceAlert::new("a4", "evap", PriceAlertCondition::Above(100.0));
        assert!(!alert.check(50.0, 0.0));
        assert!(alert.is_active());
        assert!(alert.triggered_at.is_none());
    }

    #[test]
    fn test_check_alerts() {
        let mut feed = PriceFeed::new();
        let mut token = TokenPrice::new("evap", "EVAP", 10.0, Currency::Usd);
        token.update_price(20.0, 100.0);
        feed.register_token(token);

        feed.add_alert(PriceAlert::new("a1", "evap", PriceAlertCondition::Above(15.0)));
        feed.add_alert(PriceAlert::new("a2", "evap", PriceAlertCondition::Above(25.0)));

        let triggered = feed.check_alerts();
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0], "a1");
    }

    #[test]
    fn test_active_and_triggered_alerts() {
        let mut feed = PriceFeed::new();
        let mut token = TokenPrice::new("evap", "EVAP", 10.0, Currency::Usd);
        token.update_price(20.0, 100.0);
        feed.register_token(token);

        feed.add_alert(PriceAlert::new("a1", "evap", PriceAlertCondition::Above(15.0)));
        feed.add_alert(PriceAlert::new("a2", "evap", PriceAlertCondition::Above(25.0)));
        feed.check_alerts();

        assert_eq!(feed.active_alerts().len(), 1);
        assert_eq!(feed.triggered_alerts().len(), 1);
    }

    #[test]
    fn test_remove_alert() {
        let mut feed = PriceFeed::new();
        feed.add_alert(PriceAlert::new("a1", "evap", PriceAlertCondition::Above(10.0)));
        assert!(feed.remove_alert("a1"));
        assert!(!feed.remove_alert("a1"));
        assert!(feed.alerts.is_empty());
    }

    #[test]
    fn test_valuate_portfolio() {
        let feed = make_feed_with_tokens();
        let holdings = vec![
            ("evap".to_string(), 100.0),
            ("btc".to_string(), 0.5),
            ("missing".to_string(), 10.0),
        ];
        let val = feed.valuate_portfolio(&holdings);
        // evap is 12.0 * 100 = 1200, btc is 48000 * 0.5 = 24000
        assert!((val.total_value - 25200.0).abs() < 0.01);
        assert_eq!(val.holdings.len(), 3);
        // missing token should have 0 value
        assert_eq!(val.holdings[2].2, 0.0);
    }

    #[test]
    fn test_top_gainers() {
        let feed = make_feed_with_tokens();
        let gainers = feed.top_gainers(2);
        assert_eq!(gainers.len(), 2);
        // First gainer should have the highest change_24h
        assert!(gainers[0].change_24h >= gainers[1].change_24h);
    }

    #[test]
    fn test_top_losers() {
        let feed = make_feed_with_tokens();
        let losers = feed.top_losers(2);
        assert_eq!(losers.len(), 2);
        // First loser should have the lowest change_24h
        assert!(losers[0].change_24h <= losers[1].change_24h);
    }

    #[test]
    fn test_stats() {
        let mut feed = make_feed_with_tokens();
        feed.add_alert(PriceAlert::new("a1", "evap", PriceAlertCondition::Above(15.0)));
        feed.add_alert(PriceAlert::new("a2", "evap", PriceAlertCondition::Above(5.0)));
        feed.check_alerts();

        let stats = feed.stats();
        assert_eq!(stats.total_tokens, 3);
        assert_eq!(stats.total_alerts, 2);
        assert_eq!(stats.active_alerts, 1);
        assert_eq!(stats.triggered_alerts, 1);
    }

    #[test]
    fn test_currency_variants() {
        assert_eq!(Currency::Usd, Currency::Usd);
        assert_ne!(Currency::Usd, Currency::Eur);
        assert_eq!(
            Currency::Custom("SOL".to_string()),
            Currency::Custom("SOL".to_string())
        );
        assert_ne!(
            Currency::Custom("SOL".to_string()),
            Currency::Custom("DOT".to_string())
        );
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path();
        let mut feed = make_feed_with_tokens();
        feed.add_alert(PriceAlert::new("a1", "evap", PriceAlertCondition::Above(15.0)));
        feed.save(&path).unwrap();

        let loaded = PriceFeed::load(&path).unwrap();
        assert_eq!(loaded.prices.len(), 3);
        assert_eq!(loaded.alerts.len(), 1);
        assert_eq!(loaded.get_price("evap").unwrap().current_price, 12.0);

        // Clean up.
        let _ = std::fs::remove_file(&path);

        // Test load_or_default on missing file.
        let missing = std::env::temp_dir().join("nonexistent_price_feed_file.json");
        let default_feed = PriceFeed::load_or_default(&missing);
        assert!(default_feed.prices.is_empty());
    }
}
