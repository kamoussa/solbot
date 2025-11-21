/// LLM-powered DIRECT strategy selection using OpenAI API
///
/// Instead of detecting regime → mapping to strategy,
/// the LLM directly recommends which strategy to use based on market conditions

use crate::models::Candle;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";
const MODEL: &str = "gpt-4o-mini"; // GPT-4o-mini: 16x cheaper, still excellent quality
const MAX_TOKENS: u32 = 1024;
const RATE_LIMIT_DELAY_MS: u64 = 2500; // 2.5 seconds between calls to avoid rate limits
const MAX_RETRIES: u32 = 3; // Retry failed requests up to 3 times

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Strategy {
    Momentum,
    MeanReversion,
    DCA,
}

impl Strategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Strategy::Momentum => "Momentum",
            Strategy::MeanReversion => "MeanReversion",
            Strategy::DCA => "DCA",
        }
    }
}

// NEW: Direct trading signals (Buy/Sell/Hold)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingSignal {
    Buy,
    Sell,
    Hold,
}

impl TradingSignal {
    pub fn as_str(&self) -> &'static str {
        match self {
            TradingSignal::Buy => "Buy",
            TradingSignal::Sell => "Sell",
            TradingSignal::Hold => "Hold",
        }
    }
}

// Position context for LLM trading decisions
#[derive(Debug, Clone)]
pub struct PositionContext {
    pub has_position: bool,
    pub entry_price: Option<f64>,
    pub current_price: f64,
    pub pnl_percent: Option<f64>,
    pub days_held: Option<f64>,
}

#[derive(Debug, Serialize)]
struct OpenAIRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
    temperature: f32,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: MessageContent,
}

#[derive(Debug, Deserialize)]
struct MessageContent {
    content: String,
}

#[derive(Debug, Deserialize)]
pub struct LLMStrategyResponse {
    pub strategy: String,
    pub confidence: f64,
    pub reasoning: String,
}

#[derive(Debug, Deserialize)]
pub struct LLMTradingResponse {
    pub action: String,  // "Buy", "Sell", or "Hold"
    pub confidence: f64,
    pub reasoning: String,
}

pub struct LLMStrategySelector {
    api_key: String,
    client: reqwest::Client,
    cache: HashMap<String, (Strategy, f64)>, // Cache responses to avoid API costs
    disable_cache: bool, // Disable cache for backtesting
}

