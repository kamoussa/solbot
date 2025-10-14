// Core modules
pub mod api;
pub mod models;
pub mod indicators;
pub mod strategy;
pub mod execution;
pub mod risk;
pub mod db;
pub mod llm;
pub mod persistence;

// Re-export commonly used types
pub use models::*;
pub use api::*;
pub use strategy::Strategy;

// Error handling
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
