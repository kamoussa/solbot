// Core modules
pub mod api;
pub mod backtest;
pub mod db;
pub mod discovery;
pub mod execution;
pub mod indicators;
pub mod llm;
pub mod models;
pub mod persistence;
pub mod risk;
pub mod strategy;

// Re-export commonly used types
pub use api::*;
pub use models::*;
pub use strategy::Strategy;

// Error handling
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
