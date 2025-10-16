use super::CandleBuffer;
use crate::api::DexScreenerClient;
use crate::models::{PriceSnapshot, Token};
use crate::Result;
use chrono::Utc;

/// Manages price data collection for multiple tokens
/// Simple and stupid - just fetches current prices
pub struct PriceFeedManager {
    dex_client: DexScreenerClient,
    buffer: CandleBuffer,
    tokens: Vec<Token>,
}

impl PriceFeedManager {
    /// Create a new price feed manager
    pub fn new(tokens: Vec<Token>, buffer_size: usize) -> Self {
        Self {
            dex_client: DexScreenerClient::new(),
            buffer: CandleBuffer::new(buffer_size),
            tokens,
        }
    }

    /// Fetch current price and create a snapshot
    pub async fn fetch_and_update(&mut self, token: &Token) -> Result<PriceSnapshot> {
        // Fetch price from DexScreener
        let price_data = self.dex_client.get_price(&token.mint_address).await?;

        // Create simple snapshot
        let snapshot = PriceSnapshot {
            token: token.symbol.clone(),
            price: price_data.price,
            timestamp: Utc::now(),
        };

        // Store as legacy candle for now (will migrate buffer next)
        let candle = crate::models::Candle {
            token: token.symbol.clone(),
            timestamp: snapshot.timestamp,
            open: snapshot.price,
            high: snapshot.price,
            low: snapshot.price,
            close: snapshot.price,
            volume: price_data.volume_24h,
        };
        self.buffer.add_candle(candle)?;

        tracing::info!(
            token = %token.symbol,
            price = %price_data.price,
            "Fetched price snapshot"
        );

        Ok(snapshot)
    }

    /// Fetch prices for all tokens
    pub async fn fetch_all(&mut self) -> Vec<Result<PriceSnapshot>> {
        let mut results = Vec::new();

        for token in &self.tokens.clone() {
            let result = self.fetch_and_update(token).await;
            results.push(result);
        }

        results
    }

    /// Get the candle buffer (temporary - will be PriceBuffer soon)
    pub fn buffer(&self) -> &CandleBuffer {
        &self.buffer
    }

    /// Get tracked tokens
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_token(symbol: &str) -> Token {
        Token {
            symbol: symbol.to_string(),
            mint_address: format!("So1111111111111111111111111111111111111111{}", symbol),
            name: format!("{} Token", symbol),
            decimals: 9,
        }
    }

    #[test]
    fn test_new_manager() {
        let tokens = vec![create_test_token("SOL")];
        let manager = PriceFeedManager::new(tokens.clone(), 100);

        assert_eq!(manager.tokens().len(), 1);
        assert_eq!(manager.tokens()[0].symbol, "SOL");
    }

    #[tokio::test]
    #[ignore] // Requires live API
    async fn test_fetch_and_update_live() {
        let sol = Token {
            symbol: "SOL".to_string(),
            mint_address: "So11111111111111111111111111111111111111112".to_string(),
            name: "Solana".to_string(),
            decimals: 9,
        };

        let mut manager = PriceFeedManager::new(vec![sol.clone()], 100);
        let result = manager.fetch_and_update(&sol).await;

        assert!(result.is_ok());
        let snapshot = result.unwrap();
        assert_eq!(snapshot.token, "SOL");
        assert!(snapshot.price > 0.0);

        // Verify it was added to buffer
        let candles = manager.buffer().get_candles("SOL").unwrap();
        assert_eq!(candles.len(), 1);
    }

    #[tokio::test]
    #[ignore] // Requires live API
    async fn test_fetch_all_live() {
        let tokens = vec![Token {
            symbol: "SOL".to_string(),
            mint_address: "So11111111111111111111111111111111111111112".to_string(),
            name: "Solana".to_string(),
            decimals: 9,
        }];

        let mut manager = PriceFeedManager::new(tokens, 100);
        let results = manager.fetch_all().await;

        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok());
    }

    #[test]
    fn test_buffer_access() {
        let tokens = vec![create_test_token("SOL")];
        let manager = PriceFeedManager::new(tokens, 100);

        let buffer = manager.buffer();
        assert_eq!(buffer.candle_count("SOL").unwrap(), 0);
    }
}
