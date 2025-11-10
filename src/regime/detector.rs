/// Market Regime Detector using ADX + price action
///
/// Classifies markets into regimes to select appropriate trading strategies:
/// - Bull Trend: ADX > 25 + +DI > -DI → Use Momentum
/// - Bear Crash: ADX > 25 + -DI > +DI + sharp decline → Use Mean Reversion (buy dip)
/// - Choppy: ADX < 20 → Use DCA (avoid whipsaws)

use crate::indicators::calculate_adx;
use crate::models::Candle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarketRegime {
    BullTrend,       // Strong uptrend - use Momentum
    BearCrash,       // Panic selloff - use Mean Reversion (buy the dip)
    ChoppyClear,     // Range-bound with clear support/resistance - use Mean Reversion
    ChoppyUnclear,   // Whipsaw recovery with no clear pattern - use DCA (avoid timing risk)
}

pub struct RegimeDetector {
    adx_period: usize,
    adx_strong_trend_threshold: f64,
    adx_weak_trend_threshold: f64,
    crash_lookback: usize,
    crash_threshold_pct: f64,
    range_lookback: usize,
    range_threshold_pct: f64,
}

impl Default for RegimeDetector {
    fn default() -> Self {
        Self {
            adx_period: 14,
            adx_strong_trend_threshold: 22.0, // Lowered from 25 (actual bull/crash was 19-20)
            adx_weak_trend_threshold: 16.0,   // Below this = choppy (was 20)
            crash_lookback: 7,                 // Shorter lookback (was 10)
            crash_threshold_pct: -8.0,         // More sensitive (was -10%)
            range_lookback: 20,                // Look back 20 candles for range detection
            range_threshold_pct: 10.0,         // ±10% range = range-bound
        }
    }
}

impl RegimeDetector {
    pub fn new(
        adx_period: usize,
        adx_strong_trend_threshold: f64,
        adx_weak_trend_threshold: f64,
        crash_lookback: usize,
        crash_threshold_pct: f64,
        range_lookback: usize,
        range_threshold_pct: f64,
    ) -> Self {
        Self {
            adx_period,
            adx_strong_trend_threshold,
            adx_weak_trend_threshold,
            crash_lookback,
            crash_threshold_pct,
            range_lookback,
            range_threshold_pct,
        }
    }

    /// Detect current market regime based on recent candles
    ///
    /// Returns None if insufficient data for detection
    pub fn detect_regime(&self, candles: &[Candle]) -> Option<MarketRegime> {
        // Need enough data for ADX calculation and range detection
        if candles.len() < self.adx_period + 1 {
            return None;
        }

        // Calculate ADX, +DI, -DI
        let (adx, plus_di, minus_di) = calculate_adx(candles, self.adx_period)?;

        let current_price = candles.last()?.close;

        // 1. Check for CRASH: Downtrend + sharp recent decline
        // More sensitive: lower ADX threshold, shorter lookback, smaller decline
        if minus_di > plus_di + 5.0 {
            // Look for price decline
            let lookback_start = candles.len().saturating_sub(self.crash_lookback);
            let recent_candles = &candles[lookback_start..];

            if recent_candles.len() >= 2 {
                let start_price = recent_candles.first().unwrap().close;
                let price_change_pct = ((current_price - start_price) / start_price) * 100.0;

                // If sharp decline, it's a crash (use Mean Reversion)
                if price_change_pct < self.crash_threshold_pct {
                    return Some(MarketRegime::BearCrash);
                }
            }
        }

        // 2. Check for BULL TREND: Strong uptrend
        if adx > self.adx_strong_trend_threshold && plus_di > minus_di + 5.0 {
            return Some(MarketRegime::BullTrend);
        }

        // 3. Check for CHOPPY: Weak ADX = no clear trend
        if adx < self.adx_weak_trend_threshold {
            // Distinguish between ChoppyClear (range-bound) and ChoppyUnclear (whipsaw)
            return Some(self.detect_choppy_type(candles, current_price));
        }

        // 4. Moderate ADX (between weak and strong): Could be range or unclear
        // Check if it's ranging (ChoppyClear) or just weak trend (ChoppyUnclear)
        Some(self.detect_choppy_type(candles, current_price))
    }

