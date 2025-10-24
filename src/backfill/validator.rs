use chrono::Utc;

use crate::models::Candle;
use crate::Result;

/// Validates OHLC candle data for sanity and correctness
pub struct CandleValidator;

impl CandleValidator {
    pub fn new() -> Self {
        Self
    }

    /// Validate a candle for correctness
    pub fn validate(&self, candle: &Candle) -> Result<()> {
        self.validate_prices(candle)?;
        self.validate_timestamp(candle)?;
        self.validate_ohlc_relationship(candle)?;
        Ok(())
    }

    /// Validate that all prices are positive
    fn validate_prices(&self, candle: &Candle) -> Result<()> {
        if candle.open <= 0.0 {
            return Err(format!("Invalid open price: {}", candle.open).into());
        }
        if candle.high <= 0.0 {
            return Err(format!("Invalid high price: {}", candle.high).into());
        }
        if candle.low <= 0.0 {
            return Err(format!("Invalid low price: {}", candle.low).into());
        }
        if candle.close <= 0.0 {
            return Err(format!("Invalid close price: {}", candle.close).into());
        }
        // Volume can be 0.0 (for backfilled data)
        if candle.volume < 0.0 {
            return Err(format!("Invalid volume: {}", candle.volume).into());
        }
        Ok(())
    }

    /// Validate that timestamp is not in the future
    fn validate_timestamp(&self, candle: &Candle) -> Result<()> {
        let now = Utc::now();
        if candle.timestamp > now {
            return Err(format!(
                "Candle timestamp is in the future: {} (now: {})",
                candle.timestamp, now
            )
            .into());
        }
        Ok(())
    }

    /// Validate OHLC relationships (high >= low, etc.)
    fn validate_ohlc_relationship(&self, candle: &Candle) -> Result<()> {
        // High must be >= low
        if candle.high < candle.low {
            return Err(format!("High ({}) is less than low ({})", candle.high, candle.low).into());
        }

        // High must be >= open and close
        if candle.high < candle.open {
            return Err(
                format!("High ({}) is less than open ({})", candle.high, candle.open).into(),
            );
        }
        if candle.high < candle.close {
            return Err(format!(
                "High ({}) is less than close ({})",
                candle.high, candle.close
            )
            .into());
        }

        // Low must be <= open and close
        if candle.low > candle.open {
            return Err(format!(
                "Low ({}) is greater than open ({})",
                candle.low, candle.open
            )
            .into());
        }
        if candle.low > candle.close {
            return Err(format!(
                "Low ({}) is greater than close ({})",
                candle.low, candle.close
            )
            .into());
        }

        Ok(())
    }
}

impl Default for CandleValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_valid_candle() -> Candle {
        Candle {
            token: "SOL".to_string(),
            timestamp: Utc::now() - Duration::hours(1),
            open: 100.0,
            high: 102.0,
            low: 99.0,
            close: 101.0,
            volume: 1000000.0,
        }
    }

    #[test]
    fn test_validate_valid_candle() {
        let validator = CandleValidator::new();
        let candle = make_valid_candle();

        assert!(validator.validate(&candle).is_ok());
    }

    #[test]
    fn test_validate_negative_open() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.open = -100.0;

        let result = validator.validate(&candle);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid open price"));
    }

    #[test]
    fn test_validate_negative_high() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.high = -102.0;

        let result = validator.validate(&candle);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid high price"));
    }

    #[test]
    fn test_validate_negative_low() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.low = -99.0;

        let result = validator.validate(&candle);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid low price"));
    }

    #[test]
    fn test_validate_negative_close() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.close = -101.0;

        let result = validator.validate(&candle);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid close price"));
    }

    #[test]
    fn test_validate_zero_volume_allowed() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.volume = 0.0;

        // Should be OK (backfilled candles have 0 volume)
        assert!(validator.validate(&candle).is_ok());
    }

    #[test]
    fn test_validate_negative_volume() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.volume = -1000.0;

        let result = validator.validate(&candle);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid volume"));
    }

    #[test]
    fn test_validate_high_less_than_low() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.high = 98.0; // Less than low (99.0)
        candle.low = 99.0;

        let result = validator.validate(&candle);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("High") && err_msg.contains("less than low"));
    }

    #[test]
    fn test_validate_high_less_than_open() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.open = 105.0;
        candle.high = 104.0; // Less than open

        let result = validator.validate(&candle);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("High") && err_msg.contains("less than open"));
    }

    #[test]
    fn test_validate_high_less_than_close() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.close = 105.0;
        candle.high = 104.0; // Less than close

        let result = validator.validate(&candle);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("High") && err_msg.contains("less than close"));
    }

    #[test]
    fn test_validate_low_greater_than_open() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.open = 95.0;
        candle.low = 96.0; // Greater than open

        let result = validator.validate(&candle);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Low") && err_msg.contains("greater than open"));
    }

    #[test]
    fn test_validate_low_greater_than_close() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.close = 95.0;
        candle.low = 96.0; // Greater than close

        let result = validator.validate(&candle);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Low") && err_msg.contains("greater than close"));
    }

    #[test]
    fn test_validate_future_timestamp() {
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.timestamp = Utc::now() + Duration::hours(10); // In future

        let result = validator.validate(&candle);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("future"));
    }

    #[test]
    fn test_validate_all_prices_equal() {
        // Edge case: all prices the same (valid, e.g., no trading activity)
        let validator = CandleValidator::new();
        let mut candle = make_valid_candle();
        candle.open = 100.0;
        candle.high = 100.0;
        candle.low = 100.0;
        candle.close = 100.0;

        assert!(validator.validate(&candle).is_ok());
    }
}