impl LLMStrategySelector {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
            cache: HashMap::new(),
            disable_cache: false,
        }
    }

    pub fn new_no_cache(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
            cache: HashMap::new(),
            disable_cache: true,
        }
    }

    /// Select strategy using LLM API
    ///
    /// Returns (Strategy, confidence_score, reasoning) where confidence is 0.0-1.0
    pub async fn select_strategy_with_confidence(
        &mut self,
        candles: &[Candle],
    ) -> Result<(Strategy, f64, String)> {
        self.select_strategy_with_confidence_and_context(candles, None, None).await
    }

    /// NEW: Version with optional strategy context and drawdown analysis
    pub async fn select_strategy_with_confidence_and_context(
        &mut self,
        candles: &[Candle],
        strategy_context: Option<String>,
        drawdown_context: Option<String>,
    ) -> Result<(Strategy, f64, String)> {
        if candles.len() < 50 {
            return Ok((Strategy::DCA, 0.0, "Not enough data".to_string()));
        }

        // Create cache key from last candle timestamp
        let cache_key = candles.last().unwrap().timestamp.to_rfc3339();

        // Check cache first (only if cache is enabled)
        // Note: Cache doesn't store reasoning, so return empty string
        if !self.disable_cache {
            if let Some(cached) = self.cache.get(&cache_key) {
                return Ok((cached.0, cached.1, String::from("Cached result")));
            }
        }

        // Prepare market data for LLM
        let prompt = self.create_prompt_with_context(candles, strategy_context, drawdown_context)?;

        // Retry loop with exponential backoff
        let mut retry_count = 0;
        let mut last_error = String::new();

        loop {
            // Rate limiting: Add delay between calls (except for first attempt)
            if retry_count > 0 {
                let delay_ms = RATE_LIMIT_DELAY_MS * (2_u64.pow(retry_count - 1)); // Exponential backoff
                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            } else if !self.disable_cache {
                // Add delay even on first call to respect rate limits
                tokio::time::sleep(tokio::time::Duration::from_millis(RATE_LIMIT_DELAY_MS)).await;
            }

            // Call OpenAI API
            let request = OpenAIRequest {
                model: MODEL.to_string(),
                max_tokens: MAX_TOKENS,
                temperature: 0.0, // Deterministic responses
                messages: vec![
                    Message {
                        role: "system".to_string(),
                        content: "You are an expert cryptocurrency trading strategist. Analyze market data and recommend the best trading strategy. Always respond with valid JSON only, no markdown formatting.".to_string(),
                    },
                    Message {
                        role: "user".to_string(),
                        content: prompt.clone(),
                    },
                ],
            };

            let response = match self
                .client
                .post(OPENAI_API_URL)
                .header("Authorization", format!("Bearer {}", &self.api_key))
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_error = format!("Network error: {}", e);
                    retry_count += 1;
                    if retry_count >= MAX_RETRIES {
                        return Err(last_error.into());
                    }
                    continue;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                last_error = format!("OpenAI API error {}: {}", status, body);

                // Retry on rate limit errors (429)
                if status.as_u16() == 429 {
                    retry_count += 1;
                    if retry_count >= MAX_RETRIES {
                        return Err(last_error.into());
                    }
                    continue;
                }

                // Don't retry on other errors
                return Err(last_error.into());
            }

            let openai_response: OpenAIResponse = match response.json().await {
                Ok(r) => r,
                Err(e) => {
                    last_error = format!("JSON decode error: {}", e);
                    retry_count += 1;
                    if retry_count >= MAX_RETRIES {
                        return Err(last_error.into());
                    }
                    continue;
                }
            };

            let mut text = openai_response.choices[0].message.content.clone();

            // Strip markdown code blocks (```json ... ``` or ``` ... ```)
            if text.starts_with("```") {
                text = text
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim()
                    .to_string();
            }

            // Parse JSON response
            let llm_response: LLMStrategyResponse = match serde_json::from_str(&text) {
                Ok(r) => r,
                Err(e) => {
                    last_error = format!("JSON parse error: {} (text: {})", e, text);
                    retry_count += 1;
                    if retry_count >= MAX_RETRIES {
                        return Err(last_error.into());
                    }
                    continue;
                }
            };

            // Convert string strategy to enum
            let strategy = match llm_response.strategy.as_str() {
                "Momentum" => Strategy::Momentum,
                "MeanReversion" => Strategy::MeanReversion,
                _ => Strategy::DCA,
            };

            let confidence = llm_response.confidence.clamp(0.0, 1.0);
            let reasoning = llm_response.reasoning;

            // Cache result (note: we don't cache reasoning for simplicity)
            if !self.disable_cache {
                self.cache.insert(cache_key, (strategy, confidence));
            }

            return Ok((strategy, confidence, reasoning));
        }
    }

    /// Create prompt with optional strategy and drawdown context
    fn create_prompt_with_context(
        &self,
        candles: &[Candle],
        strategy_context: Option<String>,
        drawdown_context: Option<String>,
    ) -> Result<String> {
        // Get base prompt
        let mut prompt = self.create_prompt(candles)?;

        // Prepend context sections if provided
        if let Some(strat_ctx) = strategy_context {
            prompt = format!("{}\n\n{}", strat_ctx, prompt);
        }
        if let Some(dd_ctx) = drawdown_context {
            prompt = format!("{}\n\n{}", dd_ctx, prompt);
        }

        Ok(prompt)
    }

    /// Create prompt for LLM with market data and strategy descriptions
    fn create_prompt(&self, candles: &[Candle]) -> Result<String> {
        // Take last 50 candles for analysis (reduce token count)
        let recent = if candles.len() > 50 {
            &candles[candles.len() - 50..]
        } else {
            candles
        };

        // Calculate indicators
        let prices: Vec<f64> = recent.iter().map(|c| c.close).collect();
        let rsi = crate::indicators::calculate_rsi(&prices, 14).unwrap_or(50.0);
        let (adx, plus_di, minus_di) =
            crate::indicators::calculate_adx(recent, 14).unwrap_or((20.0, 20.0, 20.0));
        let atr = crate::indicators::calculate_atr(recent, 14).unwrap_or(0.0);
        let sma_20 = crate::indicators::calculate_sma(&prices, 20).unwrap_or(prices[prices.len() - 1]);
        let current_price = prices[prices.len() - 1];

        // Calculate price statistics
        let price_change_1h = ((current_price - prices[prices.len() - 2]) / prices[prices.len() - 2]) * 100.0;
        let price_change_24h =
            if prices.len() >= 24 {
                ((current_price - prices[prices.len() - 24]) / prices[prices.len() - 24]) * 100.0
            } else {
                0.0
            };

        // ⭐ CRITICAL FIX: Calculate 14-day (336-hour) price change for Momentum detection
        let price_change_14d =
            if candles.len() >= 336 {  // 14 days × 24 hours = 336 hours
                let price_14d_ago = candles[candles.len() - 336].close;
                ((current_price - price_14d_ago) / price_14d_ago) * 100.0
            } else {
                0.0
            };

        // Calculate 10-day and 7-day changes for early momentum detection
        let price_change_10d =
            if candles.len() >= 240 {  // 10 days × 24 hours = 240 hours
                let price_10d_ago = candles[candles.len() - 240].close;
                ((current_price - price_10d_ago) / price_10d_ago) * 100.0
            } else {
                0.0
            };

        let price_change_7d =
            if candles.len() >= 168 {  // 7 days × 24 hours = 168 hours
                let price_7d_ago = candles[candles.len() - 168].close;
                ((current_price - price_7d_ago) / price_7d_ago) * 100.0
            } else {
                0.0
            };

        let price_change_3d =
            if candles.len() >= 72 {  // 3 days × 24 hours = 72 hours
                let price_3d_ago = candles[candles.len() - 72].close;
                ((current_price - price_3d_ago) / price_3d_ago) * 100.0
            } else {
                0.0
            };

        let price_max_50 = prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let price_min_50 = prices.iter().cloned().fold(f64::INFINITY, f64::min);
        let price_range = ((price_max_50 - price_min_50) / price_min_50) * 100.0;

        // Calculate 90-day context
        let lookback_90d = std::cmp::min(candles.len(), 24 * 90); // 90 days of hourly data
        let start_90d = candles.len().saturating_sub(lookback_90d);
        let prices_90d: Vec<f64> = candles[start_90d..].iter().map(|c| c.close).collect();
        let max_90d = prices_90d.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let min_90d = prices_90d.iter().cloned().fold(f64::INFINITY, f64::min);
        let range_90d = max_90d - min_90d;
        let percentile_90d = if range_90d > 0.0 {
            ((current_price - min_90d) / range_90d) * 100.0
        } else {
            50.0
        };
        let drawdown_from_high = ((current_price - max_90d) / max_90d) * 100.0;
        let gain_from_low = ((current_price - min_90d) / min_90d) * 100.0;

        // Volume analysis
        let avg_volume = recent.iter().map(|c| c.volume).sum::<f64>() / recent.len() as f64;
        let current_volume = recent.last().unwrap().volume;
        let volume_ratio = current_volume / avg_volume;

        // Market structure
        let market_structure = crate::indicators::analyze_market_structure(recent, 20);
        let structure_str = match market_structure {
            crate::indicators::MarketStructure::HigherHighsHigherLows => "Higher Highs & Higher Lows (uptrend)",
            crate::indicators::MarketStructure::LowerHighsLowerLows => "Lower Highs & Lower Lows (downtrend)",
            crate::indicators::MarketStructure::Mixed => "Mixed (no clear structure)",
        };

        // Format candle data (last 20 for context)
        let last_20 = if recent.len() > 20 {
            &recent[recent.len() - 20..]
        } else {
            recent
        };

        let candle_data: Vec<String> = last_20
            .iter()
            .map(|c| {
                format!(
                    "{{\"time\": \"{}\", \"open\": {:.2}, \"high\": {:.2}, \"low\": {:.2}, \"close\": {:.2}, \"volume\": {:.0}}}",
                    c.timestamp.format("%Y-%m-%d %H:%M"),
                    c.open,
                    c.high,
                    c.low,
                    c.close,
                    c.volume
                )
            })
            .collect();

        // TREND START DATE DETECTION: Calculate when current price trend began
        // Helps LLM identify if this is an EARLY opportunity (good) or LATE (risky)
        let daily_prices: Vec<f64> = recent
            .iter()
            .rev()
            .step_by(24) // ~24 hours per day
            .take(60) // Last 60 days
            .map(|c| c.close)
            .collect();

        let trend_age_days = if daily_prices.len() >= 3 {
            let mut consecutive_direction = 0;
            let first_direction = if daily_prices[0] > daily_prices[1] { 1 } else { -1 };

            // Count consecutive days moving in same direction
            for i in 0..daily_prices.len() - 1 {
                let curr_direction = if daily_prices[i] > daily_prices[i + 1] { 1 } else { -1 };
                if curr_direction == first_direction {
                    consecutive_direction += 1;
                } else {
                    break;
                }
            }

            consecutive_direction
        } else {
            0
        };

        // TREND MATURITY ANALYSIS: Calculate how long current trend has been running
        // This helps the LLM identify if momentum is early (good) or late (risky)
        let weekly_prices: Vec<f64> = recent
            .iter()
            .rev()
            .step_by(168) // ~168 hours per week
            .take(12) // Last 12 weeks
            .map(|c| c.close)
            .collect();

        let (uptrend_weeks, downtrend_weeks, trend_maturity_note) = if weekly_prices.len() >= 3 {
            let mut consecutive_up = 0;
            let mut consecutive_down = 0;

            // Check for consecutive higher/lower weekly closes (from most recent backwards)
            for i in 0..weekly_prices.len() - 1 {
                let curr = weekly_prices[i]; // More recent
                let prev = weekly_prices[i + 1]; // Older

                if curr > prev {
                    consecutive_up += 1;
                    if consecutive_down > 0 {
                        break; // Stop at first non-down week
                    }
                } else if curr < prev {
                    consecutive_down += 1;
                    if consecutive_up > 0 {
                        break; // Stop at first non-up week
                    }
                } else {
                    break; // Flat week breaks the streak
                }
            }

            let note = if consecutive_up >= 2 {
                format!(
                    "UPTREND ({}",
                    if consecutive_up <= 3 { "young, 2-3 weeks - EARLY stage, room to run" }
                    else if consecutive_up <= 6 { "mature, 4-6 weeks - MID stage, proceed with caution" }
                    else { "old, 7+ weeks - LATE stage, reversal risk high" }
                )
            } else if consecutive_down >= 2 {
                format!(
                    "DOWNTREND ({}",
                    if consecutive_down <= 3 { "young, 2-3 weeks - EARLY decline, may bounce soon" }
                    else if consecutive_down <= 6 { "mature, 4-6 weeks - MID decline, watch for capitulation" }
                    else { "old, 7+ weeks - LATE decline, potential bottom forming" }
                )
            } else {
                "SIDEWAYS/CHOPPY (no consistent multi-week trend - DCA best)".to_string()
            };

            (consecutive_up, consecutive_down, note)
        } else {
            (0, 0, "Insufficient data for trend maturity".to_string())
        };

        let prompt = format!(
            r#"You are an expert cryptocurrency trading strategist. Your goal is to maximize returns by selecting the right strategy for current market conditions.

⚠️ IMPORTANT: Switching strategies has COSTS (slippage, missed opportunities, whipsaw). Only switch when you have HIGH CONVICTION (0.85-0.90+ confidence) that market conditions have fundamentally changed.

## Market Summary (Last 50 Hours)
- **Current Price**: ${:.2}
- **1H Change**: {:+.2}%
- **24H Change**: {:+.2}%
- **3-Day Change**: {:+.2}% ← Acceleration signal
- **7-Day Change**: {:+.2}% ← Early momentum signal
- **10-Day Change**: {:+.2}% ← Primary momentum trigger
- **14-Day Change**: {:+.2}% ← Confirmation
- **50H Price Range**: {:.1}% (High: ${:.2}, Low: ${:.2})
- **Price vs SMA(20)**: {:+.1}% {}

## 90-Day Historical Context
- **Price Position**: {:.0}th percentile of 90-day range
  - 90-Day Low: ${:.2} | 90-Day High: ${:.2}
  - Current: ${:.2} {}
- **From 90-Day High**: {:+.1}%
- **From 90-Day Low**: {:+.1}%

## Technical Indicators
- **RSI(14)**: {:.1} {}
- **ADX(14)**: {:.1} (Trend strength)
  - +DI: {:.1}, -DI: {:.1}
  - DI Spread: {:+.1} (positive = bullish, negative = bearish)
- **ATR(14)**: {:.2} (Volatility)
- **Volume**: {:.0} (Current) vs {:.0} (Avg) = {:.2}x {}

## Market Structure
- **Pattern**: {}
- **Recent candles** (last 20 hours):
{}

## Available Strategies - Choose Based on Market Conditions

### 🚀 1. Momentum Strategy - Capture SUSTAINED Bull Runs (Target: 15-20% of Year)
- **When to use**: Clear, SUSTAINED uptrends (not just 1-2 good days)
- **Opportunity**: Catch +20-30% moves during multi-week bull runs
- **Best timing**: Enter when trend is building but still has room to run
- **✅ REQUIREMENTS - Need SOLID signals (confidence 0.80-0.95)**:
  1. **Price up 6%+ in last 7-10 days** (sustained move, not just a bounce)
  2. **ADX > 20** (clear trend forming, not just noise)
  3. **RSI 50-75** (momentum present but not overbought)
  4. **Price > SMA(20) preferred** (structure confirms uptrend)
- **Confidence levels** (UPDATED for 48h sampling):
  - 6-8% in 7-10d + ADX >20 + RSI 50-75 → 0.80-0.85 confidence
  - 8-10% in 7-10d + ADX >23 + RSI 52-70 → 0.85-0.88 confidence
  - 10%+ in 10d + ADX >25 + acceleration signals → 0.90+ confidence
- **Acceleration signals** (boost confidence +0.05):
  - 3-day change > 3%: Momentum building
  - 7-day change > 5%: Strong trend
  - Price breaking recent highs: Breakout momentum
- **Entry threshold**: 0.80+ confidence to switch (lowered to catch real bull runs)
- **Why this works**: Catches bull runs early while maintaining conviction
- **Target allocation**: 15-20% of the year (selective but achievable)

### 💥 2. Mean Reversion Strategy - Panic Crashes & Bear Capitulation (Target: 10-15% of Year)
- **When to use**: EXTREME panic crashes OR gradual bear market capitulation
- **Opportunity**: Catch +5-10% bounces after fear peaks
- **Best timing**: Enter AFTER panic selling peaks, when fear is maximum
- **✅ TWO TYPES OF PANIC - Either triggers Mean Reversion**:

  **A) FLASH CRASH (confidence 0.85-0.95) - ALL must be met**:
  - RSI < 20 (extreme oversold)
  - Price dropped > 15% in last 48 hours (massive panic)
  - Volume > 2.0x average (panic selling)
  - Price < 20th percentile of 90-day range

  **B) GRADUAL BEAR CAPITULATION (confidence 0.80-0.90) - ALL must be met**:
  - Price down 20%+ from 30-day high (extended decline)
  - RSI < 30 (oversold, sellers exhausted)
  - Price in bottom 15th percentile of 90-day range (near lows)
  - Recent 3-7 day decline slowing (capitulation ending)