    /// Distinguish between ChoppyClear (range-bound) and ChoppyUnclear (whipsaw)
    fn detect_choppy_type(&self, candles: &[Candle], current_price: f64) -> MarketRegime {
        // Look for range-bound behavior: price oscillating within ±X% band
        let lookback_start = candles.len().saturating_sub(self.range_lookback);
        let recent_candles = &candles[lookback_start..];

        if recent_candles.len() < 10 {
            return MarketRegime::ChoppyUnclear; // Not enough data
        }

        // Calculate range statistics
        let prices: Vec<f64> = recent_candles.iter().map(|c| c.close).collect();
        let max_price = prices.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        let min_price = prices.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        let avg_price = prices.iter().sum::<f64>() / prices.len() as f64;

        let range_pct = ((max_price - min_price) / avg_price) * 100.0;

        // Calculate how many times price touched the edges
        let upper_band = avg_price * (1.0 + self.range_threshold_pct / 200.0);
        let lower_band = avg_price * (1.0 - self.range_threshold_pct / 200.0);

        let mut upper_touches = 0;
        let mut lower_touches = 0;
        for price in &prices {
            if *price >= upper_band {
                upper_touches += 1;
            }
            if *price <= lower_band {
                lower_touches += 1;
            }
        }

        // ChoppyClear: Range-bound with clear support/resistance
        // - Range is moderate (not too volatile)
        // - Price has touched both upper and lower bands (oscillating)
        if range_pct >= 5.0
            && range_pct <= self.range_threshold_pct
            && upper_touches >= 2
            && lower_touches >= 2
        {
            return MarketRegime::ChoppyClear;
        }

        // ChoppyUnclear: Whipsaw with no clear pattern
        MarketRegime::ChoppyUnclear
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_candles(prices: &[(f64, f64, f64, f64)]) -> Vec<Candle> {
        prices
            .iter()
            .enumerate()
            .map(|(i, &(open, high, low, close))| Candle {
                token: "TEST".to_string(),
                timestamp: Utc::now() + chrono::Duration::hours(i as i64),
                open,
                high,
                low,
                close,
                volume: 1000.0,
            })
            .collect()
    }

    #[test]
    fn test_detect_bull_trend() {
        // Strong uptrend
        let prices = vec![
            (100.0, 102.0, 99.0, 101.0),
            (101.0, 105.0, 100.0, 104.0),
            (104.0, 108.0, 103.0, 107.0),
            (107.0, 112.0, 106.0, 110.0),
            (110.0, 115.0, 109.0, 113.0),
            (113.0, 118.0, 112.0, 116.0),
            (116.0, 121.0, 115.0, 119.0),
            (119.0, 124.0, 118.0, 122.0),
            (122.0, 127.0, 121.0, 125.0),
            (125.0, 130.0, 124.0, 128.0),
            (128.0, 133.0, 127.0, 131.0),
            (131.0, 136.0, 130.0, 134.0),
            (134.0, 139.0, 133.0, 137.0),
            (137.0, 142.0, 136.0, 140.0),
            (140.0, 145.0, 139.0, 143.0),
        ];

        let candles = create_test_candles(&prices);
        let detector = RegimeDetector::default();
        let regime = detector.detect_regime(&candles);

        assert_eq!(regime, Some(MarketRegime::BullTrend));
    }

    #[test]
    fn test_detect_choppy_market() {
        // Ranging/choppy market
        let prices = vec![
            (100.0, 102.0, 98.0, 100.0),
            (100.0, 103.0, 97.0, 99.0),
            (99.0, 102.0, 98.0, 101.0),
            (101.0, 103.0, 99.0, 100.0),
            (100.0, 102.0, 98.0, 99.0),
            (99.0, 103.0, 97.0, 101.0),
            (101.0, 103.0, 99.0, 100.0),
            (100.0, 102.0, 98.0, 99.0),
            (99.0, 103.0, 97.0, 100.0),
            (100.0, 102.0, 98.0, 101.0),
            (101.0, 103.0, 99.0, 100.0),
            (100.0, 102.0, 98.0, 99.0),
            (99.0, 103.0, 97.0, 101.0),
            (101.0, 103.0, 99.0, 100.0),
            (100.0, 102.0, 98.0, 99.0),
        ];

        let candles = create_test_candles(&prices);
        let detector = RegimeDetector::default();
        let regime = detector.detect_regime(&candles);

        assert_eq!(regime, Some(MarketRegime::ChoppyUnclear));
    }

    #[test]
    fn test_detect_bear_crash() {
        // Sharp downtrend (crash)
        let prices = vec![
            (200.0, 202.0, 198.0, 200.0),
            (200.0, 201.0, 195.0, 196.0),
            (196.0, 197.0, 190.0, 192.0),
            (192.0, 193.0, 186.0, 188.0),
            (188.0, 189.0, 182.0, 184.0),
            (184.0, 185.0, 178.0, 180.0),
            (180.0, 181.0, 174.0, 176.0),
            (176.0, 177.0, 170.0, 172.0),
            (172.0, 173.0, 166.0, 168.0),
            (168.0, 169.0, 162.0, 164.0),
            (164.0, 165.0, 158.0, 160.0),
            (160.0, 161.0, 154.0, 156.0),
            (156.0, 157.0, 150.0, 152.0),
            (152.0, 153.0, 146.0, 148.0),
            (148.0, 149.0, 142.0, 144.0),
        ];

        let candles = create_test_candles(&prices);
        let detector = RegimeDetector::default();
        let regime = detector.detect_regime(&candles);

        // Should detect bear crash (strong downtrend + sharp decline)
        assert_eq!(regime, Some(MarketRegime::BearCrash));
    }

    #[test]
    fn test_insufficient_data() {
        let prices = vec![
            (100.0, 102.0, 99.0, 101.0),
            (101.0, 105.0, 100.0, 104.0),
        ];

        let candles = create_test_candles(&prices);
        let detector = RegimeDetector::default();
        let regime = detector.detect_regime(&candles);

        assert_eq!(regime, None);
    }
}

/// Composite Regime Detector using multiple indicators
///
/// Combines ADX, ATR, Volume, Market Structure, RSI, and Moving Average
/// to provide more robust regime classification than ADX alone.
pub struct CompositeRegimeDetector {
    adx_period: usize,
    atr_period: usize,
    rsi_period: usize,
    ma_period: usize,
    lookback_period: usize,
}

impl Default for CompositeRegimeDetector {
    fn default() -> Self {
        Self {
            adx_period: 14,
            atr_period: 14,
            rsi_period: 14,
            ma_period: 20,
            lookback_period: 20,
        }
    }
}

impl CompositeRegimeDetector {
    pub fn new(
        adx_period: usize,
        atr_period: usize,
        rsi_period: usize,
        ma_period: usize,
        lookback_period: usize,
    ) -> Self {
        Self {
            adx_period,
            atr_period,
            rsi_period,
            ma_period,
            lookback_period,
        }
    }

