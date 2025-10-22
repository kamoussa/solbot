use crate::models::{Candle, DataSource, PriceData};
use crate::Result;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use tokio::time::{sleep, Duration};

const DEXSCREENER_API_BASE: &str = "https://api.dexscreener.com/latest/dex";
const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 2000; // Start with 2 seconds

/// Client for DexScreener API
#[derive(Clone)]
pub struct DexScreenerClient {
    client: Client,
}

#[derive(Debug, Deserialize)]
struct DexScreenerResponse {
    pairs: Vec<PairData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairData {
    chain_id: String,
    base_token: TokenInfo,
    price_usd: String,
    volume: VolumeData,
    #[allow(dead_code)]
    price_change: PriceChange,
}

#[derive(Debug, Deserialize)]
struct TokenInfo {
    symbol: String,
    #[allow(dead_code)]
    address: String,
}

#[derive(Debug, Deserialize)]
struct VolumeData {
    h24: f64,
}

#[derive(Debug, Deserialize, Default)]
struct PriceChange {
    #[serde(default)]
    #[allow(dead_code)]
    h24: Option<f64>,
}

impl DexScreenerClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Get current price for a token by its mint address
    /// Includes retry logic with exponential backoff for transient failures
    pub async fn get_price(&self, token_address: &str) -> Result<PriceData> {
        let mut last_error = None;

        for attempt in 1..=MAX_RETRIES {
            match self.fetch_price_once(token_address).await {
                Ok(price_data) => {
                    if attempt > 1 {
                        tracing::info!(
                            "✓ Successfully fetched {} after {} attempts",
                            token_address,
                            attempt
                        );
                    }
                    return Ok(price_data);
                }
                Err(e) => {
                    last_error = Some(e);

                    if attempt < MAX_RETRIES {
                        let backoff_ms = INITIAL_BACKOFF_MS * 2_u64.pow(attempt - 1);
                        tracing::warn!(
                            "Attempt {}/{} failed for {}: {}. Retrying in {}ms...",
                            attempt,
                            MAX_RETRIES,
                            token_address,
                            last_error.as_ref().unwrap(),
                            backoff_ms
                        );
                        sleep(Duration::from_millis(backoff_ms)).await;
                    }
                }
            }
        }

        // All retries exhausted
        Err(last_error.unwrap_or_else(|| "All retry attempts failed".into()))
    }

    /// Internal method to fetch price once (without retry logic)
    async fn fetch_price_once(&self, token_address: &str) -> Result<PriceData> {
        let url = format!("{}/tokens/{}", DEXSCREENER_API_BASE, token_address);

        let response_raw = self.client.get(&url).send().await?;
        let response: DexScreenerResponse = response_raw.json().await?;

        // Get the Solana pair (if multiple pairs exist, prefer Solana)
        let pair = response
            .pairs
            .into_iter()
            .find(|p| p.chain_id == "solana")
            .ok_or("No Solana pair found for token")?;

        Ok(PriceData {
            token: pair.base_token.symbol,
            price: pair.price_usd.parse()?,
            volume_24h: pair.volume.h24,
            timestamp: Utc::now(),
            source: DataSource::DexScreener,
        })
    }

    /// Get historical candle data
    /// Note: DexScreener free API has limited historical data
    pub async fn get_candles(
        &self,
        _token_address: &str,
        _from: DateTime<Utc>,
        _to: DateTime<Utc>,
    ) -> Result<Vec<Candle>> {
        // TODO: Implement when we have access to historical data endpoint
        // For now, return empty vector
        Ok(vec![])
    }
}

impl Default for DexScreenerClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Ignore by default to avoid hitting API in tests
    async fn test_get_price_live() {
        let client = DexScreenerClient::new();

        // SOL mint address
        let sol_mint = "So11111111111111111111111111111111111111112";

        let price_data = client.get_price(sol_mint).await;
        assert!(price_data.is_ok());

        let price = price_data.unwrap();
        assert_eq!(price.token, "SOL");
        assert!(price.price > 0.0);
    }

    #[test]
    fn test_client_creation() {
        let client = DexScreenerClient::new();
        // Just verify it compiles and creates
        drop(client);
    }
}