- **Entry threshold**: 0.80+ confidence to switch
- **Target allocation**: 10-15% of the year (includes gradual bears)

### 📊 3. DCA (Dollar Cost Averaging) - Default for Uncertain Markets (Target: 65-75% of Year)
- **When to use**: When markets are choppy, sideways, mixed, OR you lack conviction
- **This IS the default**: Most of the time, markets don't have clear momentum or panic
- **Characteristics**: Steady accumulation, works in uncertainty
- **Use when**: No clear momentum (price change <10% in 14d) AND no panic conditions
- **Entry threshold**: 0.60-0.70 confidence (this is the safe, proven choice)
- **Target allocation**: 65-75% of the year (this is normal and expected!)
- **Why it works**: Returns +1.96% baseline - hard to beat consistently

## CRITICAL RULES FOR STRATEGY SELECTION

**🎯 YOUR GOAL: Select the right strategy AND minimize costly switches**

1. **Switching has REAL COSTS**
   - Every strategy switch costs ~0.5-1% in slippage and missed opportunities
   - Rapid switching (thrashing) destroys returns
   - Only switch with 0.85+ confidence when market fundamentally changes
   - When in doubt, STAY in current strategy

2. **Momentum requires SUSTAINED signals (0.80-0.95 confidence) - UPDATED THRESHOLDS**:
   - **Not just a bounce**: Need 6%+ move over 7-10 days (building trend)
   - **Trend confirmation**: ADX >20, RSI 50-75, price > SMA(20) preferred
   - **Avoid false starts**: Look for multi-day consistency, not single spikes
   - **Confidence tiers** (LOWERED to match 48h sampling):
     - 6-8% in 7-10d + ADX >20 → 0.80-0.85 (good to switch)
     - 8-10% in 7-10d + ADX >23 → 0.85-0.88 (strong signal)
     - 10%+ in 10d + ADX >25 + acceleration → 0.90+ (excellent)
   - **Target**: 15-20% of year (selective opportunities)

