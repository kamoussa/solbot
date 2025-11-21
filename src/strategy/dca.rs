use super::Strategy;
use crate::models::{Candle, Signal};
use crate::Result;
use chrono::{DateTime, Utc};
use std::sync::Mutex;

/// Dollar Cost Averaging (DCA) strategy
///
/// Buys at fixed intervals regardless of price, accumulating over time.
/// This is a true passive investing baseline.
///
/// Strategy:
/// - Buy fixed amount at regular intervals (e.g., every 168 hours = weekly)
/// - Never sells (pure accumulation)
/// - Averages entry price over time
/// - No timing or technical analysis
#[derive(Debug)]
pub struct DCAStrategy {
    buy_interval_hours: i64,
    last_buy_time: Mutex<Option<DateTime<Utc>>>,
}

impl DCAStrategy {
    /// Create a new DCA strategy
    ///
    /// # Arguments
    /// * `buy_interval_hours` - Hours between each buy (e.g., 168 for weekly, 720 for monthly)
    pub fn new(buy_interval_hours: i64) -> Self {
        Self {
            buy_interval_hours,
            last_buy_time: Mutex::new(None),
        }
    }

    /// Create a weekly DCA strategy (buys every 7 days)
    pub fn weekly() -> Self {
        Self::new(168)
    }

    /// Create a bi-weekly DCA strategy (buys every 14 days)
    pub fn biweekly() -> Self {
        Self::new(336)
    }

    /// Create a monthly DCA strategy (buys every 30 days)
    pub fn monthly() -> Self {
        Self::new(720)
    }
}

impl Default for DCAStrategy {
    fn default() -> Self {
        Self::weekly()
    }
}

impl Strategy for DCAStrategy {
    fn generate_signal(&self, candles: &[Candle]) -> Result<Signal> {
        if candles.is_empty() {
            return Err("No candles provided".into());
        }

        let current_candle = candles.last().unwrap();
        let current_time = current_candle.timestamp;
        let mut last_buy = self.last_buy_time.lock().unwrap();

        let should_buy = match *last_buy {
            None => {
                tracing::debug!("🔍 DCA: First buy at {}", current_time);
                true
            }
            Some(last_time) => {
                // Calculate hours elapsed since last buy
                let duration = current_time.signed_duration_since(last_time);
                let hours_elapsed = duration.num_hours();
                let should = hours_elapsed >= self.buy_interval_hours;
                tracing::debug!(
                    "🔍 DCA: Current={}, Last={}, Elapsed={}h, Interval={}h, ShouldBuy={}",
                    current_time,
                    last_time,
                    hours_elapsed,
                    self.buy_interval_hours,
                    should
                );
                should
            }
        };

        if should_buy {
            *last_buy = Some(current_time);
            tracing::info!("💰 DCA BUY @ {}", current_time);
            Ok(Signal::Buy)
        } else {
            Ok(Signal::Hold)
        }
    }

    fn name(&self) -> &str {
        "DCA"
    }

    fn min_candles_required(&self) -> usize {
        1 // Only needs current candle
    }

    fn samples_needed(&self, _poll_interval_minutes: u64) -> usize {
        1 // Only needs current candle
    }

    fn lookback_hours(&self) -> u64 {
        0 // No lookback needed
    }

    fn skip_automatic_exits(&self) -> bool {
        true // DCA never sells, pure accumulation
    }

    fn supports_accumulation(&self) -> bool {
        true // DCA accumulates by adding to positions over time
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};

    /// Create a candle at a specific timestamp
    fn create_candle_at(hours_from_start: i64) -> Candle {
        let base_time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        Candle {
            token: "TEST".to_string(),
            timestamp: base_time + chrono::Duration::hours(hours_from_start),
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.0,
            volume: 1000.0,
        }
    }

    #[test]
    fn test_first_buy() {
        let strategy = DCAStrategy::weekly();
        let candles = vec![create_candle_at(0)];

        let signal = strategy.generate_signal(&candles).unwrap();
        assert_eq!(signal, Signal::Buy, "First call should always buy");
    }

