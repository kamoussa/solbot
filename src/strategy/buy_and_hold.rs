use super::Strategy;
use crate::models::{Candle, Signal};
use crate::Result;

/// Buy-and-Hold baseline strategy
///
/// This strategy buys on the first candle and holds forever.
/// Used as a baseline to compare against active trading strategies.
///
/// Strategy:
/// - Buy on first candle in dataset
/// - Hold indefinitely (never sells)
/// - Represents passive "set and forget" investing
#[derive(Debug, Clone)]
pub struct BuyAndHoldStrategy;

impl BuyAndHoldStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BuyAndHoldStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl Strategy for BuyAndHoldStrategy {
    fn generate_signal(&self, candles: &[Candle]) -> Result<Signal> {
        if candles.is_empty() {
            return Err("No candles provided".into());
        }

        // Always return Buy - the position manager will only open one position
        // and we never sell, so this effectively implements buy-and-hold
        Ok(Signal::Buy)
    }

    fn name(&self) -> &str {
        "BuyAndHold"
    }

    fn min_candles_required(&self) -> usize {
        1 // Only needs 1 candle to make a decision
    }

    fn samples_needed(&self, _poll_interval_minutes: u64) -> usize {
        1 // Only needs current candle
    }

    fn lookback_hours(&self) -> u64 {
        0 // No lookback needed
    }

    fn skip_automatic_exits(&self) -> bool {
        true // Pure buy-and-hold: no forced exits, hold forever
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_candles(count: usize) -> Vec<Candle> {
        (0..count)
            .map(|i| Candle {
                token: "TEST".to_string(),
                timestamp: Utc::now() - chrono::Duration::minutes((count - i) as i64 * 5),
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.0,
                volume: 1000.0,
            })
            .collect()
    }

    #[test]
    fn test_always_generates_buy() {
        let strategy = BuyAndHoldStrategy::new();

        // Test with different candle counts - should always return Buy
        for count in [1, 5, 10, 50, 100] {
            let candles = create_test_candles(count);
            let signal = strategy.generate_signal(&candles).unwrap();
            assert_eq!(
                signal,
                Signal::Buy,
                "Should always return Buy (got {:?} for {} candles)",
                signal,
                count
            );
        }
    }

    #[test]
    fn test_empty_candles_returns_error() {
        let strategy = BuyAndHoldStrategy::new();
        let candles: Vec<Candle> = vec![];

        let result = strategy.generate_signal(&candles);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No candles"));
    }

    #[test]
    fn test_strategy_name() {
        let strategy = BuyAndHoldStrategy::new();
        assert_eq!(strategy.name(), "BuyAndHold");
    }

    #[test]
    fn test_min_candles_required() {
        let strategy = BuyAndHoldStrategy::new();
        assert_eq!(strategy.min_candles_required(), 1);
    }

    #[test]
    fn test_never_generates_sell() {
        let strategy = BuyAndHoldStrategy::new();

        // Test with different data patterns - should never sell, always buy
        for count in 1..=100 {
            let candles = create_test_candles(count);
            let signal = strategy.generate_signal(&candles).unwrap();
            assert_eq!(
                signal,
                Signal::Buy,
                "BuyAndHold should always return Buy, never Sell or Hold"
            );
        }
    }
}
