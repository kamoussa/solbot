use crate::indicators::{calculate_rsi, calculate_sma};
use crate::models::{Candle, Signal};
use crate::strategy::Strategy;
use crate::Result;

/// Mean reversion trading strategy
///
/// Buys extreme dips (panic selling) and sells when price returns to mean.
/// Designed to capture overreactions in volatile markets.
///
/// Entry conditions (ALL must be true):
/// - Price is significantly below moving average (oversold_threshold, e.g. -8%)
/// - RSI is extremely low (rsi_extreme, e.g. < 20)
/// - Volume spike indicates panic (volume_multiplier, e.g. 2.0x average)
/// - Momentum is slowing (not a falling knife)
///
/// Exit conditions (ANY triggers exit):
/// - Price returns near moving average (within 2%)
/// - Profit target reached (e.g. +6%)
/// - Maximum hold time exceeded (e.g. 7 days)
#[derive(Debug, Clone)]
pub struct MeanReversionStrategy {
    config: MeanReversionConfig,
    poll_interval_minutes: u64,
}

#[derive(Debug, Clone)]
pub struct MeanReversionConfig {
    /// Moving average period for mean calculation
    pub ma_period: usize,

    /// How far below MA constitutes "extreme dip" (e.g. -0.08 = 8% below)
    pub oversold_threshold: f64,

    /// RSI level for "extremely oversold" (e.g. 20)
    pub rsi_extreme: f64,

    /// Volume multiplier to confirm panic (e.g. 2.0 = 2x average volume)
    pub volume_multiplier: f64,

    /// Profit target percentage to exit (e.g. 0.06 = 6%)
    pub profit_target_pct: f64,

    /// Maximum days to hold position before forced exit
    pub max_hold_days: i64,

    /// RSI period for calculation
    pub rsi_period: usize,
}

impl Default for MeanReversionConfig {
    fn default() -> Self {
        Self {
            ma_period: 20,              // 20-period MA for mean
            oversold_threshold: -0.08,  // 8% below MA (optimized from -10%)
            rsi_extreme: 20.0,          // Deeply oversold (optimized from 25)
            volume_multiplier: 2.0,     // 2x average volume
            profit_target_pct: 0.06,    // 6% profit target (optimized from 8%)
            max_hold_days: 7,           // Max 7 days hold
            rsi_period: 14,             // Standard RSI period
        }
    }
}

impl MeanReversionStrategy {
    pub fn new(config: MeanReversionConfig) -> Self {
        Self {
            config,
            poll_interval_minutes: 60, // Default hourly
        }
    }

    pub fn with_poll_interval(mut self, minutes: u64) -> Self {
        self.poll_interval_minutes = minutes;
        self
    }
}

impl Default for MeanReversionStrategy {
    fn default() -> Self {
        Self::new(MeanReversionConfig::default())
    }
}

impl Strategy for MeanReversionStrategy {
    fn generate_signal(&self, candles: &[Candle]) -> Result<Signal> {
        if candles.len() < self.min_candles_required() {
            return Err(format!(
                "Need at least {} candles for mean reversion strategy, got {}",
                self.min_candles_required(),
                candles.len()
            )
            .into());
        }

        let current = candles.last().unwrap();
        let current_price = current.close;

        // Calculate indicators
        let prices: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let volumes: Vec<f64> = candles.iter().map(|c| c.volume).collect();

        let ma = calculate_sma(&prices, self.config.ma_period).ok_or("Failed to calculate MA")?;
        let rsi = calculate_rsi(&prices, self.config.rsi_period).ok_or("Failed to calculate RSI")?;

        // Calculate average volume (exclude current candle for comparison)
        let avg_volume = if volumes.len() > 1 {
            volumes[..volumes.len() - 1].iter().sum::<f64>() / (volumes.len() - 1) as f64
        } else {
            current.volume
        };

        // Entry logic: Buy extreme dips
        let price_vs_ma = (current_price - ma) / ma;
        let volume_ratio = current.volume / avg_volume;

        // Check if momentum is slowing (not a falling knife)
        let momentum_slowing = if candles.len() >= 3 {
            let current_change = prices[prices.len() - 1] - prices[prices.len() - 2];
            let previous_change = prices[prices.len() - 2] - prices[prices.len() - 3];
            // Momentum slowing = current drop is smaller than previous drop
            current_change > previous_change
        } else {
            false
        };

        let entry_conditions = vec![
            price_vs_ma < self.config.oversold_threshold,
            rsi < self.config.rsi_extreme,
            volume_ratio > self.config.volume_multiplier,
            momentum_slowing,
        ];

        let conditions_met = entry_conditions.iter().filter(|&&x| x).count();

        tracing::debug!(
            "Mean Reversion Entry Check: price_vs_ma={:.2}% (<{:.0}%?={}), rsi={:.1} (<{}?={}), volume_ratio={:.2}x (>{}x?={}), momentum_slowing={} | {}/4 conditions",
            price_vs_ma * 100.0,
            self.config.oversold_threshold * 100.0,
            entry_conditions[0],
            rsi,
            self.config.rsi_extreme,
            entry_conditions[1],
            volume_ratio,
            self.config.volume_multiplier,
            entry_conditions[2],
            momentum_slowing,
            conditions_met
        );

        if entry_conditions.iter().all(|&x| x) {
            tracing::info!(
                "🎯 MEAN REVERSION BUY: price ${:.2} is {:.1}% below MA {:.2}, RSI {:.1}, volume {:.2}x",
                current_price,
                price_vs_ma * 100.0,
                ma,
                rsi,
                volume_ratio
            );
            return Ok(Signal::Buy);
        }

        // Exit logic would be handled by position manager
        // (return to MA, profit target, time stop)
        Ok(Signal::Hold)
    }

