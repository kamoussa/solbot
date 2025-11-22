/// Average True Range (ATR) indicator
///
/// Measures market volatility by calculating the average of true ranges over a period.
/// True Range is the greatest of:
/// - Current High - Current Low
/// - Abs(Current High - Previous Close)
/// - Abs(Current Low - Previous Close)
///
/// Uses Wilder's smoothing (same as RSI and ADX) for the moving average.

use crate::models::Candle;

/// Calculate ATR for the given candles
///
/// Returns the current ATR value, or None if insufficient data
pub fn calculate_atr(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period + 1 {
        return None;
    }

    // Calculate true ranges
    let mut true_ranges = Vec::new();
    for i in 1..candles.len() {
        let high = candles[i].high;
        let low = candles[i].low;
        let prev_close = candles[i - 1].close;

        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());

        true_ranges.push(tr);
    }

    if true_ranges.len() < period {
        return None;
    }

    // First ATR is simple average of first 'period' true ranges
    let first_atr: f64 = true_ranges.iter().take(period).sum::<f64>() / period as f64;

    // Apply Wilder's smoothing for subsequent values
    let mut atr = first_atr;
    for i in period..true_ranges.len() {
        atr = (atr * (period as f64 - 1.0) + true_ranges[i]) / period as f64;
    }

    Some(atr)
}

/// Calculate ATR and return all intermediate values (for analysis)
///
/// Returns vector of ATR values aligned with candles (starting from index period)
pub fn calculate_atr_series(candles: &[Candle], period: usize) -> Vec<f64> {
    if candles.len() < period + 1 {
        return Vec::new();
    }

    // Calculate true ranges
    let mut true_ranges = Vec::new();
    for i in 1..candles.len() {
        let high = candles[i].high;
        let low = candles[i].low;
        let prev_close = candles[i - 1].close;

        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());

        true_ranges.push(tr);
    }

    if true_ranges.len() < period {
        return Vec::new();
    }

    let mut atr_series = Vec::new();

    // First ATR is simple average of first 'period' true ranges
    let first_atr: f64 = true_ranges.iter().take(period).sum::<f64>() / period as f64;
    atr_series.push(first_atr);

    // Apply Wilder's smoothing for subsequent values
    let mut atr = first_atr;
    for i in period..true_ranges.len() {
        atr = (atr * (period as f64 - 1.0) + true_ranges[i]) / period as f64;
        atr_series.push(atr);
    }

    atr_series
}

/// Check if ATR has spiked above threshold (indicates volatility explosion)
///
/// Returns true if current ATR > threshold * recent average ATR
pub fn is_atr_spike(candles: &[Candle], period: usize, lookback: usize, threshold: f64) -> bool {
    if candles.len() < period + lookback + 1 {
        return false;
    }

    let atr_series = calculate_atr_series(candles, period);
    if atr_series.len() < lookback + 1 {
        return false;
    }

    let current_atr = atr_series[atr_series.len() - 1];

    // Calculate average ATR over lookback period (excluding current)
    let lookback_start = atr_series.len().saturating_sub(lookback + 1);
    let lookback_end = atr_series.len() - 1;

    let avg_atr = atr_series[lookback_start..lookback_end].iter().sum::<f64>()
        / (lookback_end - lookback_start) as f64;

    current_atr > threshold * avg_atr
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
    fn test_calculate_atr() {
        // Low volatility market
        let low_vol_prices = vec![
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
        ];

        let candles = create_test_candles(&low_vol_prices);
        let atr = calculate_atr(&candles, 14);

        assert!(atr.is_some());
        // ATR should be around 2.0 (high-low range)
        assert!(atr.unwrap() > 1.5 && atr.unwrap() < 2.5);
    }

    #[test]
    fn test_calculate_atr_high_volatility() {
        // High volatility market with gaps
        let high_vol_prices = vec![
            (100.0, 105.0, 95.0, 102.0),
            (102.0, 110.0, 98.0, 105.0),
            (105.0, 108.0, 92.0, 95.0),
            (95.0, 103.0, 88.0, 100.0),
            (100.0, 115.0, 97.0, 110.0),
            (110.0, 112.0, 95.0, 98.0),
            (98.0, 108.0, 90.0, 105.0),
            (105.0, 120.0, 100.0, 115.0),
            (115.0, 118.0, 105.0, 110.0),
            (110.0, 125.0, 108.0, 120.0),
            (120.0, 130.0, 115.0, 125.0),
            (125.0, 128.0, 110.0, 115.0),
            (115.0, 122.0, 105.0, 118.0),
            (118.0, 130.0, 115.0, 125.0),
            (125.0, 135.0, 120.0, 130.0),
        ];

        let candles = create_test_candles(&high_vol_prices);
        let atr = calculate_atr(&candles, 14);

        assert!(atr.is_some());
        // ATR should be higher for volatile market
        assert!(atr.unwrap() > 10.0);
    }

    #[test]
    fn test_atr_spike_detection() {
        // Normal volatility followed by spike
        let mut prices = vec![];

        // 20 candles with low volatility
        for _ in 0..20 {
            prices.push((100.0, 101.0, 99.0, 100.0));
        }

        // 5 candles with high volatility (spike)
        for _ in 0..5 {
            prices.push((100.0, 110.0, 90.0, 105.0));
        }

        let candles = create_test_candles(&prices);

        // Should detect spike (current ATR > 2x average)
        assert!(is_atr_spike(&candles, 14, 10, 2.0));
    }

    #[test]
    fn test_insufficient_data() {
        let prices = vec![
            (100.0, 101.0, 99.0, 100.0),
            (100.0, 101.0, 99.0, 100.0),
        ];

        let candles = create_test_candles(&prices);
        let atr = calculate_atr(&candles, 14);

        assert!(atr.is_none());
    }

    #[test]
    fn test_atr_series() {
        let prices = vec![
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
            (100.0, 105.0, 95.0, 100.0),
        ];

        let candles = create_test_candles(&prices);
        let atr_series = calculate_atr_series(&candles, 14);

        // Should have 1 ATR value (15 candles - 14 period = 1)
        assert_eq!(atr_series.len(), 1);
    }
}
