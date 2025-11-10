// Technical indicators module
// Implements RSI, MA, ADX, ATR for technical analysis

pub mod adx;
pub mod atr;
pub mod market_analysis;
pub mod moving_average;
pub mod rsi;

pub use adx::calculate_adx;
pub use atr::{calculate_atr, calculate_atr_series, is_atr_spike};
pub use market_analysis::{
    analyze_market_structure, calculate_average_volume, calculate_volume_direction_ratio,
    is_rsi_rising, is_volume_spike, MarketStructure,
};
pub use moving_average::{calculate_ema, calculate_sma};
pub use rsi::calculate_rsi;