3. **Mean Reversion for EXTREMES & GRADUAL BEARS (0.80-0.95 confidence) - UPDATED**:
   - Small dips (RSI 30-40, -5-10% drop): Use DCA, not Mean Reversion
   - Flash crash (RSI <20, -15%+ in 48h, 2x+ volume): Mean Reversion (0.85-0.95)
   - Gradual bear (down 20%+ from 30d high, RSI <30, bottom 15%): Mean Reversion (0.80-0.90)
   - **Target**: 10-15% of year (includes both flash crashes and bear capitulations)

4. **DCA is the PROVEN DEFAULT (0.60-0.70 confidence)**:
   - Returns +1.96% baseline (hard to beat consistently)
   - Use when: no clear momentum (price <8% in 14d) AND no panic
   - **This is NORMAL**: 65-75% of year should be DCA
   - DCA isn't "giving up" - it's the smart choice in choppy markets

5. **Confidence Calibration - UPDATED FOR 48H SAMPLING**:
   - **Momentum**: 0.80-0.95 (HIGH conviction, LOWERED to catch real bull runs)
   - **Mean Reversion**: 0.80-0.95 (flash crashes + gradual bears, LOWERED)
   - **DCA**: 0.60-0.70 (low threshold, this is the safe default)
   - **When uncertain (<0.80)**: Stay in DCA or current strategy

