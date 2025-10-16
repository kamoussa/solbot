use crate::models::Candle;
use chrono::{DateTime, Duration, Utc};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// Market scenario types for synthetic data generation
#[derive(Debug, Clone, Copy)]
pub enum MarketScenario {
    /// Steady uptrend with noise (+2% daily average)
    Uptrend,
    /// Steady downtrend with noise (-2% daily average)
    Downtrend,
    /// Sideways/choppy market (±1% around mean)
    Sideways,
    /// High volatility (±5% large swings)
    Volatile,
    /// Contains time gaps (missing candles)
    WithGaps,
    /// Rapid drawdown to test circuit breakers
    DrawdownTest,
}

/// Generates synthetic price data for backtesting
pub struct SyntheticDataGenerator {
    rng: StdRng,
    base_price: f64,
    base_volume: f64,
}

impl SyntheticDataGenerator {
    /// Create a new generator with a seed for reproducibility
    pub fn new(seed: u64) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            base_price: 150.0,
            base_volume: 1_000_000.0,
        }
    }

    /// Generate candles for a specific market scenario
    ///
    /// # Arguments
    /// * `scenario` - The market scenario to simulate
    /// * `num_candles` - Number of candles to generate (recommend 500+ for full backtest)
    /// * `interval_minutes` - Minutes between candles (default: 5)
    ///
    /// # Returns
    /// Vec of candles with timestamps, prices, and volume
    pub fn generate(
        &mut self,
        scenario: MarketScenario,
        num_candles: usize,
        interval_minutes: i64,
    ) -> Vec<Candle> {
        let start_time = Utc::now() - Duration::minutes(num_candles as i64 * interval_minutes);

        match scenario {
            MarketScenario::Uptrend => {
                self.generate_uptrend(start_time, num_candles, interval_minutes)
            }
            MarketScenario::Downtrend => {
                self.generate_downtrend(start_time, num_candles, interval_minutes)
            }
            MarketScenario::Sideways => {
                self.generate_sideways(start_time, num_candles, interval_minutes)
            }
            MarketScenario::Volatile => {
                self.generate_volatile(start_time, num_candles, interval_minutes)
            }
            MarketScenario::WithGaps => {
                self.generate_with_gaps(start_time, num_candles, interval_minutes)
            }
            MarketScenario::DrawdownTest => {
                self.generate_drawdown(start_time, num_candles, interval_minutes)
            }
        }
    }

    /// Generate uptrend: +2% daily with noise
    fn generate_uptrend(
        &mut self,
        start_time: DateTime<Utc>,
        num_candles: usize,
        interval_minutes: i64,
    ) -> Vec<Candle> {
        let mut candles = Vec::with_capacity(num_candles);
        let mut current_price = self.base_price;

        // +2% per day = +0.0000347% per minute = ~0.00174% per 5 minutes
        let drift_per_interval = 0.02 / (24.0 * 60.0 / interval_minutes as f64);

        for i in 0..num_candles {
            let timestamp = start_time + Duration::minutes(i as i64 * interval_minutes);

            // Apply drift + reduced noise so trend is dominant
            let drift = current_price * drift_per_interval;
            let noise = current_price * self.rng.gen_range(-0.001..0.001); // ±0.1% noise
            current_price += drift + noise;

            let candle = self.create_candle(current_price, timestamp);
            candles.push(candle);
        }

        candles
    }

    /// Generate downtrend: -2% daily with noise
    fn generate_downtrend(
        &mut self,
        start_time: DateTime<Utc>,
        num_candles: usize,
        interval_minutes: i64,
    ) -> Vec<Candle> {
        let mut candles = Vec::with_capacity(num_candles);
        let mut current_price = self.base_price;

        let drift_per_interval = -0.02 / (24.0 * 60.0 / interval_minutes as f64);

        for i in 0..num_candles {
            let timestamp = start_time + Duration::minutes(i as i64 * interval_minutes);

            // Apply drift + reduced noise so trend is dominant
            let drift = current_price * drift_per_interval;
            let noise = current_price * self.rng.gen_range(-0.001..0.001); // ±0.1% noise
            current_price += drift + noise;

            let candle = self.create_candle(current_price, timestamp);
            candles.push(candle);
        }

        candles
    }

    /// Generate sideways market: mean-reverting random walk
    fn generate_sideways(
        &mut self,
        start_time: DateTime<Utc>,
        num_candles: usize,
        interval_minutes: i64,
    ) -> Vec<Candle> {
        let mut candles = Vec::with_capacity(num_candles);
        let mut current_price = self.base_price;
        let mean_price = self.base_price;

        for i in 0..num_candles {
            let timestamp = start_time + Duration::minutes(i as i64 * interval_minutes);

            // Mean reversion force + noise
            let reversion = (mean_price - current_price) * 0.1; // 10% pull to mean
            let noise = current_price * self.rng.gen_range(-0.01..0.01); // ±1% noise
            current_price += reversion + noise;

            let candle = self.create_candle(current_price, timestamp);
            candles.push(candle);
        }

        candles
    }

    /// Generate volatile market: large swings
    fn generate_volatile(
        &mut self,
        start_time: DateTime<Utc>,
        num_candles: usize,
        interval_minutes: i64,
    ) -> Vec<Candle> {
        let mut candles = Vec::with_capacity(num_candles);
        let mut current_price = self.base_price;

        for i in 0..num_candles {
            let timestamp = start_time + Duration::minutes(i as i64 * interval_minutes);

            // Large random moves
            let change = current_price * self.rng.gen_range(-0.05..0.05); // ±5% per candle
            current_price += change;

            // Prevent price from going too low
            if current_price < self.base_price * 0.5 {
                current_price = self.base_price * 0.5;
            }

            let candle = self.create_candle(current_price, timestamp);
            candles.push(candle);
        }

        candles
    }

    /// Generate data with time gaps
    fn generate_with_gaps(
        &mut self,
        start_time: DateTime<Utc>,
        num_candles: usize,
        interval_minutes: i64,
    ) -> Vec<Candle> {
        let mut candles = Vec::with_capacity(num_candles);
        let mut current_price = self.base_price;
        let mut actual_index = 0;

        for i in 0..num_candles {
            // Skip every 50th candle to create gaps
            if i % 50 == 49 {
                actual_index += 2; // Create a gap
                continue;
            }

            let timestamp = start_time + Duration::minutes(actual_index as i64 * interval_minutes);

            let change = current_price * self.rng.gen_range(-0.01..0.01);
            current_price += change;

            let candle = self.create_candle(current_price, timestamp);
            candles.push(candle);

            actual_index += 1;
        }

        candles
    }

    /// Generate rapid drawdown scenario
    fn generate_drawdown(
        &mut self,
        start_time: DateTime<Utc>,
        num_candles: usize,
        interval_minutes: i64,
    ) -> Vec<Candle> {
        let mut candles = Vec::with_capacity(num_candles);
        let mut current_price = self.base_price;

        for i in 0..num_candles {
            let timestamp = start_time + Duration::minutes(i as i64 * interval_minutes);

            // First half: normal growth
            // Second half: rapid 25% drop
            if i < num_candles / 2 {
                let change = current_price * self.rng.gen_range(-0.005..0.01); // Slight uptrend
                current_price += change;
            } else {
                // Rapid decline: -25% over second half
                let drop_rate = -0.25 / (num_candles as f64 / 2.0);
                let drop = current_price * drop_rate;
                let noise = current_price * self.rng.gen_range(-0.005..0.005);
                current_price += drop + noise;
            }

            let candle = self.create_candle(current_price, timestamp);
            candles.push(candle);
        }

        candles
    }

    /// Helper to create a candle from price and timestamp
    fn create_candle(&mut self, price: f64, timestamp: DateTime<Utc>) -> Candle {
        // Create realistic OHLC from close price
        let noise_pct = 0.002; // ±0.2% intrabar movement

        // Generate high and low around the close price
        let high = price * (1.0 + self.rng.gen_range(0.0..noise_pct));
        let low = price * (1.0 - self.rng.gen_range(0.0..noise_pct));

        // Generate open and clamp it between low and high
        let open_raw = price * (1.0 + self.rng.gen_range(-noise_pct..noise_pct));
        let open = open_raw.clamp(low, high);

        // Vary volume ±30%
        let volume = self.base_volume * self.rng.gen_range(0.7..1.3);

        Candle {
            token: "SYNTH".to_string(),
            timestamp,
            open,
            high,
            low,
            close: price,
            volume,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_uptrend() {
        let mut gen = SyntheticDataGenerator::new(42);
        let candles = gen.generate(MarketScenario::Uptrend, 500, 5);

        assert_eq!(candles.len(), 500);

        // First and last price - should be higher at end
        let first_price = candles.first().unwrap().close;
        let last_price = candles.last().unwrap().close;

        assert!(
            last_price > first_price,
            "Uptrend should end higher: {} -> {}",
            first_price,
            last_price
        );
    }

    #[test]
    fn test_generate_downtrend() {
        let mut gen = SyntheticDataGenerator::new(42);
        let candles = gen.generate(MarketScenario::Downtrend, 500, 5);

        assert_eq!(candles.len(), 500);

        let first_price = candles.first().unwrap().close;
        let last_price = candles.last().unwrap().close;

        assert!(
            last_price < first_price,
            "Downtrend should end lower: {} -> {}",
            first_price,
            last_price
        );
    }

    #[test]
    fn test_generate_sideways() {
        let mut gen = SyntheticDataGenerator::new(42);
        let candles = gen.generate(MarketScenario::Sideways, 500, 5);

        assert_eq!(candles.len(), 500);

        // Should stay roughly around base price (±10%)
        let base = gen.base_price;
        for candle in &candles {
            assert!(
                candle.close > base * 0.9 && candle.close < base * 1.1,
                "Sideways should stay near base: {} vs {}",
                candle.close,
                base
            );
        }
    }

    #[test]
    fn test_generate_with_gaps() {
        let mut gen = SyntheticDataGenerator::new(42);
        let candles = gen.generate(MarketScenario::WithGaps, 100, 5);

        // Should have fewer candles due to gaps
        assert!(candles.len() < 100);

        // Verify there's actually a gap in timestamps
        let mut has_gap = false;
        for i in 1..candles.len() {
            let time_diff = (candles[i].timestamp - candles[i - 1].timestamp).num_minutes();
            if time_diff > 5 {
                has_gap = true;
                break;
            }
        }

        assert!(has_gap, "Should contain time gaps");
    }

    #[test]
    fn test_timestamps_are_sequential() {
        let mut gen = SyntheticDataGenerator::new(42);
        let candles = gen.generate(MarketScenario::Uptrend, 100, 5);

        for i in 1..candles.len() {
            assert!(
                candles[i].timestamp > candles[i - 1].timestamp,
                "Timestamps should be sequential"
            );
        }
    }

    #[test]
    fn test_ohlc_consistency() {
        let mut gen = SyntheticDataGenerator::new(42);
        let candles = gen.generate(MarketScenario::Uptrend, 100, 5);

        for candle in &candles {
            assert!(candle.high >= candle.close, "High should be >= close");
            assert!(candle.high >= candle.open, "High should be >= open");
            assert!(candle.low <= candle.close, "Low should be <= close");
            assert!(candle.low <= candle.open, "Low should be <= open");
        }
    }
}