    /// Detect regime using composite scoring system
    pub fn detect_regime(&self, candles: &[Candle]) -> Option<MarketRegime> {
        self.detect_regime_with_confidence(candles).map(|(regime, _score)| regime)
    }

    /// Detect regime and return confidence score
    ///
    /// Returns (MarketRegime, confidence_score) where:
    /// - BullTrend: score 0.0-6.0 (higher = more confident)
    /// - BearCrash: score 0.0-5.0 (higher = more confident)
    /// - ChoppyClear: score 0.0-5.0 (higher = more confident)
    /// - ChoppyUnclear: score 0.0-3.0 (low confidence in all regimes)
    pub fn detect_regime_with_confidence(&self, candles: &[Candle]) -> Option<(MarketRegime, f64)> {
        if candles.len() < self.adx_period + self.lookback_period {
            return None;
        }

        // Calculate all indicators
        let (adx, plus_di, minus_di) = crate::indicators::calculate_adx(candles, self.adx_period)?;
        let atr = crate::indicators::calculate_atr(candles, self.atr_period)?;

        // Extract close prices for RSI and SMA
        let prices: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let rsi = crate::indicators::calculate_rsi(&prices, self.rsi_period)?;
        let sma = crate::indicators::calculate_sma(&prices, self.ma_period)?;

        let current_price = candles.last()?.close;
        let market_structure = crate::indicators::analyze_market_structure(candles, self.lookback_period);

        // Score each regime
        let bull_score = self.score_bull_trend(adx, plus_di, minus_di, current_price, sma, &market_structure, candles)?;
        let crash_score = self.score_bear_crash(adx, plus_di, minus_di, atr, rsi, candles)?;
        let range_score = self.score_choppy_clear(adx, plus_di, minus_di, atr, rsi, candles)?;

        // Return highest scoring regime with conservative thresholds
        // Require strong evidence before committing to a specific regime
        let max_score = bull_score.max(crash_score).max(range_score);

        if max_score < 3.0 {
            // No regime has strong confidence → ChoppyUnclear
            return Some((MarketRegime::ChoppyUnclear, max_score));
        }

        // Require 3.5+ for Bull (be conservative about uptrends)
        // Require 3.0+ for Crash (more aggressive - important to catch)
        // Require 4.0+ for ChoppyClear (should be very rare)
        if bull_score == max_score && bull_score >= 3.5 {
            Some((MarketRegime::BullTrend, bull_score))
        } else if crash_score == max_score && crash_score >= 3.0 {
            Some((MarketRegime::BearCrash, crash_score))
        } else if range_score == max_score && range_score >= 4.0 {
            Some((MarketRegime::ChoppyClear, range_score))
        } else {
            // Default to uncertain if scores don't meet thresholds
            Some((MarketRegime::ChoppyUnclear, max_score))
        }
    }
    