## Decision Framework - Evaluate Carefully Before Switching

**Step 1: CHECK FOR SUSTAINED MOMENTUM** (15-20% of year) - UPDATED THRESHOLDS
- Check recent price changes: 3d, 7d, 10d (focus on shorter windows with 48h sampling)
- **EXCELLENT Momentum** (confidence 0.90-0.95):
  - Price up 10%+ in 7-10 days (strong sustained move)
  - ADX > 25 (very strong trend)
  - RSI 55-70 (strong but not overbought)
  - 3d change >3% OR 7d change >5% (accelerating trend)
  - Price > SMA(20) AND making higher highs
  - → **Recommend MOMENTUM with 0.90-0.95 confidence**

- **STRONG Momentum** (confidence 0.85-0.88):
  - Price up 8-10% in 7-10 days
  - ADX > 23 (clear strong trend)
  - RSI 52-72 (momentum present)
  - Price > SMA(20)
  - → **Recommend MOMENTUM with 0.85-0.88 confidence**

- **GOOD Momentum** (confidence 0.80-0.85):
  - Price up 6-8% in 7-10 days (building trend)
  - ADX > 20 (trend forming)
  - RSI 50-75 (room to run)
  - Price near or above SMA(20)
  - → **Recommend MOMENTUM with 0.80-0.85 confidence**

