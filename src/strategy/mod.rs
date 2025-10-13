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
}
