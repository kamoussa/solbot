use crate::Result;
use reqwest::Client;
use serde::Deserialize;

const BIRDEYE_API_BASE: &str = "https://public-api.birdeye.so";

/// Client for Birdeye API (Solana data)
///
/// **FREE TIER LIMITATIONS:**
/// - Only `/defi/price` and `/defi/token_trending` are available
/// - OHLCV, security, trades, price_volume require premium
#[derive(Clone)]
pub struct BirdeyeClient {
    client: Client,
    api_key: String,
}

// ============== Response Types ==============

#[derive(Debug, Deserialize)]
struct BirdeyeResponse<T> {
    data: T,
    success: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PriceData {
    value: f64,
    #[serde(default)]
    liquidity: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrendingData {
    tokens: Vec<TrendingTokenRaw>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrendingTokenRaw {
    address: String,
    symbol: String,
    name: String,
    decimals: u8,
    liquidity: f64,
    #[serde(rename = "volume24hUSD")]
    volume_24h_usd: f64,
    #[serde(rename = "volume24hChangePercent")]
    volume_24h_change_percent: Option<f64>,
    fdv: Option<f64>,
    marketcap: Option<f64>,
    rank: Option<u32>,
    price: f64,
    #[serde(rename = "price24hChangePercent")]
    price_24h_change_percent: Option<f64>,
}

// ============== Public Types ==============

#[derive(Debug, Clone)]
pub struct TrendingToken {
    pub address: String,
    pub symbol: String,
    pub name: String,
    pub decimals: u8,
    pub liquidity_usd: f64,
    pub volume_24h_usd: f64,
    pub volume_24h_change_percent: f64,
    pub fdv: f64,
    pub marketcap: f64,
    pub rank: u32,
    pub price: f64,
    pub price_24h_change_percent: f64,
}

impl From<TrendingTokenRaw> for TrendingToken {
    fn from(raw: TrendingTokenRaw) -> Self {
        TrendingToken {
            address: raw.address,
            symbol: raw.symbol,
            name: raw.name,
            decimals: raw.decimals,
            liquidity_usd: raw.liquidity,
            volume_24h_usd: raw.volume_24h_usd,
            volume_24h_change_percent: raw.volume_24h_change_percent.unwrap_or(0.0),
            fdv: raw.fdv.unwrap_or(0.0),
            marketcap: raw.marketcap.unwrap_or(0.0),
            rank: raw.rank.unwrap_or(9999),
            price: raw.price,
            price_24h_change_percent: raw.price_24h_change_percent.unwrap_or(0.0),
        }
    }
}

// ============== Implementation ==============

impl BirdeyeClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    /// Get current price for a token
    /// Endpoint: GET /defi/price?address={address}&include_liquidity=true
    ///
    /// **FREE TIER:** ✅ Available
    pub async fn get_price(&self, address: &str) -> Result<(f64, Option<f64>)> {
        let url = format!(
            "{}/defi/price?address={}&include_liquidity=true",
            BIRDEYE_API_BASE, address
        );

        let response = self
            .client
            .get(&url)
            .header("X-API-KEY", &self.api_key)
            .header("x-chain", "solana")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("Birdeye API error: {}", response.status()).into());
        }

        let data: BirdeyeResponse<PriceData> = response.json().await?;

        if !data.success {
            return Err("Birdeye API returned success=false".into());
        }

        Ok((data.data.value, data.data.liquidity))
    }

    /// Get trending tokens
    /// Endpoint: GET /defi/token_trending?sort_by={sort_by}&sort_type={sort_type}&offset={offset}&limit={limit}
    ///
    /// **FREE TIER:** ✅ Available (up to 1000 tokens!)
    ///
    /// sort_by options:
    /// - "rank" - Most popular by Birdeye rank
    /// - "volume24hUSD" - Highest 24h volume
    /// - "volume24hChangePercent" - Biggest volume increase
    /// - "price24hChangePercent" - Biggest price change
    ///
    /// sort_type options:
    /// - "asc" - Ascending order
    /// - "desc" - Descending order
    ///
    /// offset: Pagination offset (default 0)
    /// limit: Number of tokens to return (1-1000, default 20)
    pub async fn get_trending(
        &self,
        sort_by: &str,
        sort_type: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<TrendingToken>> {
        let url = format!(
            "{}/defi/token_trending?sort_by={}&sort_type={}&offset={}&limit={}",
            BIRDEYE_API_BASE, sort_by, sort_type, offset, limit
        );

        let response = self
            .client
            .get(&url)
            .header("X-API-KEY", &self.api_key)
            .header("x-chain", "solana")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("Birdeye API error: {}", response.status()).into());
        }

        let data: BirdeyeResponse<TrendingData> = response.json().await?;

        if !data.success {
            return Err("Birdeye API returned success=false".into());
        }

        // Convert to our public type
        let tokens = data
            .data
            .tokens
            .into_iter()
            .map(TrendingToken::from)
            .collect();

        Ok(tokens)
    }
}