    fn score_bull_trend(
        &self,
        adx: f64,
        plus_di: f64,
        minus_di: f64,
        current_price: f64,
        sma: f64,
        market_structure: &crate::indicators::MarketStructure,
        candles: &[Candle],
    ) -> Option<f64> {
        let mut score = 0.0;
        
        // ADX shows trend + bullish direction (+2 points)
        if adx > 20.0 && plus_di > minus_di + 5.0 {
            score += 2.0;
        }
        
        // Price above MA (+1 point)
        if current_price > sma {
            score += 1.0;
        }
        
        // Market structure is uptrend (+1 point)
        if *market_structure == crate::indicators::MarketStructure::HigherHighsHigherLows {
            score += 1.0;
        }
        
        // RSI rising (+1 point)
        if let Some(true) = crate::indicators::is_rsi_rising(candles, self.rsi_period, 5) {
            score += 1.0;
        }
        
        // Volume on up days (+1 point)
        if let Some((up_ratio, _)) = crate::indicators::calculate_volume_direction_ratio(candles, self.lookback_period) {
            if up_ratio > 0.6 {
                score += 1.0;
            }
        }
        
        Some(score)
    }
    
    fn score_bear_crash(
        &self,
        adx: f64,
        plus_di: f64,
        minus_di: f64,
        atr: f64,
        rsi: f64,
        candles: &[Candle],
    ) -> Option<f64> {
        let mut score = 0.0;
        
        // ATR spike indicates volatility explosion (+2 points)
        if crate::indicators::is_atr_spike(candles, self.atr_period, self.lookback_period, 2.0) {
            score += 2.0;
        }
        
        // Volume spike indicates panic (+1 point)
        if crate::indicators::is_volume_spike(candles, self.lookback_period, 1.5) {
            score += 1.0;
        }
        
        // Sharp price decline (+1 point)
        if candles.len() >= 7 {
            let start_price = candles[candles.len() - 7].close;
            let current_price = candles.last()?.close;
            let change_pct = ((current_price - start_price) / start_price) * 100.0;
            if change_pct < -8.0 {
                score += 1.0;
            }
        }
        
        // RSI oversold (+1 point)
        if rsi < 30.0 {
            score += 1.0;
        }
        
        // -DI > +DI (bearish direction) (+1 point)
        if minus_di > plus_di + 5.0 {
            score += 1.0;
        }
        
        Some(score)
    }
    
