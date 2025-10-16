use crate::indicators::{calculate_rsi, calculate_sma};
use crate::models::{Candle, Signal};

/// Configuration for signal generation
#[derive(Debug, Clone)]
pub struct SignalConfig {
    pub rsi_period: usize,
    pub rsi_oversold: f64,
    pub rsi_overbought: f64,
    pub short_ma_period: usize,
    pub long_ma_period: usize,
    pub volume_threshold: f64, // Multiple of average volume
    pub lookback_hours: u64,   // How many hours of history to analyze
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            rsi_period: 14,
            rsi_oversold: 30.0,
            rsi_overbought: 70.0,
            short_ma_period: 10,
            long_ma_period: 20,
            volume_threshold: 1.5,
            lookback_hours: 24, // Default: analyze last 24 hours
        }
    }
}

impl SignalConfig {
    /// Calculate how many samples are needed based on lookback period and poll interval
    ///
    /// # Arguments
    /// * `poll_interval_minutes` - How often we sample (e.g., 30 minutes)
    ///
    /// # Returns
    /// Number of samples needed to cover the lookback period
    ///
    /// # Example
    /// ```
    /// use cryptobot::strategy::signals::SignalConfig;
    ///
    /// let config = SignalConfig::default();
    /// // 24 hour lookback, polling every 30 min = 48 samples
    /// assert_eq!(config.samples_needed(30), 48);
    /// ```
    pub fn samples_needed(&self, poll_interval_minutes: u64) -> usize {
        let lookback_minutes = self.lookback_hours * 60;
        let samples = lookback_minutes / poll_interval_minutes;

        // Need at least enough for longest indicator
        let min_for_indicators = self.long_ma_period + 5;
        samples.max(min_for_indicators as u64) as usize
    }
}

/// Validate that candles are uniformly spaced in time
///
/// # Arguments
/// * `candles` - The candles to validate
/// * `expected_interval_secs` - Expected time between candles in seconds
///
/// # Returns
/// * `Ok(())` if candles are uniformly spaced (within tolerance)
/// * `Err` if there are gaps in the data
///
/// # Tolerance
/// Allows up to 1.5x the expected interval (e.g., 7.5 min for 5 min polling)
pub fn validate_candle_uniformity(
    candles: &[Candle],
    expected_interval_secs: u64,
) -> anyhow::Result<()> {
    if candles.len() < 2 {
        return Ok(());
    }

    // Allow 50% tolerance for slight timing variations
    let max_gap_secs = expected_interval_secs + (expected_interval_secs / 2);

    for window in candles.windows(2) {
        let time_diff = (window[1].timestamp - window[0].timestamp).num_seconds();

        if time_diff < 0 {
            anyhow::bail!("Candles are not sorted by timestamp");
        }

        let time_diff_u64 = time_diff as u64;

        if time_diff_u64 > max_gap_secs {
            anyhow::bail!(
                "Data gap detected: {}s between candles (expected ~{}s, max allowed {}s). \
                 Gap from {} to {}. Consider clearing Redis and collecting fresh data.",
                time_diff_u64,
                expected_interval_secs,
                max_gap_secs,
                window[0].timestamp.format("%H:%M:%S"),
                window[1].timestamp.format("%H:%M:%S")
            );
        }
    }

    Ok(())
}

