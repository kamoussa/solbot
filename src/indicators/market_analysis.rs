/// Market structure and volume analysis
///
/// Provides functions to analyze price structure (higher highs/lows) and volume patterns

use crate::models::Candle;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarketStructure {
    HigherHighsHigherLows,  // Uptrend
    LowerHighsLowerLows,    // Downtrend
    Mixed,                  // No clear structure
}

/// Analyze market structure over a lookback period
///
/// Returns the predominant market structure:
/// - HigherHighsHigherLows: Uptrend (most swings are higher highs + higher lows)
/// - LowerHighsLowerLows: Downtrend (most swings are lower highs + lower lows)
/// - Mixed: No clear structure
pub fn analyze_market_structure(candles: &[Candle], lookback: usize) -> MarketStructure {
    if candles.len() < lookback || lookback < 4 {
        return MarketStructure::Mixed;
    }

    let start_idx = candles.len().saturating_sub(lookback);
    let recent_candles = &candles[start_idx..];

    // Find swing highs and lows (local peaks and troughs)
    let mut swing_highs = Vec::new();
    let mut swing_lows = Vec::new();

    for i in 1..recent_candles.len() - 1 {
        let prev = recent_candles[i - 1].close;
        let curr = recent_candles[i].close;
        let next = recent_candles[i + 1].close;

        // Swing high: higher than both neighbors
        if curr > prev && curr > next {
            swing_highs.push((i, curr));
        }

        // Swing low: lower than both neighbors
        if curr < prev && curr < next {
            swing_lows.push((i, curr));
        }
    }

    // Need at least 2 swing highs and 2 swing lows to determine structure
    if swing_highs.len() < 2 || swing_lows.len() < 2 {
        return MarketStructure::Mixed;
    }

    // Count higher highs vs lower highs
    let mut higher_highs = 0;
    let mut lower_highs = 0;
    for i in 1..swing_highs.len() {
        if swing_highs[i].1 > swing_highs[i - 1].1 {
            higher_highs += 1;
        } else {
            lower_highs += 1;
        }
    }

    // Count higher lows vs lower lows
    let mut higher_lows = 0;
    let mut lower_lows = 0;
    for i in 1..swing_lows.len() {
        if swing_lows[i].1 > swing_lows[i - 1].1 {
            higher_lows += 1;
        } else {
            lower_lows += 1;
        }
    }

    // Determine structure
    let uptrend_signals = higher_highs + higher_lows;
    let downtrend_signals = lower_highs + lower_lows;

    if uptrend_signals > downtrend_signals && uptrend_signals >= 3 {
        MarketStructure::HigherHighsHigherLows
    } else if downtrend_signals > uptrend_signals && downtrend_signals >= 3 {
        MarketStructure::LowerHighsLowerLows
    } else {
        MarketStructure::Mixed
    }
}

/// Calculate average volume over a period
pub fn calculate_average_volume(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period {
        return None;
    }

    let start_idx = candles.len().saturating_sub(period);
    let recent_candles = &candles[start_idx..];

    let total_volume: f64 = recent_candles.iter().map(|c| c.volume).sum();
    Some(total_volume / period as f64)
}

/// Check if volume has spiked above threshold
///
/// Returns true if current volume > threshold * recent average volume
pub fn is_volume_spike(candles: &[Candle], lookback: usize, threshold: f64) -> bool {
    if candles.len() < lookback + 1 {
        return false;
    }

    let current_volume = candles[candles.len() - 1].volume;

    // Calculate average volume over lookback period (excluding current)
    let lookback_candles = &candles[candles.len() - lookback - 1..candles.len() - 1];
    let avg_volume: f64 = lookback_candles.iter().map(|c| c.volume).sum::<f64>()
        / lookback_candles.len() as f64;

    current_volume > threshold * avg_volume
}