    fn score_choppy_clear(
        &self,
        adx: f64,
        plus_di: f64,
        minus_di: f64,
        atr: f64,
        rsi: f64,
        candles: &[Candle],
    ) -> Option<f64> {
        let mut score = 0.0;

        // EXCLUSION: Cannot be range-bound if market is trending AT ALL
        // Any directional bias = trending, not ranging
        if plus_di > minus_di + 5.0 || minus_di > plus_di + 5.0 {
            // Directional bias → cannot be a range
            return Some(0.0);
        }

        // EXCLUSION: Cannot be range if ADX shows any trend strength
        if adx > 18.0 {
            // If ADX > 18 there's some directional movement
            return Some(0.0);
        }

        // Very Low ADX (very weak trend) (+1 point)
        if adx < 15.0 {
            score += 1.0;
        }

        // Check if price is range-bound with STRICTER criteria (+2 points)
        if candles.len() >= self.lookback_period {
            let recent = &candles[candles.len() - self.lookback_period..];
            let prices: Vec<f64> = recent.iter().map(|c| c.close).collect();
            let max = prices.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
            let min = prices.iter().fold(f64::INFINITY, |a, &b| a.min(b));
            let avg = prices.iter().sum::<f64>() / prices.len() as f64;
            let range_pct = ((max - min) / avg) * 100.0;

            // Stricter range requirement: 5-12% (not 5-15%)
            if range_pct >= 5.0 && range_pct <= 12.0 {
                // Tighter bands: ±5% from average (not ±7.5%)
                let upper = avg * 1.05;
                let lower = avg * 0.95;
                let upper_touches = prices.iter().filter(|&&p| p >= upper).count();
                let lower_touches = prices.iter().filter(|&&p| p <= lower).count();

                // Require MORE touches: 3+ on each side (not 2+)
                if upper_touches >= 3 && lower_touches >= 3 {
                    // Verify range stability: check if first half and second half have similar ranges
                    let mid = prices.len() / 2;
                    let first_half = &prices[..mid];
                    let second_half = &prices[mid..];

                    let first_max = first_half.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
                    let first_min = first_half.iter().fold(f64::INFINITY, |a, &b| a.min(b));
                    let first_range = first_max - first_min;

                    let second_max = second_half.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
                    let second_min = second_half.iter().fold(f64::INFINITY, |a, &b| a.min(b));
                    let second_range = second_max - second_min;

                    // Range should be stable (not expanding or contracting rapidly)
                    let range_diff_pct = ((first_range - second_range).abs() / first_range.max(0.001)) * 100.0;

                    if range_diff_pct < 40.0 {
                        // Range is stable across time periods
                        score += 2.0;
                    }
                }
            }
        }

        // RSI oscillating (not extreme) (+1 point)
        if rsi >= 35.0 && rsi <= 65.0 {
            score += 1.0;
        }

        // Stable ATR (low volatility) (+1 point)
        let atr_series = crate::indicators::calculate_atr_series(candles, self.atr_period);
        if atr_series.len() >= 5 {
            let recent_atr = &atr_series[atr_series.len() - 5..];
            let avg_atr = recent_atr.iter().sum::<f64>() / recent_atr.len() as f64;
            let variance = recent_atr.iter().map(|a| (a - avg_atr).powi(2)).sum::<f64>() / recent_atr.len() as f64;
            let std_dev = variance.sqrt();

            // Low variance = stable volatility
            if std_dev / avg_atr < 0.2 {
                score += 1.0;
            }
        }

        Some(score)
    }
}
