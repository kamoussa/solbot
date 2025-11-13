// Market regime detection module
pub mod detector;
pub mod llm_detector;
pub mod strategy_selector;

pub use detector::{CompositeRegimeDetector, MarketRegime, RegimeDetector};
pub use llm_detector::{LLMRegimeDetector, LLMRegimeResponse};
pub use strategy_selector::{LLMStrategySelector, LLMStrategyResponse, Strategy};
