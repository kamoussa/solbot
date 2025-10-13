use crate::Result;
use reqwest::Client;
use serde::Deserialize;

// Jupiter Swap API v1
// Docs: https://dev.jup.ag/docs/swap-api/get-quote
const JUPITER_QUOTE_API: &str = "https://lite-api.jup.ag/swap/v1";

/// Client for Jupiter aggregator API
#[derive(Clone)]
pub struct JupiterClient {
    client: Client,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuoteResponse {
    input_mint: String,
    in_amount: String,
    output_mint: String,
    out_amount: String,
    other_amount_threshold: String,
    price_impact_pct: String,
    #[serde(default)]
    route_plan: Vec<serde_json::Value>,  // Complex nested structure, using Value for now
    context_slot: Option<u64>,
}

/// Quote information from Jupiter
#[derive(Debug, Clone)]
pub struct Quote {
    pub price: f64,           // Output per unit of input
    pub price_impact_pct: f64, // Price impact percentage
    pub in_amount: u64,
    pub out_amount: u64,
}

impl JupiterClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Get a quote for swapping tokens
    ///
    /// # Arguments
    /// * `input_mint` - Input token mint address
    /// * `output_mint` - Output token mint address
    /// * `amount` - Amount in raw units (e.g., lamports for SOL)
    /// * `slippage_bps` - Slippage tolerance in basis points (50 = 0.5%)
    pub async fn get_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u16,
    ) -> Result<Quote> {
        let url = format!(
            "{}/quote?inputMint={}&outputMint={}&amount={}&slippageBps={}",
            JUPITER_QUOTE_API, input_mint, output_mint, amount, slippage_bps
        );
        tracing::info!("URL: {}", url);

        let response: QuoteResponse = self.client
            .get(&url)
            .send()
            .await?
            .json()
            .await?;
        tracing::info!("Response: {:?}", response);

        let in_amount: u64 = response.in_amount.parse()?;
        let out_amount: u64 = response.out_amount.parse()?;
        let price_impact: f64 = response.price_impact_pct.parse().unwrap_or(0.0);

        // Calculate price (output per unit of input)
        // Note: Both amounts are in raw units (lamports/smallest unit)
        // For SOL->USDC: SOL has 9 decimals, USDC has 6 decimals
        // Price = (out_amount / 10^6) / (in_amount / 10^9) = (out_amount / in_amount) * 1000
        // But this is token-specific. For now, just return the ratio
        // Caller needs to handle decimal conversion based on token specs
        let price = out_amount as f64 / in_amount as f64;

        Ok(Quote {
            price,
            price_impact_pct: price_impact,
            in_amount,
            out_amount,
        })
    }

    /// Execute a swap (placeholder - requires wallet integration)
    pub async fn execute_swap(
        &self,
        _input_mint: &str,
        _output_mint: &str,
        _amount: u64,
    ) -> Result<String> {
        // TODO: Implement actual swap execution with wallet signing
        Err("Swap execution not yet implemented".into())
    }
}

impl Default for JupiterClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    //#[ignore]  // Ignore by default to avoid hitting API in tests
    async fn test_get_quote_live() {
        let client = JupiterClient::new();

        // Get quote for SOL -> USDC
        let sol_mint = "So11111111111111111111111111111111111111112";
        let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let amount = 1_000_000_000; // 1 SOL (9 decimals)
        let slippage = 50; // 0.5% slippage

        let result = client.get_quote(sol_mint, usdc_mint, amount, slippage).await;
        assert!(result.is_ok());

        let quote = result.unwrap();
        assert!(quote.price > 0.0);
        assert!(quote.in_amount == amount);
        assert!(quote.out_amount > 0);
    }

    #[test]
    fn test_client_creation() {
        let client = JupiterClient::new();
        drop(client);
    }
}
