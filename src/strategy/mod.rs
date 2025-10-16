// Trading strategy module
pub mod momentum;
pub mod signals;

use crate::models::{Candle, Signal};
use crate::Result;

/// Base trait for all trading strategies
pub trait Strategy: Send + Sync {
    /// Generate a trading signal based on market data
    fn generate_signal(&self, candles: &[Candle]) -> Result<Signal>;

    /// Get strategy name
    fn name(&self) -> &str;

    /// Minimum candles required for this strategy
    fn min_candles_required(&self) -> usize;

    /// Number of candles required for lookback at given polling interval
    /// Default implementation uses min_candles_required
    fn samples_needed(&self, _poll_interval_minutes: u64) -> usize {
        self.min_candles_required()
    }

    /// Lookback period in hours for this strategy
    fn lookback_hours(&self) -> u64 {
        24 // Default 24 hours
    }
}
