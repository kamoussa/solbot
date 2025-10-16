use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Circuit breakers to prevent catastrophic losses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakers {
    pub max_daily_loss_pct: f64,
    pub max_drawdown_pct: f64,
    pub max_consecutive_losses: u32,
    pub max_position_size_pct: f64,
    pub max_daily_trades: u32,
}

impl Default for CircuitBreakers {
    fn default() -> Self {
        Self {
            max_daily_loss_pct: 0.05,    // -5% daily
            max_drawdown_pct: 0.20,      // -20% from peak
            max_consecutive_losses: 5,   // 5 losses in a row
            max_position_size_pct: 0.05, // 5% max per position
            max_daily_trades: 10,        // Max 10 trades per day
        }
    }
}

#[derive(Debug, Clone)]
pub struct TradingState {
    pub portfolio_value: f64,
    pub peak_portfolio_value: f64,
    pub daily_pnl: f64,
    pub consecutive_losses: u32,
    pub daily_trades: u32,
    pub last_reset: DateTime<Utc>,
}

impl TradingState {
    pub fn new(initial_capital: f64) -> Self {
        Self {
            portfolio_value: initial_capital,
            peak_portfolio_value: initial_capital,
            daily_pnl: 0.0,
            consecutive_losses: 0,
            daily_trades: 0,
            last_reset: Utc::now(),
        }
    }

    pub fn update_portfolio_value(&mut self, new_value: f64) {
        self.portfolio_value = new_value;
        if new_value > self.peak_portfolio_value {
            self.peak_portfolio_value = new_value;
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CircuitBreakerTrip {
    DailyLoss,
    MaxDrawdown,
    ConsecutiveLosses,
    DailyTradeLimit,
}

impl CircuitBreakers {
    pub fn check(&self, state: &TradingState) -> Result<(), CircuitBreakerTrip> {
        // Check daily loss
        let daily_loss_pct = state.daily_pnl / state.portfolio_value;
        if daily_loss_pct < -self.max_daily_loss_pct {
            return Err(CircuitBreakerTrip::DailyLoss);
        }

        // Check drawdown
        let drawdown =
            (state.peak_portfolio_value - state.portfolio_value) / state.peak_portfolio_value;
        if drawdown > self.max_drawdown_pct {
            return Err(CircuitBreakerTrip::MaxDrawdown);
        }

        // Check consecutive losses
        if state.consecutive_losses >= self.max_consecutive_losses {
            return Err(CircuitBreakerTrip::ConsecutiveLosses);
        }

        // Check daily trade limit
        if state.daily_trades >= self.max_daily_trades {
            return Err(CircuitBreakerTrip::DailyTradeLimit);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_daily_loss() {
        let breakers = CircuitBreakers::default();
        let mut state = TradingState::new(10000.0);

        // Simulate -6% daily loss
        state.daily_pnl = -600.0;
        state.portfolio_value = 9400.0;

        let result = breakers.check(&state);
        assert_eq!(result, Err(CircuitBreakerTrip::DailyLoss));
    }

    #[test]
    fn test_circuit_breaker_drawdown() {
        let breakers = CircuitBreakers::default();
        let mut state = TradingState::new(10000.0);

        // Peak was 12000, now 9000 = 25% drawdown
        state.peak_portfolio_value = 12000.0;
        state.portfolio_value = 9000.0;

        let result = breakers.check(&state);
        assert_eq!(result, Err(CircuitBreakerTrip::MaxDrawdown));
    }

    #[test]
    fn test_circuit_breaker_consecutive_losses() {
        let breakers = CircuitBreakers::default();
        let mut state = TradingState::new(10000.0);

        state.consecutive_losses = 5;

        let result = breakers.check(&state);
        assert_eq!(result, Err(CircuitBreakerTrip::ConsecutiveLosses));
    }

    #[test]
    fn test_circuit_breaker_ok() {
        let breakers = CircuitBreakers::default();
        let state = TradingState::new(10000.0);

        let result = breakers.check(&state);
        assert!(result.is_ok());
    }
}