- **Weak signals** (price <6% in 10d OR ADX <20): → Continue to Step 2

**Step 2: CHECK FOR PANIC / BEAR CAPITULATION** (10-15% of year) - UPDATED
- **Flash Crash Capitulation** (confidence 0.85-0.95):
  - RSI < 20 (extreme oversold)
  - Price -15%+ in 48h (massive drop)
  - Volume > 2x average (panic selling)
  - Price < 20th percentile of 90d range
  - ALL conditions must be met
  - → **Recommend MEAN REVERSION with 0.85-0.95 confidence**

- **Gradual Bear Capitulation** (confidence 0.80-0.90) - NEW:
  - Price down 20%+ from 30-day high (extended decline)
  - RSI < 30 (oversold, sellers exhausted)
  - Price in bottom 15th percentile of 90d range (near lows)
  - Recent 3-7 day decline slowing (capitulation ending)
  - ALL conditions must be met
  - → **Recommend MEAN REVERSION with 0.80-0.90 confidence**

- **Normal dip** (RSI 30-40, modest -10-15% drop): → Continue to Step 3

**Step 3: DEFAULT TO DCA** (60-70% of year - this is normal!) - UPDATED
- No sustained momentum (price <6% in 10d OR ADX <20)
- No panic/bear capitulation conditions
- Markets are choppy, sideways, or uncertain
- Market is choppy, sideways, uncertain, or mixed
- → **Recommend DCA with 0.60-0.70 confidence**

**Step 4: Final confidence check**
- If ALL key signals strongly aligned → Keep high confidence (0.88-0.95)
- If signals are mixed or marginal → Use DCA (0.65)
- Remember: Switching costs money - need 0.85+ to justify a switch!

Respond ONLY with valid JSON (no markdown, no code blocks):

