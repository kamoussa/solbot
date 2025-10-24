use anyhow::{Context, Result};
use governor::{Quota, RateLimiter};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::sync::RwLock;

const COINGECKO_API_BASE: &str = "https://api.coingecko.com/api/v3";
const RATE_LIMIT_RPM: u32 = 30; // Demo API: 30 requests per minute
const MAX_RETRIES: u32 = 3;

// Type alias for the rate limiter to simplify signatures
type CoinGeckoRateLimiter = RateLimiter<
    governor::state::direct::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
>;

/// Cache for CoinGecko coin ID mappings
#[derive(Debug, Clone, Default)]
struct CoinCache {
    /// Maps Solana mint address -> CoinGecko coin_id
    by_address: HashMap<String, String>,
    /// Maps CoinGecko coin_id -> Solana mint address (inverted for lookups)
    by_coin_id: HashMap<String, String>,
    /// Maps symbol (uppercase) -> Vec of CoinGecko coin_ids
    by_symbol: HashMap<String, Vec<String>>,
}

/// CoinGecko API client with caching and rate limiting
///
/// This struct is cloneable to allow sharing across async tasks.
/// All clones share the same rate limiter and coin cache.
#[derive(Clone)]
pub struct CoinGeckoClient {
    client: Client,
    api_key: String,
    coin_cache: Arc<RwLock<CoinCache>>,
    rate_limiter: Arc<CoinGeckoRateLimiter>,
}

/// Response from /coins/list endpoint
#[derive(Debug, Deserialize)]
struct CoinListEntry {
    id: String,
    symbol: String,
    #[allow(dead_code)]
    name: String,
    platforms: HashMap<String, String>,
}

/// Response from /market_chart endpoint
#[derive(Debug, Deserialize)]
pub struct MarketChartData {
    pub prices: Vec<[f64; 2]>,        // [timestamp_ms, price]
    pub total_volumes: Vec<[f64; 2]>, // [timestamp_ms, volume_24h]
}

impl CoinGeckoClient {
    /// Create a new CoinGecko client and load coin cache
    pub async fn new(api_key: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        // Create rate limiter: 30 requests per minute
        let quota = Quota::per_minute(NonZeroU32::new(RATE_LIMIT_RPM).unwrap());
        let rate_limiter = Arc::new(RateLimiter::direct(quota));

        let mut instance = Self {
            client,
            api_key,
            coin_cache: Arc::new(RwLock::new(CoinCache::default())),
            rate_limiter,
        };

        // Load coin cache on initialization
        instance.refresh_coin_cache().await?;

        Ok(instance)
    }

