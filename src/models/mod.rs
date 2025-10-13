use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Represents a cryptocurrency token
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Token {
    pub symbol: String,
    pub mint_address: String,  // Solana mint address
    pub name: String,
    pub decimals: u8,
}

/// Price data at a specific point in time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceData {
    pub token: String,
    pub price: f64,
    pub volume_24h: f64,
    pub timestamp: DateTime<Utc>,
    pub source: DataSource,
}

/// Simple price snapshot - just price and timestamp
/// This is our core data structure - no fake OHLCV
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceSnapshot {
    pub token: String,
    pub price: f64,
    pub timestamp: DateTime<Utc>,
}

/// OHLCV candlestick data (legacy - will be removed)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub token: String,
    pub timestamp: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Data source identifier
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DataSource {
    DexScreener,
    Jupiter,
    Fallback,
}

/// Trading signal
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Signal {
    Buy,
    Sell,
    Hold,
}

/// Position in a token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: Uuid,
    pub token: String,
    pub entry_price: f64,
    pub quantity: f64,
    pub entry_time: DateTime<Utc>,
    pub stop_loss: f64,
    pub take_profit: Option<f64>,  // None until trailing activated
    pub status: PositionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PositionStatus {
    Open,
    Closed,
}

/// Trade execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: Uuid,
    pub token: String,
    pub side: TradeSide,
    pub price: f64,
    pub quantity: f64,
    pub timestamp: DateTime<Utc>,
    pub tx_signature: Option<String>,  // Solana transaction signature
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradeSide {
    Buy,
    Sell,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_creation() {
        let token = Token {
            symbol: "SOL".to_string(),
            mint_address: "So11111111111111111111111111111111111111112".to_string(),
            name: "Solana".to_string(),
            decimals: 9,
        };

        assert_eq!(token.symbol, "SOL");
        assert_eq!(token.decimals, 9);
    }

    #[test]
    fn test_position_creation() {
        let position = Position {
            id: Uuid::new_v4(),
            token: "SOL".to_string(),
            entry_price: 100.0,
            quantity: 10.0,
            entry_time: Utc::now(),
            stop_loss: 92.0,  // -8% stop loss
            take_profit: None,
            status: PositionStatus::Open,
        };

        assert_eq!(position.status, PositionStatus::Open);
        assert_eq!(position.stop_loss, 92.0);
    }
}