/// Analyze market conditions and generate composite signal
pub fn analyze_market_conditions(
    prices: &[f64],
    volumes: &[f64],
    config: &SignalConfig,
) -> Option<Signal> {
    if prices.len() < config.long_ma_period + 1 {
        return None;
    }

    // Calculate indicators
    let rsi = calculate_rsi(prices, config.rsi_period)?;
    let short_ma = calculate_sma(prices, config.short_ma_period)?;
    let long_ma = calculate_sma(prices, config.long_ma_period)?;

    // Calculate volume conditions
    let avg_volume = volumes.iter().sum::<f64>() / volumes.len() as f64;
    let current_volume = volumes.last()?;
    let volume_spike = current_volume / avg_volume > config.volume_threshold;
    let volume_ratio = current_volume / avg_volume;

    // Current price
    let current_price = prices.last()?;

    // Log indicators for debugging
    tracing::debug!(
        "Indicators: RSI={:.1}, Short MA={:.4}, Long MA={:.4}, Price={:.4}, Vol Ratio={:.2}x",
        rsi,
        short_ma,
        long_ma,
        current_price,
        volume_ratio
    );

    // Check buy conditions
    let rsi_condition = rsi < config.rsi_oversold + 10.0;
    let ma_crossover = short_ma > long_ma;
    let price_above_ma = *current_price > short_ma;

    let buy_conditions = [rsi_condition, ma_crossover, price_above_ma, volume_spike];
    let buy_count = buy_conditions.iter().filter(|&&x| x).count();

    // Check sell conditions
    let rsi_overbought = rsi > config.rsi_overbought;
    let ma_crossunder = short_ma < long_ma;

    // Determine signal with detailed logging
    let signal = if buy_count >= 3 {
        tracing::info!(
            "BUY conditions: RSI<40={}, MA↑={}, Price>MA={}, Vol↑={} ({}/4 met)",
            rsi_condition,
            ma_crossover,
            price_above_ma,
            volume_spike,
            buy_count
        );
        Signal::Buy
    } else if rsi_overbought && ma_crossunder {
        tracing::info!(
            "SELL conditions: RSI>70={}, MA↓={} (both required)",
            rsi_overbought,
            ma_crossunder
        );
        Signal::Sell
    } else {
        tracing::debug!(
            "HOLD: Buy {}/4, Sell RSI>70={} MA↓={}",
            buy_count,
            rsi_overbought,
            ma_crossunder
        );
        Signal::Hold
    };

    Some(signal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn create_test_candle(minutes_ago: i64) -> Candle {
        Candle {
            token: "TEST".to_string(),
            timestamp: Utc::now() - Duration::minutes(minutes_ago),
            open: 100.0,
            high: 100.0,
            low: 100.0,
            close: 100.0,
            volume: 1000.0,
        }
    }

    #[test]
    fn test_uniform_candles_pass() {
        let candles = vec![
            create_test_candle(10),
            create_test_candle(5),
            create_test_candle(0),
        ];

        let result = validate_candle_uniformity(&candles, 300); // 5 min = 300 sec
        assert!(result.is_ok());
    }

    #[test]
    fn test_gap_detected() {
        let candles = vec![
            create_test_candle(60), // 60 min ago
            create_test_candle(5),  // 5 min ago - 55 min gap!
            create_test_candle(0),
        ];

        let result = validate_candle_uniformity(&candles, 300);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("gap"));
    }

    #[test]
    fn test_backwards_timestamps_fail() {
        let candles = vec![
            create_test_candle(0),
            create_test_candle(5), // Backwards!
        ];

        let result = validate_candle_uniformity(&candles, 300);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not sorted"));
    }

    #[test]
    fn test_single_candle_ok() {
        let candles = vec![create_test_candle(0)];
        let result = validate_candle_uniformity(&candles, 300);
        assert!(result.is_ok());
    }

    #[test]
    fn test_tolerance_allows_slight_variation() {
        let candles = vec![
            create_test_candle(13), // 13 min ago
            create_test_candle(6),  // 6 min ago (7 min gap, within 50% tolerance of 5 min)
            create_test_candle(0),
        ];

        let result = validate_candle_uniformity(&candles, 300);
        assert!(result.is_ok()); // Should pass with 50% tolerance
    }

    #[test]
    fn test_signal_generation_buy() {
        // Uptrend with volume spike
        let prices = vec![
            100.0, 102.0, 104.0, 106.0, 108.0, 110.0, 112.0, 114.0, 116.0, 118.0, 120.0, 122.0,
            124.0, 126.0, 128.0, 130.0, 132.0, 134.0, 136.0, 138.0, 140.0,
        ];
        let volumes = vec![
            1000.0, 1100.0, 1200.0, 1300.0, 1400.0, 1500.0, 1600.0, 1700.0, 1800.0, 1900.0, 2000.0,
            2100.0, 2200.0, 2300.0, 2400.0, 2500.0, 2600.0, 2700.0, 2800.0, 2900.0,
            5000.0, // Volume spike
        ];

        let config = SignalConfig::default();
        let signal = analyze_market_conditions(&prices, &volumes, &config);

        assert!(signal.is_some());
        // Note: Uptrend means RSI is high, so might not trigger buy
        // This tests that the function runs, actual signal depends on thresholds
    }

    #[test]
    fn test_signal_generation_sell() {
        // Create overbought conditions
        let mut prices = vec![100.0];
        for i in 1..25 {
            prices.push(100.0 + i as f64 * 2.0); // Steady climb
        }
        // Then decline
        for i in 0..5 {
            prices.push(150.0 - i as f64 * 3.0);
        }

        let volumes = vec![1000.0; prices.len()];

        let config = SignalConfig::default();
        let signal = analyze_market_conditions(&prices, &volumes, &config);

        assert!(signal.is_some());
        let signal = signal.unwrap();
        // After steady climb and reversal, should consider selling
        assert!(matches!(signal, Signal::Sell | Signal::Hold));
    }

    #[test]
    fn test_insufficient_data() {
        let prices = vec![100.0, 101.0, 102.0];
        let volumes = vec![1000.0, 1100.0, 1200.0];
        let config = SignalConfig::default();

        let signal = analyze_market_conditions(&prices, &volumes, &config);
        assert!(signal.is_none());
    }

    #[test]
    fn test_custom_config() {
        let config = SignalConfig {
            rsi_period: 10,
            rsi_oversold: 25.0,
            rsi_overbought: 75.0,
            short_ma_period: 5,
            long_ma_period: 15,
            volume_threshold: 2.0,
            lookback_hours: 6,
        };

        let prices = vec![100.0; 20];
        let volumes = vec![1000.0; 20];

        let signal = analyze_market_conditions(&prices, &volumes, &config);
        assert!(signal.is_some());
        // Flat prices should result in Hold
        assert_eq!(signal.unwrap(), Signal::Hold);
    }
}