    fn name(&self) -> &str {
        "Mean Reversion"
    }

    fn min_candles_required(&self) -> usize {
        // Need MA period + RSI period + a few extra for momentum check
        self.config.ma_period.max(self.config.rsi_period) + 3
    }

    fn samples_needed(&self, poll_interval_minutes: u64) -> usize {
        // Calculate samples based on lookback period
        let lookback_minutes = self.lookback_hours() * 60;
        (lookback_minutes / poll_interval_minutes) as usize
    }

    fn lookback_hours(&self) -> u64 {
        // Need enough data for MA calculation
        // MA period * poll interval in hours
        let hours_per_candle = self.poll_interval_minutes / 60;
        self.config.ma_period as u64 * hours_per_candle + 24 // Extra buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_candles(prices: Vec<f64>, volumes: Vec<f64>) -> Vec<Candle> {
        let num_prices = prices.len();
        prices
            .into_iter()
            .zip(volumes.into_iter())
            .enumerate()
            .map(|(i, (price, volume))| Candle {
                token: "TEST".to_string(),
                timestamp: Utc::now() - chrono::Duration::hours((num_prices - i) as i64),
                open: price,
                high: price * 1.01,
                low: price * 0.99,
                close: price,
                volume,
            })
            .collect()
    }

    #[test]
    fn test_mean_reversion_requires_minimum_candles() {
        let strategy = MeanReversionStrategy::default();
        let candles = create_test_candles(vec![100.0; 10], vec![1000.0; 10]);

        let result = strategy.generate_signal(&candles);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Need at least"));
    }

    #[test]
    fn test_mean_reversion_no_signal_in_normal_market() {
        let strategy = MeanReversionStrategy::default();

        // Normal market: price near MA, normal RSI, normal volume
        let prices = vec![100.0; 30]; // Flat prices = high RSI, no dip
        let volumes = vec![1000.0; 30];
        let candles = create_test_candles(prices, volumes);

        let signal = strategy.generate_signal(&candles).unwrap();
        assert_eq!(signal, Signal::Hold);
    }

    #[test]
    fn test_mean_reversion_buy_on_extreme_dip() {
        let strategy = MeanReversionStrategy::default();

        // Create a panic dip scenario:
        // - Prices drop 15% below recent average (MA ~100, current ~85)
        // - Volume spikes 3x
        // - Momentum slowing (drops getting smaller)
        let mut prices = vec![100.0; 20]; // Establish MA at 100
        prices.extend(vec![95.0, 90.0, 87.0, 85.0]); // Drop with slowing momentum

        let mut volumes = vec![1000.0; 20];
        volumes.extend(vec![1500.0, 2000.0, 2500.0, 3000.0]); // Volume increasing

        let candles = create_test_candles(prices, volumes);

        let signal = strategy.generate_signal(&candles).unwrap();
        assert_eq!(signal, Signal::Buy);
    }

    #[test]
    fn test_mean_reversion_no_buy_on_falling_knife() {
        let strategy = MeanReversionStrategy::default();

        // Falling knife: drops accelerating (not slowing)
        let mut prices = vec![100.0; 20];
        prices.extend(vec![95.0, 88.0, 78.0, 65.0]); // Accelerating drops

        let mut volumes = vec![1000.0; 20];
        volumes.extend(vec![3000.0; 4]); // High volume but accelerating drop

        let candles = create_test_candles(prices, volumes);

        let signal = strategy.generate_signal(&candles).unwrap();
        // Should not buy - momentum NOT slowing
        assert_eq!(signal, Signal::Hold);
    }

    #[test]
    fn test_mean_reversion_no_buy_without_volume_spike() {
        let strategy = MeanReversionStrategy::default();

        // Price dips but volume normal (not panic)
        let mut prices = vec![100.0; 20];
        prices.extend(vec![95.0, 90.0, 87.0, 85.0]);

        let volumes = vec![1000.0; 24]; // No volume spike

        let candles = create_test_candles(prices, volumes);

        let signal = strategy.generate_signal(&candles).unwrap();
        // Should not buy - no volume confirmation
        assert_eq!(signal, Signal::Hold);
    }
}