    /// Make a rate-limited API request with retry logic
    async fn make_request(&self, url: &str) -> Result<reqwest::Response> {
        for attempt in 1..=MAX_RETRIES {
            // Wait for rate limiter
            self.rate_limiter.until_ready().await;

            match self.client.get(url).send().await {
                Ok(response) => {
                    let status = response.status();

                    if status.is_success() {
                        return Ok(response);
                    }

                    // Handle rate limit errors
                    if status.as_u16() == 429 {
                        let backoff_secs = 2u64.pow(attempt);
                        tracing::warn!(
                            "Rate limited by CoinGecko (429), backing off for {}s (attempt {}/{})",
                            backoff_secs,
                            attempt,
                            MAX_RETRIES
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        continue;
                    }

                    // Handle server errors (5xx)
                    if status.is_server_error() {
                        let backoff_secs = 2u64.pow(attempt);
                        tracing::warn!(
                            "Server error {} from CoinGecko, retrying in {}s (attempt {}/{})",
                            status,
                            backoff_secs,
                            attempt,
                            MAX_RETRIES
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        continue;
                    }

                    // Other errors (4xx) - don't retry
                    let error_text = response
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    anyhow::bail!("CoinGecko API error ({}): {}", status, error_text);
                }
                Err(e) if attempt < MAX_RETRIES => {
                    let backoff_secs = 2u64.pow(attempt);
                    tracing::warn!(
                        "Network error: {}, retrying in {}s (attempt {}/{})",
                        e,
                        backoff_secs,
                        attempt,
                        MAX_RETRIES
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                }
                Err(e) => anyhow::bail!("Network error after {} retries: {}", MAX_RETRIES, e),
            }
        }

        anyhow::bail!("Failed after {} retries", MAX_RETRIES)
    }

    /// Refresh the coin cache from /coins/list API
    async fn refresh_coin_cache(&mut self) -> Result<()> {
        tracing::info!("Loading CoinGecko coin cache...");

        let url = format!(
            "{}/coins/list?include_platform=true&x_cg_demo_api_key={}",
            COINGECKO_API_BASE, self.api_key
        );

        let response = self.make_request(&url).await?;

        let coins: Vec<CoinListEntry> =
            response.json().await.context("Failed to parse coin list")?;

        let mut cache = CoinCache::default();
        let mut solana_count = 0;

        for coin in coins {
            // Check if it has a Solana platform address
            if let Some(solana_address) = coin.platforms.get("solana") {
                // Skip empty addresses (invalid data)
                if !solana_address.is_empty() {
                    cache
                        .by_address
                        .insert(solana_address.clone(), coin.id.clone());
                    cache
                        .by_coin_id
                        .insert(coin.id.clone(), solana_address.clone());
                    solana_count += 1;
                }
            }

            // Also index by symbol (can have multiple coins with same symbol)
            let symbol_upper = coin.symbol.to_uppercase();
            cache
                .by_symbol
                .entry(symbol_upper)
                .or_insert_with(Vec::new)
                .push(coin.id.clone());
        }

        // Special case: Override wrapped-solana with native solana for the wrapped SOL address
        // This ensures we get better market data from the native coin
        cache.by_address.insert(
            "So11111111111111111111111111111111111111112".to_string(),
            "solana".to_string(),
        );
        cache.by_coin_id.insert(
            "solana".to_string(),
            "So11111111111111111111111111111111111111112".to_string(),
        );

        tracing::info!("Loaded {} Solana tokens from CoinGecko", solana_count);

        *self.coin_cache.write().await = cache;

        Ok(())
    }

    /// Find CoinGecko coin_id for a token using three-tier lookup
    pub async fn find_coin_id(&self, symbol: &str, mint_address: &str) -> Result<String> {
        let cache = self.coin_cache.read().await;

        // 1. Try exact mint address match (most reliable)
        if let Some(coin_id) = cache.by_address.get(mint_address) {
            tracing::debug!("Found {} by mint address: {}", symbol, coin_id);
            return Ok(coin_id.clone());
        }

        // 2. Try symbol match, prefer ones with Solana platform
        if let Some(coin_ids) = cache.by_symbol.get(&symbol.to_uppercase()) {
            // First try to find one that has a Solana platform
            for coin_id in coin_ids {
                if cache.by_coin_id.contains_key(coin_id) {
                    tracing::debug!(
                        "Found {} by symbol with Solana platform: {}",
                        symbol,
                        coin_id
                    );
                    return Ok(coin_id.clone());
                }
            }

            // Fall back to first match
            tracing::warn!(
                "Found {} by symbol only (no Solana platform): {}",
                symbol,
                coin_ids[0]
            );
            return Ok(coin_ids[0].clone());
        }

        // 3. Not found
        anyhow::bail!("Token {} ({}) not found in CoinGecko", symbol, mint_address)
    }

    /// Fetch market chart data (price and volume time series)
    pub async fn get_market_chart(&self, coin_id: &str, days: u32) -> Result<MarketChartData> {
        let url = format!(
            "{}/coins/{}/market_chart?vs_currency=usd&days={}&x_cg_demo_api_key={}",
            COINGECKO_API_BASE, coin_id, days, self.api_key
        );

        tracing::debug!("Fetching market chart for {} ({}d)", coin_id, days);

        let response = self.make_request(&url).await?;

        let data: MarketChartData = response
            .json()
            .await
            .context("Failed to parse market chart")?;

        tracing::debug!("Fetched {} price points for {}", data.prices.len(), coin_id);

        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to get API key from env or skip test
    fn get_test_api_key() -> Option<String> {
        std::env::var("COINGECKO_API_KEY").ok()
    }

    #[tokio::test]
    #[ignore] // Requires API key and network
    async fn test_load_coin_cache() {
        let api_key = get_test_api_key().expect("COINGECKO_API_KEY not set");
        let client = CoinGeckoClient::new(api_key).await.unwrap();

        let cache = client.coin_cache.read().await;

        // Should have thousands of Solana tokens
        assert!(
            cache.by_address.len() > 5000,
            "Expected >5000 Solana tokens, got {}",
            cache.by_address.len()
        );

        // Check that inverted map is close (may differ slightly due to manual overrides)
        let size_diff = (cache.by_address.len() as i32 - cache.by_coin_id.len() as i32).abs();
        assert!(
            size_diff <= 1,
            "by_address and by_coin_id should have similar sizes, got {} vs {}",
            cache.by_address.len(),
            cache.by_coin_id.len()
        );

        // Should have symbol mappings
        assert!(
            cache.by_symbol.len() > 1000,
            "Expected >1000 symbols, got {}",
            cache.by_symbol.len()
        );
    }

    #[tokio::test]
    #[ignore] // Requires API key and network
    async fn test_find_coin_id_by_address() {
        let api_key = get_test_api_key().expect("COINGECKO_API_KEY not set");
        let client = CoinGeckoClient::new(api_key).await.unwrap();

        // Jupiter token
        let coin_id = client
            .find_coin_id("JUP", "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN")
            .await
            .unwrap();

        assert_eq!(coin_id, "jupiter-exchange-solana");
    }

    #[tokio::test]
    #[ignore] // Requires API key and network
    async fn test_find_coin_id_native_sol() {
        let api_key = get_test_api_key().expect("COINGECKO_API_KEY not set");
        let client = CoinGeckoClient::new(api_key).await.unwrap();

        // Native SOL (wrapped address)
        let coin_id = client
            .find_coin_id("SOL", "So11111111111111111111111111111111111111112")
            .await
            .unwrap();

        assert_eq!(coin_id, "solana");
    }

    #[tokio::test]
    #[ignore] // Requires API key and network
    async fn test_find_coin_id_not_found() {
        let api_key = get_test_api_key().expect("COINGECKO_API_KEY not set");
        let client = CoinGeckoClient::new(api_key).await.unwrap();

        // Fake token
        let result = client
            .find_coin_id("FAKECOIN", "FakeAddress12345678901234567890123456")
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    #[ignore] // Requires API key and network
    async fn test_get_market_chart_live() {
        let api_key = get_test_api_key().expect("COINGECKO_API_KEY not set");
        let client = CoinGeckoClient::new(api_key).await.unwrap();

        // Get 1 day of data for Solana
        let data = client.get_market_chart("solana", 1).await.unwrap();

        // Should have ~287 price points for 1 day
        assert!(
            data.prices.len() > 200,
            "Expected >200 price points, got {}",
            data.prices.len()
        );

        // Should have volume data
        assert_eq!(data.prices.len(), data.total_volumes.len());

        // Validate data format
        for price_point in &data.prices[..5] {
            assert!(price_point[0] > 0.0, "Timestamp should be positive");
            assert!(price_point[1] > 0.0, "Price should be positive");
        }
    }
}
