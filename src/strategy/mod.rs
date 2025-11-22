// Trading strategy module
pub mod buy_and_hold;
pub mod dca;
pub mod mean_reversion;
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

    /// Whether to skip automatic exit conditions (stop loss, trailing stop, time stop)
    ///
    /// Default: false (enforce automatic exits for risk management)
    ///
    /// Set to true for baseline strategies like pure buy-and-hold that should
    /// hold positions indefinitely without forced exits.
    fn skip_automatic_exits(&self) -> bool {
        false
    }

    /// Whether this strategy supports position accumulation (adding to existing positions)
    ///
    /// Default: false (one position at a time)
    ///
    /// Set to true for DCA-style strategies that periodically add to positions
    /// rather than closing and reopening.
    fn supports_accumulation(&self) -> bool {
        false
    }
}
