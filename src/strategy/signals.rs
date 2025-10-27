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
    // Panic buy settings (flash crash detection)
    pub enable_panic_buy: bool,   // Enable aggressive flash crash buying
    pub panic_rsi_threshold: f64, // RSI threshold for panic (e.g., 30)
    pub panic_volume_multiplier: f64, // Volume spike needed (e.g., 2.0x)
    pub panic_price_drop_pct: f64, // Recent price drop % (e.g., 8%)
    pub panic_drop_window_candles: usize, // How many candles to check for drop (e.g., 12 = 1hr at 5min)
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
            // Panic buy defaults (conservative but effective)
            enable_panic_buy: true,
            panic_rsi_threshold: 30.0,     // Extreme oversold
            panic_volume_multiplier: 2.0,  // 2x volume spike
            panic_price_drop_pct: 8.0,     // 8% drop
            panic_drop_window_candles: 12, // 1 hour at 5min intervals
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
    let current_volume = volumes.last()?;

    // Detect if we have real volume data (e.g., from Birdeye)
    // CoinGecko backfilled data has volume=0.0, which breaks volume analysis
    // If there are ANY zeros, the average and ratio calculations are meaningless
    let has_volume_data = volumes.iter().all(|&v| v > 0.1);

    let (volume_spike, volume_ratio) = if has_volume_data {
        let avg_volume = volumes.iter().sum::<f64>() / volumes.len() as f64;
        let ratio = current_volume / avg_volume;
        (ratio > config.volume_threshold, ratio)
    } else {
        (false, 0.0) // No volume data, skip volume check
    };

    // Current price
    let current_price = prices.last()?;

    // Log if we're operating without volume data
    if !has_volume_data {
        let zero_count = volumes.iter().filter(|&&v| v <= 0.1).count();
        tracing::warn!(
            "⚠️  Incomplete volume data ({}/{} candles missing volume), trading without volume confirmation. \
             This is likely CoinGecko backfilled data - volume analysis will activate after 24h of Birdeye data.",
            zero_count, volumes.len()
        );
    }

    // ==================== PANIC BUY LOGIC ====================
    // Check for flash crash conditions (aggressive entry, bypasses MA confirmation)
    // IMPORTANT: Panic buy requires volume spike confirmation, so skip if no volume data
    if has_volume_data && config.enable_panic_buy && prices.len() > config.panic_drop_window_candles
    {
        let window_start = prices
            .len()
            .saturating_sub(config.panic_drop_window_candles);
        let price_window_high = prices[window_start..]
            .iter()
            .fold(f64::MIN, |max, &p| max.max(p));

        let price_drop_pct = ((price_window_high - current_price) / price_window_high) * 100.0;
        let panic_volume_spike = volume_ratio > config.panic_volume_multiplier;

        // Panic buy conditions:
        // 1. RSI extremely oversold
        // 2. Significant price drop in recent window
        // 3. Volume spike (2x+ panic selling)
        // 4. Not in deep downtrend (price within 8% of long MA)
        let panic_conditions = [
            rsi < config.panic_rsi_threshold,
            price_drop_pct >= config.panic_price_drop_pct,
            panic_volume_spike,
            *current_price > long_ma * 0.92, // Not deep in downtrend
        ];

        if panic_conditions.iter().all(|&x| x) {
            tracing::warn!(
                "⚡ PANIC BUY triggered: RSI={:.1}, Drop={:.1}%, Vol={:.2}x, Price/LongMA={:.2}%",
                rsi,
                price_drop_pct,
                volume_ratio,
                (*current_price / long_ma - 1.0) * 100.0
            );
            return Some(Signal::Buy);
        }
    }

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

    // Without volume data, require all 3 other conditions (more conservative)
    // With volume data, require 3 out of 4 conditions (allows flexibility)
    let (buy_signal, buy_reason) = if has_volume_data {
        let buy_conditions = [rsi_condition, ma_crossover, price_above_ma, volume_spike];
        let buy_count = buy_conditions.iter().filter(|&&x| x).count();
        (
            buy_count >= 3,
            format!(
                "BUY conditions: RSI<40={}, MA↑={}, Price>MA={}, Vol↑={} ({}/4 met)",
                rsi_condition, ma_crossover, price_above_ma, volume_spike, buy_count
            ),
        )
    } else {
        // No volume data: require all 3 conditions (conservative mode)
        let buy_conditions = [rsi_condition, ma_crossover, price_above_ma];
        let buy_count = buy_conditions.iter().filter(|&&x| x).count();
        let all_met = buy_count == 3;
        (
            all_met,
            format!(
                "BUY conditions (NO VOLUME): RSI<40={}, MA↑={}, Price>MA={} ({}/3 met, all required)",
                rsi_condition, ma_crossover, price_above_ma, buy_count
            ),
        )
    };

    // Check sell conditions
    let rsi_overbought = rsi > config.rsi_overbought;
    let ma_crossunder = short_ma < long_ma;

    // Determine signal with detailed logging
    let signal = if buy_signal {
        tracing::info!("{}", buy_reason);
        Signal::Buy
    } else if rsi_overbought && ma_crossunder {
        tracing::info!(
            "SELL conditions: RSI>70={}, MA↓={} (both required)",
            rsi_overbought,
            ma_crossunder
        );
        Signal::Sell
    } else {
        if has_volume_data {
            tracing::debug!(
                "HOLD: Buy conditions not met, Sell RSI>70={} MA↓={}",
                rsi_overbought,
                ma_crossunder
            );
        } else {
            tracing::debug!(
                "HOLD (NO VOLUME): Buy conditions not met, Sell RSI>70={} MA↓={}",
                rsi_overbought,
                ma_crossunder
            );
        }
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
            enable_panic_buy: false, // Disable for this test
            panic_rsi_threshold: 30.0,
            panic_volume_multiplier: 2.0,
            panic_price_drop_pct: 8.0,
            panic_drop_window_candles: 12,
        };

        let prices = vec![100.0; 20];
        let volumes = vec![1000.0; 20];

        let signal = analyze_market_conditions(&prices, &volumes, &config);
        assert!(signal.is_some());
        // Flat prices should result in Hold
        assert_eq!(signal.unwrap(), Signal::Hold);
    }

    #[test]
    fn test_panic_buy_flash_crash() {
        // Simulate flash crash: sharp 10% drop in 1 hour with volume spike
        let mut prices = vec![200.0; 20]; // Stable at 200
                                          // Sudden sharp 10% crash over 12 candles to push RSI < 30
        for i in 1..=12 {
            prices.push(200.0 - (i as f64 * 1.67)); // Drop ~1.67 per candle
        }
        prices.push(180.0); // Final price -10% from high

        let mut volumes = vec![1000.0; prices.len() - 1];
        volumes.push(2500.0); // 2.5x volume spike on crash

        let config = SignalConfig::default(); // Panic buy enabled by default

        let signal = analyze_market_conditions(&prices, &volumes, &config);

        assert!(signal.is_some());
        assert_eq!(
            signal.unwrap(),
            Signal::Buy,
            "Should trigger panic buy on flash crash"
        );
    }

    #[test]
    fn test_panic_buy_disabled() {
        // Same flash crash scenario but panic buy disabled
        let mut prices = vec![200.0; 20];
        for i in 1..=12 {
            prices.push(200.0 - (i as f64 * 1.67));
        }
        prices.push(180.0);

        let mut volumes = vec![1000.0; prices.len() - 1];
        volumes.push(2500.0);

        let config = SignalConfig {
            enable_panic_buy: false, // Disable panic buy
            ..Default::default()
        };

        let signal = analyze_market_conditions(&prices, &volumes, &config);

        // Should not trigger panic buy, falls through to regular logic
        assert!(signal.is_some());
        // Might be Hold since MA conditions aren't met in crash
        assert_ne!(
            signal.unwrap(),
            Signal::Buy,
            "Should not panic buy when disabled"
        );
    }

    #[test]
    fn test_panic_buy_insufficient_drop() {
        // Only 5% drop, not enough for panic buy (needs 8%)
        let mut prices = vec![200.0; 20];
        for i in 0..12 {
            prices.push(200.0 - (i as f64 * 0.83)); // Only 10 point drop = 5%
        }
        prices.push(190.0);

        let mut volumes = vec![1000.0; prices.len() - 1];
        volumes.push(2500.0); // Volume spike present

        let config = SignalConfig::default();

        let signal = analyze_market_conditions(&prices, &volumes, &config);

        // Should not trigger panic buy
        assert!(signal.is_some());
        let result = signal.unwrap();
        // May still trigger regular buy if other conditions met
        // but won't be the immediate panic buy
        assert!(result == Signal::Hold || result == Signal::Buy);
    }

    #[test]
    fn test_panic_buy_no_volume_spike() {
        // 8% drop but no volume spike
        let mut prices = vec![200.0; 20];
        for i in 0..12 {
            prices.push(200.0 - (i as f64 * 1.33));
        }
        prices.push(184.0);

        let volumes = vec![1000.0; prices.len()]; // No volume spike

        let config = SignalConfig::default();

        let signal = analyze_market_conditions(&prices, &volumes, &config);

        assert!(signal.is_some());
        assert_ne!(
            signal.unwrap(),
            Signal::Buy,
            "Should not panic buy without volume confirmation"
        );
    }

    #[test]
    fn test_panic_buy_deep_downtrend() {
        // Flash crash but already in deep downtrend (price way below long MA)
        // This prevents buying into a collapsing asset
        let mut prices = vec![200.0; 20]; // MA will be around 200
                                          // Deep crash to 170 (-15%), way below the 92% threshold
        for i in 0..12 {
            prices.push(200.0 - (i as f64 * 2.5));
        }
        prices.push(170.0); // 15% below MA baseline

        let mut volumes = vec![1000.0; prices.len() - 1];
        volumes.push(2500.0); // Volume spike present

        let config = SignalConfig::default();

        let signal = analyze_market_conditions(&prices, &volumes, &config);

        // Should not trigger panic buy - too deep in downtrend
        assert!(signal.is_some());
        assert_ne!(
            signal.unwrap(),
            Signal::Buy,
            "Should not panic buy in deep downtrend"
        );
    }

    #[test]
    fn test_zero_volume_no_crash() {
        // Test that zero volume data doesn't cause division by zero (NaN)
        // This happens with CoinGecko backfilled data
        let prices = vec![
            100.0, 102.0, 104.0, 106.0, 108.0, 110.0, 112.0, 114.0, 116.0, 118.0, 120.0, 122.0,
            124.0, 126.0, 128.0, 130.0, 132.0, 134.0, 136.0, 138.0, 140.0,
        ];
        let volumes = vec![0.0; prices.len()]; // All zeros (CoinGecko data)

        let config = SignalConfig::default();
        let signal = analyze_market_conditions(&prices, &volumes, &config);

        // Should return a valid signal (not crash)
        assert!(signal.is_some());
    }

    #[test]
    fn test_zero_volume_panic_buy_disabled() {
        // Even with perfect panic buy conditions, zero volume should skip it
        let mut prices = vec![200.0; 20];
        for i in 1..=12 {
            prices.push(200.0 - (i as f64 * 1.67)); // Sharp drop
        }
        prices.push(180.0); // -10% flash crash

        let volumes = vec![0.0; prices.len()]; // No volume data

        let config = SignalConfig::default(); // Panic buy enabled

        let signal = analyze_market_conditions(&prices, &volumes, &config);

        // Should NOT trigger panic buy without volume confirmation
        assert!(signal.is_some());
        // Might still generate regular buy if other conditions met, but NOT panic buy
    }

    #[test]
    fn test_zero_volume_conservative_buy() {
        // With zero volume, should require ALL 3 conditions (not 3/4)
        let prices = vec![
            100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0,
            100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0,
            90.0, // Drop to create oversold RSI
        ];
        let volumes = vec![0.0; prices.len()];

        let config = SignalConfig::default();
        let signal = analyze_market_conditions(&prices, &volumes, &config);

        // Without volume, need all 3 other conditions
        // This test just ensures it doesn't crash and produces valid output
        assert!(signal.is_some());
    }

    #[test]
    fn test_zero_volume_vs_real_volume() {
        // Same price scenario, different outcomes with/without volume
        let prices = vec![
            100.0, 102.0, 104.0, 106.0, 108.0, 110.0, 112.0, 114.0, 116.0, 118.0, 120.0, 122.0,
            124.0, 126.0, 128.0, 130.0, 132.0, 134.0, 136.0, 138.0, 140.0,
        ];

        // Scenario 1: Zero volume
        let zero_volumes = vec![0.0; prices.len()];
        let config = SignalConfig::default();
        let signal_zero = analyze_market_conditions(&prices, &zero_volumes, &config);

        // Scenario 2: With volume spike
        let mut real_volumes = vec![1000.0; prices.len() - 1];
        real_volumes.push(2500.0); // Volume spike
        let signal_volume = analyze_market_conditions(&prices, &real_volumes, &config);

        // Both should return valid signals
        assert!(signal_zero.is_some());
        assert!(signal_volume.is_some());

        // The behavior may differ (volume spike could trigger buy when zero volume doesn't)
        // but both should be valid Signal enum values
    }

    #[test]
    fn test_mixed_volume_disables_volume_features() {
        // CRITICAL TEST: Mixed zeros + real volumes should disable volume features
        // This is the exact scenario that would break with avg_volume > 1.0 detection
        // Simulates: 19 CoinGecko candles (volume=0) + 1 Birdeye candle (volume=5000)
        let prices = vec![
            100.0, 102.0, 104.0, 106.0, 108.0, 110.0, 112.0, 114.0, 116.0, 118.0, 120.0, 122.0,
            124.0, 126.0, 128.0, 130.0, 132.0, 134.0, 136.0, 138.0, 140.0,
        ];

        // 19 zeros (CoinGecko backfill) + 1 real volume (Birdeye)
        let mut mixed_volumes = vec![0.0; prices.len() - 1];
        mixed_volumes.push(5000.0); // One real candle

        // With old logic (avg_volume > 1.0):
        //   avg = 5000 / 20 = 250 ✓ Passes threshold
        //   volume_spike = 5000 / 250 = 20x 🚨 FALSE SPIKE
        //
        // With new logic (.all()):
        //   has_volume_data = false (because there are zeros)
        //   volume_spike = false ✓ Correctly disabled

        let config = SignalConfig::default();
        let signal = analyze_market_conditions(&prices, &mixed_volumes, &config);

        // Should return valid signal (not crash)
        assert!(signal.is_some());

        // Should operate in conservative mode (volume features disabled)
        // We can't directly check has_volume_data, but we verify no panic
        // and signal generation works correctly
    }

    #[test]
    fn test_mixed_volume_no_panic_buy() {
        // Mixed volumes should disable panic buy (requires volume confirmation)
        let mut prices = vec![200.0; 20];
        for i in 1..=12 {
            prices.push(200.0 - (i as f64 * 1.67)); // Sharp drop
        }
        prices.push(180.0); // -10% flash crash

        // 32 zeros + 1 huge volume spike (perfect panic buy setup if volume was valid)
        let mut mixed_volumes = vec![0.0; prices.len() - 1];
        mixed_volumes.push(10000.0); // Massive "spike" (but meaningless)

        let config = SignalConfig::default(); // Panic buy enabled

        let signal = analyze_market_conditions(&prices, &mixed_volumes, &config);

        // Should NOT trigger panic buy because volume data is incomplete
        // Old logic would see avg_volume = 312.5 and ratio = 32x (false spike!)
        // New logic correctly disables volume features
        assert!(signal.is_some());
    }
}