/// Calculate the ratio of up-volume to down-volume
///
/// Returns (up_volume_ratio, down_volume_ratio) over lookback period
/// Ratios sum to 1.0
///
/// Used to detect accumulation (high up_volume) vs distribution (high down_volume)
pub fn calculate_volume_direction_ratio(candles: &[Candle], lookback: usize) -> Option<(f64, f64)> {
    if candles.len() < lookback + 1 {
        return None;
    }

    let start_idx = candles.len().saturating_sub(lookback);
    let recent_candles = &candles[start_idx..];

    let mut up_volume = 0.0;
    let mut down_volume = 0.0;

    for i in 1..recent_candles.len() {
        let price_change = recent_candles[i].close - recent_candles[i - 1].close;
        let volume = recent_candles[i].volume;

        if price_change > 0.0 {
            up_volume += volume;
        } else if price_change < 0.0 {
            down_volume += volume;
        }
        // Neutral days (no price change) don't count
    }

    let total_volume = up_volume + down_volume;
    if total_volume == 0.0 {
        return Some((0.5, 0.5));
    }

    Some((up_volume / total_volume, down_volume / total_volume))
}

/// Check if RSI is trending (rising or falling)
///
/// Returns:
/// - Some(true) if RSI is rising over lookback period
/// - Some(false) if RSI is falling
/// - None if insufficient data or unclear trend
pub fn is_rsi_rising(candles: &[Candle], rsi_period: usize, lookback: usize) -> Option<bool> {
    use crate::indicators::calculate_rsi;

    if candles.len() < rsi_period + lookback {
        return None;
    }

    // Calculate RSI for each of the last 'lookback' periods
    let mut rsi_values = Vec::new();
    for i in 0..lookback {
        let end_idx = candles.len() - lookback + i + 1;
        let window = &candles[..end_idx];

        // Extract prices for RSI calculation
        let prices: Vec<f64> = window.iter().map(|c| c.close).collect();
        if let Some(rsi) = calculate_rsi(&prices, rsi_period) {
            rsi_values.push(rsi);
        }
    }

    if rsi_values.len() < 2 {
        return None;
    }

    // Simple linear regression to determine trend
    let first = rsi_values[0];
    let last = rsi_values[rsi_values.len() - 1];
    let change = last - first;

    // Require at least 5 point change to consider it a trend
    if change.abs() < 5.0 {
        return None;
    }

    Some(change > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_candles(prices: &[(f64, f64, f64, f64, f64)]) -> Vec<Candle> {
        prices
            .iter()
            .enumerate()
            .map(|(i, &(open, high, low, close, volume))| Candle {
                token: "TEST".to_string(),
                timestamp: Utc::now() + chrono::Duration::hours(i as i64),
                open,
                high,
                low,
                close,
                volume,
            })
            .collect()
    }

    #[test]
    fn test_uptrend_structure() {
        // Clear uptrend with higher highs and higher lows
        let prices = vec![
            (100.0, 102.0, 99.0, 101.0, 1000.0),
            (101.0, 103.0, 100.0, 102.0, 1000.0),
            (102.0, 100.0, 97.0, 99.0, 1000.0),   // Swing low 1
            (99.0, 105.0, 98.0, 104.0, 1000.0),   // Swing high 1
            (104.0, 106.0, 101.0, 103.0, 1000.0),
            (103.0, 104.0, 100.0, 102.0, 1000.0), // Swing low 2 (higher)
            (102.0, 110.0, 101.0, 108.0, 1000.0), // Swing high 2 (higher)
            (108.0, 111.0, 105.0, 107.0, 1000.0),
            (107.0, 108.0, 103.0, 105.0, 1000.0), // Swing low 3 (higher)
            (105.0, 115.0, 104.0, 112.0, 1000.0), // Swing high 3 (higher)
        ];

        let candles = create_test_candles(&prices);
        let structure = analyze_market_structure(&candles, 10);

        assert_eq!(structure, MarketStructure::HigherHighsHigherLows);
    }

    #[test]
    fn test_downtrend_structure() {
        // Clear downtrend with lower highs and lower lows
        let prices = vec![
            (200.0, 202.0, 199.0, 200.0, 1000.0),
            (200.0, 203.0, 198.0, 199.0, 1000.0),
            (199.0, 202.0, 196.0, 197.0, 1000.0), // Swing high 1
            (197.0, 198.0, 192.0, 193.0, 1000.0), // Swing low 1
            (193.0, 196.0, 190.0, 195.0, 1000.0), // Swing high 2 (lower)
            (195.0, 196.0, 188.0, 189.0, 1000.0), // Swing low 2 (lower)
            (189.0, 192.0, 186.0, 191.0, 1000.0), // Swing high 3 (lower)
            (191.0, 192.0, 182.0, 183.0, 1000.0), // Swing low 3 (lower)
            (183.0, 186.0, 180.0, 185.0, 1000.0),
            (185.0, 186.0, 178.0, 179.0, 1000.0),
        ];

        let candles = create_test_candles(&prices);
        let structure = analyze_market_structure(&candles, 10);

        assert_eq!(structure, MarketStructure::LowerHighsLowerLows);
    }

    #[test]
    fn test_mixed_structure() {
        // Choppy market with no clear structure
        let prices = vec![
            (100.0, 102.0, 98.0, 100.0, 1000.0),
            (100.0, 103.0, 97.0, 99.0, 1000.0),
            (99.0, 102.0, 98.0, 101.0, 1000.0),
            (101.0, 103.0, 99.0, 100.0, 1000.0),
            (100.0, 102.0, 98.0, 99.0, 1000.0),
            (99.0, 103.0, 97.0, 101.0, 1000.0),
            (101.0, 103.0, 99.0, 100.0, 1000.0),
            (100.0, 102.0, 98.0, 99.0, 1000.0),
        ];

        let candles = create_test_candles(&prices);
        let structure = analyze_market_structure(&candles, 8);

        assert_eq!(structure, MarketStructure::Mixed);
    }

    #[test]
    fn test_volume_spike() {
        // Normal volume followed by spike
        let mut prices = vec![];
        for _ in 0..10 {
            prices.push((100.0, 101.0, 99.0, 100.0, 1000.0));
        }
        // Volume spike
        prices.push((100.0, 105.0, 99.0, 104.0, 2500.0));

        let candles = create_test_candles(&prices);

        // Should detect spike (2.5x average)
        assert!(is_volume_spike(&candles, 10, 2.0));
        assert!(!is_volume_spike(&candles, 10, 3.0));
    }

    #[test]
    fn test_volume_direction_ratio() {
        // More volume on up days (accumulation)
        let prices = vec![
            (100.0, 102.0, 98.0, 99.0, 500.0),  // Down, 500 volume
            (99.0, 105.0, 98.0, 103.0, 2000.0), // Up, 2000 volume
            (103.0, 104.0, 101.0, 102.0, 300.0), // Down, 300 volume
            (102.0, 108.0, 101.0, 107.0, 1800.0), // Up, 1800 volume
            (107.0, 109.0, 105.0, 106.0, 200.0), // Down, 200 volume
        ];

        let candles = create_test_candles(&prices);
        let (up_ratio, down_ratio) = calculate_volume_direction_ratio(&candles, 5).unwrap();

        // Up volume = 3800, Down volume = 1000, Total = 4800
        // Up ratio ~= 0.79, Down ratio ~= 0.21
        assert!(up_ratio > 0.75);
        assert!(down_ratio < 0.25);
        assert!((up_ratio + down_ratio - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_average_volume() {
        let prices = vec![
            (100.0, 101.0, 99.0, 100.0, 1000.0),
            (100.0, 101.0, 99.0, 100.0, 2000.0),
            (100.0, 101.0, 99.0, 100.0, 3000.0),
        ];

        let candles = create_test_candles(&prices);
        let avg_vol = calculate_average_volume(&candles, 3).unwrap();

        assert_eq!(avg_vol, 2000.0);
    }
}