// ============== PREMIUM TIER ENDPOINTS (commented out) ==============
//
// The following endpoints require a premium Birdeye subscription:
//
// ❌ get_ohlcv() - Real OHLCV candles
// ❌ get_price_volume() - Price + volume combined
// ❌ get_security() - Security checks (mint auth, freeze, etc.)
// ❌ get_trades() - Recent trades for buy/sell flow
// ❌ get_exit_liquidity() - Exit liquidity calculation
//
// For now, we use:
// - DexScreener for candles (already implemented)
// - Manual allowlist for security (no API needed)
// - Trending data for discovery (has most metadata we need)

#[cfg(test)]
mod tests {
    use super::*;

    fn get_test_client() -> BirdeyeClient {
        let api_key = std::env::var("BIRDEYE_API_KEY").unwrap_or_else(|_| "test_key".to_string());
        BirdeyeClient::new(api_key)
    }

    #[tokio::test]
    #[ignore] // Ignore by default to avoid hitting API in tests
    async fn test_get_price_live() {
        let client = get_test_client();

        // SOL mint address
        let sol_mint = "So11111111111111111111111111111111111111112";

        let result = client.get_price(sol_mint).await;

        if let Err(ref e) = result {
            eprintln!("ERROR: {}", e);
        }

        assert!(result.is_ok(), "API call failed: {:?}", result.err());

        let (price, liquidity) = result.unwrap();
        assert!(price > 0.0);
        assert!(liquidity.is_some());
        assert!(liquidity.unwrap() > 0.0);

        println!(
            "SOL price: ${:.2}, Liquidity: ${:.0}",
            price,
            liquidity.unwrap()
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_trending_live() {
        let client = get_test_client();

        let result = client.get_trending("volume24hUSD", "desc", 0, 10).await;

        if let Err(ref e) = result {
            eprintln!("ERROR: {}", e);
        }

        assert!(result.is_ok(), "API call failed: {:?}", result.err());

        let tokens = result.unwrap();
        assert!(!tokens.is_empty());
        assert!(tokens.len() <= 10);

        println!("Got {} trending tokens", tokens.len());
        for (i, token) in tokens.iter().enumerate() {
            println!(
                "{}. {} ({}) - Price: ${:.6}, Vol: ${:.0}, Liq: ${:.0}, Change: {:.2}%",
                i + 1,
                token.symbol,
                token.name,
                token.price,
                token.volume_24h_usd,
                token.liquidity_usd,
                token.price_24h_change_percent
            );
        }

        // Validate token structure
        for token in &tokens {
            assert!(!token.address.is_empty());
            assert!(!token.symbol.is_empty());
            assert!(token.price >= 0.0);
            assert!(token.liquidity_usd >= 0.0);
            assert!(token.volume_24h_usd >= 0.0);
        }
    }

    #[test]
    fn test_client_creation() {
        let client = BirdeyeClient::new("test_key".to_string());
        assert_eq!(client.api_key, "test_key");
    }

    /// Test safety filter with fresh Birdeye data
    #[tokio::test]
    #[ignore]
    async fn test_safety_filter() {
        use crate::discovery::safety::is_safe_token;

        let client = get_test_client();

        println!("\n=== TESTING SAFETY FILTER WITH FRESH BIRDEYE DATA ===\n");

        // Fetch top tokens by rank (most established)
        let result = client.get_trending("rank", "asc", 0, 20).await;
        assert!(result.is_ok(), "API call failed: {:?}", result.err());

        let tokens = result.unwrap();
        println!("Fetched {} trending tokens\n", tokens.len());

        let mut safe_count = 0;
        let mut rejected_count = 0;

        for (i, token) in tokens.iter().enumerate() {
            let (is_safe, reason) = is_safe_token(token);

            let status = if is_safe { "✅ SAFE" } else { "❌ REJECT" };

            println!("{}. {} ({}) - {}", i + 1, token.symbol, token.name, status);
            println!("   Address: {}", token.address);
            println!("   Price: ${:.6}, Rank: {}", token.price, token.rank);
            println!(
                "   Liquidity: ${:.0}, Volume 24h: ${:.0}",
                token.liquidity_usd, token.volume_24h_usd
            );
            println!("   FDV: ${:.0}, MCap: ${:.0}", token.fdv, token.marketcap);

            if token.fdv > 0.0 {
                let liq_ratio = (token.liquidity_usd / token.fdv) * 100.0;
                println!("   Liq/FDV ratio: {:.3}%", liq_ratio);
            }

            let vol_liq_ratio = token.volume_24h_usd / token.liquidity_usd.max(1.0);
            println!("   Vol/Liq ratio: {:.2}x", vol_liq_ratio);

            println!("   Reason: {}", reason);
            println!();

            if is_safe {
                safe_count += 1;
            } else {
                rejected_count += 1;
            }
        }

        println!("\n=== SUMMARY ===");
        println!("Total: {}", tokens.len());
        println!(
            "Safe: {} ({:.1}%)",
            safe_count,
            (safe_count as f64 / tokens.len() as f64) * 100.0
        );
        println!(
            "Rejected: {} ({:.1}%)",
            rejected_count,
            (rejected_count as f64 / tokens.len() as f64) * 100.0
        );
    }
}
