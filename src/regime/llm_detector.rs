/// LLM-powered regime detection using OpenAI API
///
/// Uses GPT-4 to analyze market data and detect regime with higher accuracy
/// than quantitative indicators alone

use crate::models::Candle;
use crate::regime::MarketRegime;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";
const MODEL: &str = "gpt-4o-mini"; // GPT-4o-mini: 16x cheaper, still excellent quality
const MAX_TOKENS: u32 = 1024;
const RATE_LIMIT_DELAY_MS: u64 = 2500; // 2.5 seconds between calls to avoid rate limits
const MAX_RETRIES: u32 = 3; // Retry failed requests up to 3 times

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
pub struct LLMRegimeResponse {
    pub regime: String,
    pub confidence: f64,
    pub reasoning: String,
}

pub struct LLMRegimeDetector {
    api_key: String,
    client: reqwest::Client,
    cache: HashMap<String, (MarketRegime, f64)>, // Cache responses to avoid API costs
    disable_cache: bool, // Disable cache for backtesting
}

impl LLMRegimeDetector {
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

    /// Detect regime using Claude API
    ///
    /// Returns (MarketRegime, confidence_score) where confidence is 0.0-1.0
    pub async fn detect_regime_with_confidence(
        &mut self,
        candles: &[Candle],
    ) -> Result<(MarketRegime, f64)> {
        if candles.len() < 50 {
            return Ok((MarketRegime::ChoppyUnclear, 0.0));
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
                        content: "You are an expert cryptocurrency market analyst. Analyze market data and classify regimes accurately. Always respond with valid JSON only, no markdown formatting.".to_string(),
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
            let llm_response: LLMRegimeResponse = match serde_json::from_str(&text) {
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

            // Convert string regime to enum
            let regime = match llm_response.regime.as_str() {
                "BullTrend" => MarketRegime::BullTrend,
                "BearCrash" => MarketRegime::BearCrash,
                "ChoppyClear" => MarketRegime::ChoppyClear,
                _ => MarketRegime::ChoppyUnclear,
            };

            let confidence = llm_response.confidence.clamp(0.0, 1.0);

            // Cache result
            if !self.disable_cache {
                self.cache.insert(cache_key, (regime, confidence));
            }

            return Ok((regime, confidence));
        }
    }

    /// Create prompt for Claude with market data
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

        // Calculate 90-day context (critical for avoiding misclassification!)
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

        // TREND MATURITY ANALYSIS: Calculate how long current trend has been running
        // This helps the LLM identify if we're in early vs late stage of trend
        let weekly_prices: Vec<f64> = recent
            .iter()
            .rev()
            .step_by(168) // ~168 hours per week
            .take(12) // Last 12 weeks
            .map(|c| c.close)
            .collect();

        let trend_maturity_note = if weekly_prices.len() >= 3 {
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

            if consecutive_up >= 2 {
                format!(
                    "UPTREND ({})",
                    if consecutive_up <= 3 { "young, 2-3 weeks - EARLY stage, room to run" }
                    else if consecutive_up <= 6 { "mature, 4-6 weeks - MID stage, proceed with caution" }
                    else { "old, 7+ weeks - LATE stage, reversal risk high" }
                )
            } else if consecutive_down >= 2 {
                format!(
                    "DOWNTREND ({})",
                    if consecutive_down <= 3 { "young, 2-3 weeks - EARLY decline, may bounce soon" }
                    else if consecutive_down <= 6 { "mature, 4-6 weeks - MID decline, watch for capitulation" }
                    else { "old, 7+ weeks - LATE decline, potential bottom forming" }
                )
            } else {
                "SIDEWAYS/CHOPPY (no consistent multi-week trend - default to Choppy regime)".to_string()
            }
        } else {
            "Insufficient data for trend maturity".to_string()
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

        let prompt = format!(
            r#"You are an expert cryptocurrency market analyst. Analyze the following SOL market data and classify the current regime.

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
- **📈 TREND MATURITY**: {}
  - **CRITICAL**: Trend age affects regime classification!
  - **Young uptrends (2-3 weeks)**: BEST time for BullTrend classification
  - **Mature uptrends (4-6 weeks)**: Caution, may switch to Choppy soon
  - **Old uptrends (7+ weeks)**: AVOID BullTrend, likely reversal - use Choppy or BearCrash
  - **Young downtrends (2-3 weeks)**: May bounce - use Choppy unless panic
  - **Sideways/Choppy trend**: Default to Choppy regimes
- **Recent candles** (last 20 hours):
{}

## Regime Classification Guidelines

Classify the current market regime as one of:

1. **BullTrend**: Sustained upward momentum with healthy characteristics
   - Requirements: Price rising 24h+, strong +DI (> -DI), higher highs/lows, price > SMA
   - **⏰ TREND TIMING CHECK**: Uptrend must be ≤6 weeks old (check Trend Maturity!)
   - Volume: Steady or increasing on rallies
   - **Confidence**: High only when MULTIPLE bullish signals + young/mature trend
   - **AVOID** if trend is >7 weeks old (late stage, reversal risk)

2. **BearCrash**: Active panic selling with sharp recent decline
   - Requirements: Price dropped >10% in last 24-48h, ATR spike, RSI <25, strong -DI
   - Volume: High panic selling volume
   - **Trend context**: More likely in young downtrends or panic after long uptrends
   - NOTE: Normal corrections (<10%) are NOT crashes - use Choppy instead
   - Confidence: High only when seeing ACTIVE selling, not just low prices

3. **ChoppyClear**: Sideways consolidation with clear range boundaries
   - Characteristics: Stable range, low ADX (<20), RSI 30-70, clear support/resistance
   - Use this for: Normal corrections, consolidations, range-bound markets

4. **ChoppyUnclear**: Mixed or uncertain conditions (DEFAULT for ambiguity)
   - Use when: Conflicting signals, unclear direction, transitional periods
   - DEFAULT: When unsure between BullTrend and BearCrash, choose this

IMPORTANT:
- Normal volatility is NOT a crash - crypto often moves 5-10% without being a crash
- Recovery from lows can be bullish - don't assume every dip is a continuing crash
- When in doubt, prefer Choppy classifications over extreme (Bull/Crash) classifications
- Require STRONG evidence for BullTrend or BearCrash - be conservative

Respond ONLY with valid JSON (no markdown, no code blocks):

{{
  "regime": "BullTrend|BearCrash|ChoppyClear|ChoppyUnclear",
  "confidence": 0.85,
  "reasoning": "Brief explanation (1-2 sentences) of why this regime was chosen"
}}

Confidence should be 0.0-1.0:
- 0.9-1.0: Very strong signals (multiple indicators strongly aligned)
- 0.7-0.9: Clear signals (most indicators agree)
- 0.5-0.7: Moderate signals (some agreement)
- 0.3-0.5: Weak signals (conflicting indicators)
- 0.0-0.3: Very uncertain (unclear data)
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