    #[test]
    fn test_holds_between_intervals() {
        let strategy = DCAStrategy::new(10); // Buy every 10 hours

        // First buy at hour 0
        let candles = vec![create_candle_at(0)];
        assert_eq!(strategy.generate_signal(&candles).unwrap(), Signal::Buy);

        // Hour 5: too soon - should hold
        let candles = vec![create_candle_at(5)];
        assert_eq!(strategy.generate_signal(&candles).unwrap(), Signal::Hold);

        // Hour 9: still too soon
        let candles = vec![create_candle_at(9)];
        assert_eq!(strategy.generate_signal(&candles).unwrap(), Signal::Hold);

        // Hour 10: exactly 10 hours - should buy again
        let candles = vec![create_candle_at(10)];
        assert_eq!(strategy.generate_signal(&candles).unwrap(), Signal::Buy);
    }

    #[test]
    fn test_weekly_interval() {
        let strategy = DCAStrategy::weekly();

        // First buy at hour 0
        let candles = vec![create_candle_at(0)];
        assert_eq!(strategy.generate_signal(&candles).unwrap(), Signal::Buy);

        // 1 week - 1 hour (167 hours): should hold
        let candles = vec![create_candle_at(167)];
        assert_eq!(strategy.generate_signal(&candles).unwrap(), Signal::Hold);

        // 1 week (168 hours): should buy
        let candles = vec![create_candle_at(168)];
        assert_eq!(strategy.generate_signal(&candles).unwrap(), Signal::Buy);

        // 2 weeks - 1 hour (335 hours from start, 167 from last buy): should hold
        let candles = vec![create_candle_at(335)];
        assert_eq!(strategy.generate_signal(&candles).unwrap(), Signal::Hold);

        // 2 weeks (336 hours from start, 168 from last buy): should buy again
        let candles = vec![create_candle_at(336)];
        assert_eq!(strategy.generate_signal(&candles).unwrap(), Signal::Buy);
    }

    #[test]
    fn test_never_sells() {
        let strategy = DCAStrategy::weekly();

        // Generate many signals - should only ever be Buy or Hold, never Sell
        for hour in 0..=1000 {
            let candles = vec![create_candle_at(hour)];
            let signal = strategy.generate_signal(&candles).unwrap();
            assert_ne!(
                signal,
                Signal::Sell,
                "DCA should never sell (at hour {})",
                hour
            );
        }
    }

    #[test]
    fn test_empty_candles_returns_error() {
        let strategy = DCAStrategy::weekly();
        let candles: Vec<Candle> = vec![];

        let result = strategy.generate_signal(&candles);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No candles"));
    }

    #[test]
    fn test_skips_automatic_exits() {
        let strategy = DCAStrategy::weekly();
        assert_eq!(
            strategy.skip_automatic_exits(),
            true,
            "DCA should skip automatic exits for pure accumulation"
        );
    }

    #[test]
    fn test_strategy_name() {
        let strategy = DCAStrategy::weekly();
        assert_eq!(strategy.name(), "DCA");
    }

    #[test]
    fn test_yearly_simulation() {
        // Simulate 1 year of hourly candles (8760 hours)
        let strategy = DCAStrategy::weekly();
        let base_time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();

        let mut buy_count = 0;

        // Generate hourly candles for a full year
        for hour in 0..8760 {
            let timestamp = base_time + chrono::Duration::hours(hour);
            let candles = vec![Candle {
                token: "TEST".to_string(),
                timestamp,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.0,
                volume: 1000.0,
            }];

            let signal = strategy.generate_signal(&candles).unwrap();
            if signal == Signal::Buy {
                buy_count += 1;
            }
        }

        // Should make approximately 52 weekly buys (8760 hours / 168 hours = 52.14)
        assert!(
            buy_count >= 51 && buy_count <= 53,
            "Expected 51-53 buys over 1 year, got {}",
            buy_count
        );
    }
}
