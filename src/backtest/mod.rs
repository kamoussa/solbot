pub mod metrics;
pub mod runner;
pub mod synthetic;

pub use metrics::{BacktestMetrics, TradeRecord};
pub use runner::BacktestRunner;
pub use synthetic::{MarketScenario, SyntheticDataGenerator};
