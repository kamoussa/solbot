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
    /// Returns (Strategy, confidence_score) where confidence is 0.0-1.0
    pub async fn select_strategy_with_confidence(
        &mut self,
        candles: &[Candle],
    ) -> Result<(Strategy, f64)> {
        if candles.len() < 50 {
            return Ok((Strategy::DCA, 0.0));
        }

        // Create cache key from last candle timestamp
        let cache_key = candles.last().unwrap().timestamp.to_rfc3339();

        // Check cache first (only if cache is enabled)
        if !self.disable_cache {
            if let Some(cached) = self.cache.get(&cache_key) {
                return Ok(*cached);
            }
        }

        // Prepare market data for LLM
        let prompt = self.create_prompt(candles)?;

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

            // Cache result
            if !self.disable_cache {
                self.cache.insert(cache_key, (strategy, confidence));
            }

            return Ok((strategy, confidence));
        }
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
            r#"You are an expert cryptocurrency trading strategist. Your PRIMARY GOAL is to maximize returns, not to generate trading signals.

## Market Summary (Last 50 Hours)
- **Current Price**: ${:.2}
- **1H Change**: {:+.2}%
- **24H Change**: {:+.2}%
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
- **📅 TREND AGE**: Current trend has been running for **{} days**
  - **Interpretation**:
    - **< 14 days**: YOUNG trend - BEST opportunity for momentum (early stage)
    - **14-28 days**: MATURE trend - Mid-stage, proceed with caution
    - **> 28 days**: OLD trend - High reversal risk, avoid momentum
- **📈 TREND MATURITY** (Weekly): {}
  - **CRITICAL**: Trend age determines momentum opportunity timing!
  - **Young trends (2-3 weeks)**: BEST time for momentum (early stage)
  - **Mature trends (4-6 weeks)**: Caution, mid-stage, could reverse
  - **Old trends (7+ weeks)**: AVOID momentum, reversal risk high
- **Recent candles** (last 20 hours):
{}

## Available Strategies - Choose Based on Market Conditions

### 1. DCA (Dollar Cost Averaging) - Default for 70-80% of Year
- **When it shines**: Choppy markets, sideways trends, unclear direction, mixed signals
- **Characteristics**: Steady accumulation, consistent buying, works in uncertainty
- **Best use**: When you DON'T see clear momentum OR panic opportunities
- **Entry threshold**: 0.65 confidence (easy to use as default)
- **Typical allocation**: 70-80% of the year (most of the time)
- **Why it works**: Avoids mistiming entries/exits, captures long-term growth

### 2. Momentum Strategy - Capture Early Bull Runs (10-15% of Year)
- **When it shines**: YOUNG, strong uptrends in early stages (2-3 weeks old)
- **Opportunity**: Can capture +20-30% moves during bull runs
- **Best timing**: Enter EARLY in trend formation (week 1-3), exit before maturity
- **⏰ CRITICAL - Trend Age Matters**:
  - **✅ YOUNG (2-3 weeks)**: BEST opportunity, early stage, room to run → confidence 0.70-0.85
  - **⚠️ MATURE (4-6 weeks)**: Mid-stage, higher reversal risk → confidence 0.60-0.70
  - **❌ OLD (7+ weeks)**: Late stage, reversal imminent → DO NOT USE, back to DCA
- **Requirements for entry (ALL must be met)**:
  - **Trend age ≤6 weeks** (check Trend Maturity section above!)
  - ADX > 30 (very strong trend, not weak 20-25)
  - Clear higher highs AND higher lows pattern (7+ days)
  - +DI > -DI by at least 10 points (bullish momentum)
  - RSI 40-65 (room to run, not overbought 70+)
  - Price > SMA(20) with expanding range
- **Entry threshold**: 0.70 confidence
- **Typical allocation**: 10-15% of the year (rare but high-impact opportunities)

### 3. Mean Reversion Strategy - Catch Panic Bounces (5-10% of Year)
- **When it shines**: EXTREME panic crashes with capitulation (RSI < 20)
- **Opportunity**: Can catch +5-10% bounces after fear peaks
- **Best timing**: Enter AFTER panic selling peaks, when fear is maximum
- **Requirements for entry (ALL must be met)**:
  - RSI < 20 (extreme oversold, not just 30-40)
  - Price dropped > 15% in last 48 hours (true panic, not just -5-10%)
  - Volume > 2.0x average (massive panic selling, not just 1.3-1.5x)
  - Price < 20th percentile of 90-day range (near lows)
  - Clear capitulation pattern (sellers exhausted)
- **Entry threshold**: 0.75 confidence
- **Typical allocation**: 5-10% of the year (very rare panic events)

## CRITICAL RULES FOR OPPORTUNITY IDENTIFICATION

**🎯 YOUR GOAL: MAXIMIZE RETURNS BY IDENTIFYING THE RIGHT OPPORTUNITIES**

1. **Most of the year is DCA territory (70-80%)**
   - Use DCA with 0.65+ confidence when market is choppy/mixed/unclear
   - DCA is the DEFAULT - you need to see clear opportunities to switch

2. **Look for HIGH-CONVICTION opportunities to switch**:
   - **Momentum**: Young uptrend (2-3 weeks) + ADX >30 + all conditions → confidence 0.70-0.85
   - **Mean Reversion**: Panic crash (RSI <20, -15%+ drop, 2x volume) → confidence 0.75+
   - If you don't see these clear setups → Stay in DCA (0.65+ confidence)

3. **Momentum is about TIMING (trend age is critical)**:
   - YOUNG trends (2-3 weeks): BEST opportunity for momentum (early stage)
   - MATURE trends (4-6 weeks): Risky, late-stage, prefer DCA
   - OLD trends (7+ weeks): Reversal imminent, DO NOT USE momentum

4. **Mean Reversion is about EXTREMES (not every dip)**:
   - Small dips (RSI 35-40, -5-8% drop): NOT extreme → DCA
   - True panic (RSI <20, -15%+ drop, high volume): Mean Reversion opportunity
   - Capitulation is key - enter when fear peaks, not on the way down

5. **Confidence Calibration**:
   - **DCA**: 0.65+ (easy default for uncertain/choppy markets)
   - **Momentum**: 0.70-0.85 (young trend + all conditions met)
   - **Mean Reversion**: 0.75+ (extreme panic + all conditions met)

## Decision Framework

**Step 1**: Check Trend Maturity (if there's a trend)
- Young uptrend (2-3 weeks) + ADX >30 + conditions met → **Consider Momentum** (0.70-0.85)
- Mature/Old uptrend (4+ weeks) → **Stay in DCA** (0.65+)
- No clear trend → **Use DCA** (0.65+)

**Step 2**: Check for Panic Crash
- RSI <20 + price -15%+ in 48h + 2x volume → **Consider Mean Reversion** (0.75+)
- Normal dip (RSI 30-40, modest drop) → **Use DCA** (0.65+)

**Step 3**: If neither clear opportunity → **Use DCA** (0.65+)
- Most of the year doesn't have clear momentum or panic setups
- DCA is the steady default for choppy/uncertain markets

**Step 4**: Adjust confidence based on conviction
- All conditions strongly met → Higher confidence (0.75-0.85)
- Some conditions partially met → Lower confidence (0.65-0.70)
- Conditions not met → DCA (0.65+)

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
            trend_age_days,
            trend_maturity_note,
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
}
