use crate::models::{Candle, PriceData, DataSource};
use crate::Result;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

const DEXSCREENER_API_BASE: &str = "https://api.dexscreener.com/latest/dex";

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
    pub async fn get_price(&self, token_address: &str) -> Result<PriceData> {
        let url = format!("{}/tokens/{}", DEXSCREENER_API_BASE, token_address);

        let response_raw = self.client
            .get(&url)
            .send()
            .await?;
        //tracing::info!("Response: {:?}", response_raw.status());
        //tracing::info!("Response: {:?}", response_raw.headers());
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
    #[ignore]  // Ignore by default to avoid hitting API in tests
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