{{
  "strategy": "Momentum|MeanReversion|DCA",
  "confidence": 0.85,
  "reasoning": "Brief explanation (1-2 sentences) of why this strategy was chosen"
}}
"#,
            current_price,
            price_change_1h,
            price_change_24h,
            price_change_3d,
            price_change_7d,
            price_change_10d,
            price_change_14d,
            price_range,
            price_max_50,
            price_min_50,
            ((current_price - sma_20) / sma_20) * 100.0,
            if current_price > sma_20 { "(above)" } else { "(below)" },
            // 90-day context
            percentile_90d,
            min_90d,
            max_90d,
            current_price,
            if percentile_90d < 25.0 {
                "(near bottom)"
            } else if percentile_90d > 75.0 {
                "(near top)"
            } else {
                "(mid-range)"
            },
            drawdown_from_high,
            gain_from_low,
            // Technical indicators
            rsi,
            if rsi > 70.0 {
                "(overbought)"
            } else if rsi < 30.0 {
                "(oversold)"
            } else {
                "(neutral)"
            },
            adx,
            plus_di,
            minus_di,
            plus_di - minus_di,
            atr,
            current_volume,
            avg_volume,
            volume_ratio,
            if volume_ratio > 1.5 {
                "(HIGH)"
            } else if volume_ratio < 0.7 {
                "(LOW)"
            } else {
                "(normal)"
            },
            structure_str,
            candle_data.join(",\n")
        );

        Ok(prompt)
    }

    /// Clear the cache (useful for testing)
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Get cache size
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }

    /// NEW: Get direct trading signal (Buy/Sell/Hold) from LLM
    ///
    /// Returns (TradingSignal, confidence_score, reasoning) where confidence is 0.0-1.0
    pub async fn get_trading_signal_with_context(
        &mut self,
        candles: &[Candle],
        position: &PositionContext,
    ) -> Result<(TradingSignal, f64, String)> {
        if candles.len() < 50 {
            return Ok((TradingSignal::Hold, 0.0, "Not enough data".to_string()));
        }

        // Create cache key from last candle timestamp + position state
        let cache_key = format!(
            "{}_{}_{}",
            candles.last().unwrap().timestamp.to_rfc3339(),
            position.has_position,
            position.entry_price.unwrap_or(0.0)
        );

        // Check cache first (only if cache is enabled)
        // TODO: Implement caching for trading signals if needed

        // Prepare market data + position context for LLM
        // OPTION D: Route to specialized prompts based on position state
        let prompt = if position.has_position {
            self.create_exit_prompt(candles, position)?
        } else {
            self.create_entry_prompt(candles)?
        };

        // Retry loop with exponential backoff (same as strategy selection)
        let mut retry_count = 0;
        let mut last_error = String::new();

        loop {
            // Rate limiting
            if retry_count > 0 {
                let delay_ms = RATE_LIMIT_DELAY_MS * (2_u64.pow(retry_count - 1));
                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            } else if !self.disable_cache {
                tokio::time::sleep(tokio::time::Duration::from_millis(RATE_LIMIT_DELAY_MS)).await;
            }

            // Call OpenAI API
            let request = OpenAIRequest {
                model: MODEL.to_string(),
                max_tokens: MAX_TOKENS,
                temperature: 0.0,
                messages: vec![
                    Message {
                        role: "system".to_string(),
                        content: "You are a professional cryptocurrency swing trader. Analyze market conditions and provide a clear Buy, Sell, or Hold recommendation with confidence score and reasoning.".to_string(),
                    },
                    Message {
                        role: "user".to_string(),
                        content: prompt.clone(),
                    },
                ],
            };

            let response = self
                .client
                .post(OPENAI_API_URL)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        last_error = format!("HTTP {}: {}", resp.status(), resp.text().await.unwrap_or_default());
                        retry_count += 1;
                        if retry_count >= MAX_RETRIES {
                            return Err(anyhow::anyhow!("API request failed after {} retries: {}", MAX_RETRIES, last_error).into());
                        }
                        continue;
                    }

                    let api_response: OpenAIResponse = resp.json().await?;
                    let content = &api_response.choices[0].message.content;

                    // Parse JSON response
                    let parsed: LLMTradingResponse = serde_json::from_str(content)?;

                    // Map action string to TradingSignal enum
                    let signal = match parsed.action.to_lowercase().as_str() {
                        "buy" => TradingSignal::Buy,
                        "sell" => TradingSignal::Sell,
                        "hold" => TradingSignal::Hold,
                        _ => return Err(anyhow::anyhow!("Unknown trading signal: {}", parsed.action).into()),
                    };

                    return Ok((signal, parsed.confidence, parsed.reasoning));
                }
                Err(e) => {
                    last_error = e.to_string();
                    retry_count += 1;
                    if retry_count >= MAX_RETRIES {
                        return Err(anyhow::anyhow!("API request failed after {} retries: {}", MAX_RETRIES, last_error).into());
                    }
                }
            }
        }
    }

    /// Create LLM prompt for ENTRY decisions (when in cash)
    fn create_entry_prompt(&self, candles: &[Candle]) -> Result<String> {
        use crate::indicators::{calculate_rsi, calculate_adx};

        let current = candles.last().unwrap();
        let current_price = current.close;

        // Calculate technical indicators
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let rsi = calculate_rsi(&closes, 14).unwrap_or(50.0);

        let adx_result = calculate_adx(candles, 14);
        let (adx, plus_di, minus_di) = adx_result.unwrap_or((0.0, 0.0, 0.0));

        // Price changes
        let price_24h_ago = candles.get(candles.len().saturating_sub(24)).map(|c| c.close).unwrap_or(current_price);
        let price_7d_ago = candles.get(candles.len().saturating_sub(168)).map(|c| c.close).unwrap_or(current_price);
        let price_14d_ago = candles.get(candles.len().saturating_sub(336)).map(|c| c.close).unwrap_or(current_price);

        let change_24h = ((current_price - price_24h_ago) / price_24h_ago) * 100.0;
        let change_7d = ((current_price - price_7d_ago) / price_7d_ago) * 100.0;
        let change_14d = ((current_price - price_14d_ago) / price_14d_ago) * 100.0;

        let prompt = format!(
            r#"You are analyzing SOL/USD for potential BUY opportunities. You are currently in CASH.

**MARKET DATA:**
- Current price: ${:.2}
- 24H change: {:.2}%
- 7D change: {:.2}%
- 14D change: {:.2}%
- RSI(14): {:.1}
- ADX: {:.1} (+DI: {:.1}, -DI: {:.1})

**DECISION: Should you BUY or WAIT?**

**Buy if you see:**
1. **Momentum building:** 7-14 day uptrend (6%+ gains), RSI 50-70, ADX >20
2. **Oversold bounce:** RSI <30, recent sharp decline, capitulation signs

**Wait if:**
- Choppy/uncertain conditions
- RSI overbought (>75)
- Weak or no clear trend

**OUTPUT (JSON only):**
{{
  "action": "Buy|Hold",
  "confidence": 0.0-1.0,
  "reasoning": "Brief explanation"
}}

**Important:** Only recommend Buy with 0.70+ confidence if you see a clear opportunity."#,
            current_price,
            change_24h,
            change_7d,
            change_14d,
            rsi,
            adx,
            plus_di,
            minus_di
        );

        Ok(prompt)
    }

    /// Create LLM prompt for EXIT decisions (when in position) - SIMPLE BINARY CHOICE
    fn create_exit_prompt(&self, candles: &[Candle], position: &PositionContext) -> Result<String> {
        use crate::indicators::{calculate_rsi, calculate_adx};

        let current = candles.last().unwrap();
        let current_price = current.close;
        let entry_price = position.entry_price.unwrap_or(current_price);
        let pnl = position.pnl_percent.unwrap_or(0.0);
        let days = position.days_held.unwrap_or(0.0);

        // Calculate technical indicators
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let rsi = calculate_rsi(&closes, 14).unwrap_or(50.0);

        let adx_result = calculate_adx(candles, 14);
        let (adx, plus_di, minus_di) = adx_result.unwrap_or((0.0, 0.0, 0.0));

        // Recent price action
        let price_24h_ago = candles.get(candles.len().saturating_sub(24)).map(|c| c.close).unwrap_or(current_price);
        let change_24h = ((current_price - price_24h_ago) / price_24h_ago) * 100.0;

        // Determine trend description
        let trend_desc = if adx > 25.0 {
            if plus_di > minus_di {
                "Strong uptrend"
            } else {
                "Strong downtrend"
            }
        } else if adx > 15.0 {
            if plus_di > minus_di {
                "Weak uptrend"
            } else {
                "Weak downtrend"
            }
        } else {
            "No clear trend (choppy)"
        };

        let prompt = format!(
            r#"CURRENT POSITION STATUS:
- Entry Price: ${:.2}
- Current Price: ${:.2}
- P&L: {:.2}%
- Days Held: {:.1}
- 24H Change: {:.2}%
- RSI: {:.1}
- Trend: {} (ADX: {:.1})

**DECISION: Should you EXIT (Sell) or HOLD?**

This is a simple binary choice. Consider:

**EXIT if:**
- P&L ≤ -8% (stop loss - cut losses)
- P&L ≥ +15% (take profit - lock in gains)
- Held 7+ days with <5% gain (time stop - not working)
- Clear reversal (RSI >75 overbought OR strong downtrend forming)

**HOLD if:**
- Position is working (P&L improving, trend intact)
- No clear reason to exit
- Still within acceptable range

**Important:** Be decisive. Don't overthink this. It's a yes/no question.

**OUTPUT (JSON only):**
{{
  "action": "Sell|Hold",
  "confidence": 0.0-1.0,
  "reasoning": "Brief explanation"
}}

Only recommend Sell with 0.70+ confidence if there's a clear exit reason."#,
            entry_price,
            current_price,
            pnl,
            days,
            change_24h,
            rsi,
            trend_desc,
            adx
        );

        Ok(prompt)
    }
}
