use super::{
    signals::{analyze_market_conditions, validate_candle_uniformity, SignalConfig},
    Strategy,
};
use crate::models::{Candle, Signal};
use crate::Result;

/// Momentum-based swing trading strategy
///
/// This strategy identifies momentum shifts using:
/// - RSI for overbought/oversold conditions
/// - Moving average crossovers for trend direction
/// - Volume analysis for confirmation
///
/// Designed for 1-7 day holding periods (swing trading)
#[derive(Debug, Clone)]
pub struct MomentumStrategy {
    config: SignalConfig,
    poll_interval_minutes: u64,
}

impl MomentumStrategy {
    pub fn new(config: SignalConfig) -> Self {
        Self {
            config,
            poll_interval_minutes: 5, // Default: 5 minutes
        }
    }

    pub fn with_poll_interval(mut self, poll_interval_minutes: u64) -> Self {
        self.poll_interval_minutes = poll_interval_minutes;
        self
    }

    /// Calculate samples needed based on poll interval
    ///
    /// # Arguments
    /// * `poll_interval_minutes` - How often we sample prices
    ///
    /// # Example
    /// ```
    /// use cryptobot::strategy::momentum::MomentumStrategy;
    ///
    /// let strategy = MomentumStrategy::default();
    /// // 24hr lookback, 30min polling = 48 samples needed
    /// assert_eq!(strategy.samples_needed(30), 48);
    /// ```
    pub fn samples_needed(&self, poll_interval_minutes: u64) -> usize {
        self.config.samples_needed(poll_interval_minutes)
    }

    /// Get lookback duration in hours
    pub fn lookback_hours(&self) -> u64 {
        self.config.lookback_hours
    }

    /// Extract prices from candles
    fn extract_prices(candles: &[Candle]) -> Vec<f64> {
        candles.iter().map(|c| c.close).collect()
    }

    /// Extract volumes from candles
    fn extract_volumes(candles: &[Candle]) -> Vec<f64> {
        candles.iter().map(|c| c.volume).collect()
    }
}

impl Default for MomentumStrategy {
    fn default() -> Self {
        Self::new(SignalConfig::default())
    }
}

impl Strategy for MomentumStrategy {
    fn generate_signal(&self, candles: &[Candle]) -> Result<Signal> {
        if candles.len() < self.min_candles_required() {
            return Err(format!(
                "Insufficient data: {} candles, need {}",
                candles.len(),
                self.min_candles_required()
            )
            .into());
        }

        // Validate that candles are uniformly spaced (no gaps)
        let expected_interval_secs = self.poll_interval_minutes * 60;
        validate_candle_uniformity(candles, expected_interval_secs)?;

        let prices = Self::extract_prices(candles);
        let volumes = Self::extract_volumes(candles);

        let signal = analyze_market_conditions(&prices, &volumes, &self.config)
            .ok_or("Failed to generate signal from market data")?;

        Ok(signal)
    }

    fn name(&self) -> &str {
        "MomentumStrategy"
    }

    fn min_candles_required(&self) -> usize {
        // Need enough data for longest indicator (long MA + buffer)
        self.config.long_ma_period + 5
    }

    fn samples_needed(&self, poll_interval_minutes: u64) -> usize {
        self.config.samples_needed(poll_interval_minutes)
    }

    fn lookback_hours(&self) -> u64 {
        self.config.lookback_hours
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_candles(prices: Vec<f64>, volumes: Vec<f64>) -> Vec<Candle> {
        prices
            .iter()
            .zip(volumes.iter())
            .enumerate()
            .map(|(i, (&price, &volume))| Candle {
                token: "TEST".to_string(),
                // Space candles 5 minutes apart to match polling interval
                timestamp: Utc::now() - chrono::Duration::minutes((prices.len() - i) as i64 * 5),
                open: price,
                high: price * 1.01,
                low: price * 0.99,
                close: price,
                volume,
            })
            .collect()
    }

    #[test]
    fn test_strategy_requires_sufficient_data() {
        let strategy = MomentumStrategy::default();
        let candles = create_test_candles(vec![100.0, 101.0], vec![1000.0, 1100.0]);

        let result = strategy.generate_signal(&candles);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Insufficient data"));
    }

    #[test]
    fn test_strategy_with_sufficient_data() {
        let strategy = MomentumStrategy::default();

        // Create uptrend data
        let prices: Vec<f64> = (0..30).map(|i| 100.0 + i as f64).collect();
        let volumes = vec![1000.0; 30];
        let candles = create_test_candles(prices, volumes);

        let result = strategy.generate_signal(&candles);
        assert!(result.is_ok());

        let signal = result.unwrap();
        assert!(matches!(signal, Signal::Buy | Signal::Sell | Signal::Hold));
    }

    #[test]
    fn test_strategy_name() {
        let strategy = MomentumStrategy::default();
        assert_eq!(strategy.name(), "MomentumStrategy");
    }

    #[test]
    fn test_min_candles_required() {
        let strategy = MomentumStrategy::default();
        let min_required = strategy.min_candles_required();

        // Should be long_ma_period (20) + 5 = 25
        assert_eq!(min_required, 25);
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
            lookback_hours: 12,
        };

        let strategy = MomentumStrategy::new(config);
        assert_eq!(strategy.min_candles_required(), 20); // 15 + 5
        assert_eq!(strategy.lookback_hours(), 12);

        // Test samples calculation: 12 hours / 30 min = 24 samples
        assert_eq!(strategy.samples_needed(30), 24);
    }

    #[test]
    fn test_downtrend_generates_hold_or_sell() {
        let strategy = MomentumStrategy::default();

        // Create downtrend
        let prices: Vec<f64> = (0..30).map(|i| 200.0 - i as f64 * 2.0).collect();
        let volumes = vec![1000.0; 30];
        let candles = create_test_candles(prices, volumes);

        let result = strategy.generate_signal(&candles);
        assert!(result.is_ok());

        let signal = result.unwrap();
        // In downtrend, should not generate buy signal
        assert!(matches!(signal, Signal::Hold | Signal::Sell));
    }

    #[test]
    fn test_sideways_market_generates_hold() {
        let strategy = MomentumStrategy::default();

        // Create sideways market (oscillating)
        let prices: Vec<f64> = (0..30)
            .map(|i| if i % 2 == 0 { 100.0 } else { 102.0 })
            .collect();
        let volumes = vec![1000.0; 30];
        let candles = create_test_candles(prices, volumes);

        let result = strategy.generate_signal(&candles);
        assert!(result.is_ok());

        let signal = result.unwrap();
        // Sideways market should likely generate hold
        assert_eq!(signal, Signal::Hold);
    }
}
