use std::sync::{Arc, Mutex};

use crate::execution::{ExitReason, PositionManager};
use crate::models::Signal;

#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionAction {
    Execute { quantity: f64 },
    Skip,
    Close {
        position_id: uuid::Uuid,
        exit_reason: ExitReason,
    },
}

#[derive(Debug, Clone)]
pub struct ExecutionDecision {
    pub action: ExecutionAction,
    pub reason: String,
}

pub struct Executor {
    position_manager: Arc<Mutex<PositionManager>>,
}

impl Executor {
    pub fn new(position_manager: Arc<Mutex<PositionManager>>) -> Self {
        Self { position_manager }
    }

    /// Process a signal and decide what to do
    pub fn process_signal(
        &mut self,
        signal: &Signal,
        token: &str,
        current_price: f64,
    ) -> anyhow::Result<ExecutionDecision> {
        let pm = self.position_manager.lock().unwrap();

        match signal {
            Signal::Buy => {
                // Check 1: Do we already have this token?
                if pm.has_open_position(token) {
                    return Ok(ExecutionDecision {
                        action: ExecutionAction::Skip,
                        reason: "Already have open position".to_string(),
                    });
                }

                // Check 2: Circuit breakers
                if let Err(trip) = pm.circuit_breakers().check(pm.trading_state()) {
                    return Ok(ExecutionDecision {
                        action: ExecutionAction::Skip,
                        reason: format!("Circuit breaker: {:?}", trip),
                    });
                }

                // Check 3: Calculate size (5% of portfolio)
                let max_position_pct = pm.circuit_breakers().max_position_size_pct;
                let quantity =
                    self.calculate_position_size(&pm, current_price, max_position_pct)?;

                // Execute
                Ok(ExecutionDecision {
                    action: ExecutionAction::Execute { quantity },
                    reason: "Buy signal with available capital".to_string(),
                })
            }

            Signal::Sell => {
                // Check: Do we have this token?
                if let Some(position) = pm.get_open_position(token) {
                    // Calculate unrealized P&L %
                    let unrealized_pnl_pct =
                        (current_price - position.entry_price) / position.entry_price;

                    // Only sell on technical signal if we're up at least 5%
                    // This ensures technical exits are for profit-taking, not loss-cutting
                    if unrealized_pnl_pct >= 0.05 {
                        Ok(ExecutionDecision {
                            action: ExecutionAction::Close {
                                position_id: position.id,
                                exit_reason: ExitReason::StrategySell,
                            },
                            reason: format!(
                                "Sell signal with {:.1}% profit (>5% threshold)",
                                unrealized_pnl_pct * 100.0
                            ),
                        })
                    } else {
                        Ok(ExecutionDecision {
                            action: ExecutionAction::Skip,
                            reason: format!(
                                "Sell signal ignored - only {:.1}% profit (need >5%)",
                                unrealized_pnl_pct * 100.0
                            ),
                        })
                    }
                } else {
                    Ok(ExecutionDecision {
                        action: ExecutionAction::Skip,
                        reason: "No position to sell".to_string(),
                    })
                }
            }

            Signal::Hold => {
                // Hold means do nothing
                Ok(ExecutionDecision {
                    action: ExecutionAction::Skip,
                    reason: "Hold signal".to_string(),
                })
            }
        }
    }

    /// Calculate position size based on portfolio value and risk limits
    fn calculate_position_size(
        &self,
        pm: &PositionManager,
        current_price: f64,
        max_position_pct: f64,
    ) -> anyhow::Result<f64> {
        // Get current portfolio value
        let portfolio_value = pm.trading_state().portfolio_value;

        // Calculate max position value (e.g., 5% of portfolio)
        let max_position_value = portfolio_value * max_position_pct;

        // Calculate quantity based on current price
        let quantity = max_position_value / current_price;

        Ok(quantity)
    }

    /// Get reference to position manager (for testing)
    #[cfg(test)]
    pub fn position_manager(&self) -> Arc<Mutex<PositionManager>> {
        self.position_manager.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::PositionManager;
    use crate::models::Signal;
    use crate::risk::CircuitBreakers;

    #[test]
    fn test_skip_buy_when_already_positioned() {
        let pm = Arc::new(Mutex::new(PositionManager::new(
            10000.0,
            CircuitBreakers::default(),
        )));
        pm.lock()
            .unwrap()
            .open_position("SOL".to_string(), 100.0, 1.0)
            .unwrap();

        let mut executor = Executor::new(pm);
        let decision = executor.process_signal(&Signal::Buy, "SOL", 105.0).unwrap();

        assert!(matches!(decision.action, ExecutionAction::Skip));
        assert!(decision.reason.contains("Already have"));
    }

    #[test]
    fn test_skip_sell_when_no_position() {
        let pm = Arc::new(Mutex::new(PositionManager::new(
            10000.0,
            CircuitBreakers::default(),
        )));
        let mut executor = Executor::new(pm);

        let decision = executor
            .process_signal(&Signal::Sell, "SOL", 100.0)
            .unwrap();

        assert!(matches!(decision.action, ExecutionAction::Skip));
        assert!(decision.reason.contains("No position"));
    }

    #[test]
    fn test_execute_buy_when_valid() {
        let pm = Arc::new(Mutex::new(PositionManager::new(
            10000.0,
            CircuitBreakers::default(),
        )));
        let mut executor = Executor::new(pm);

        let decision = executor.process_signal(&Signal::Buy, "SOL", 100.0).unwrap();

        assert!(matches!(decision.action, ExecutionAction::Execute { .. }));
        if let ExecutionAction::Execute { quantity } = decision.action {
            // Portfolio = 10000, max position = 5% = 500
            // Price = 100, so quantity = 500 / 100 = 5
            assert_eq!(quantity, 5.0);
        }
    }

    #[test]
    fn test_position_sizing() {
        let pm = Arc::new(Mutex::new(PositionManager::new(
            10000.0,
            CircuitBreakers::default(),
        )));
        let executor = Executor::new(pm.clone());

        // Portfolio = 10000, max position = 5% = 500
        // Price = 100, so quantity = 500 / 100 = 5
        let quantity = {
            let pm_lock = pm.lock().unwrap();
            executor
                .calculate_position_size(&pm_lock, 100.0, 0.05)
                .unwrap()
        };
        assert_eq!(quantity, 5.0);

        // Price = 50, so quantity = 500 / 50 = 10
        let quantity = {
            let pm_lock = pm.lock().unwrap();
            executor
                .calculate_position_size(&pm_lock, 50.0, 0.05)
                .unwrap()
        };
        assert_eq!(quantity, 10.0);
    }

    #[test]
    fn test_circuit_breaker_blocks_execution() {
        let breakers = CircuitBreakers {
            max_daily_loss_pct: 0.05,
            ..Default::default()
        };

        let pm = Arc::new(Mutex::new(PositionManager::new(10000.0, breakers)));

        // Trigger circuit breaker by setting daily loss to -6%
        {
            let mut pm_lock = pm.lock().unwrap();
            pm_lock.trading_state_mut().daily_pnl = -600.0; // -6%
        }

        let mut executor = Executor::new(pm);
        let decision = executor.process_signal(&Signal::Buy, "SOL", 100.0).unwrap();

        assert!(matches!(decision.action, ExecutionAction::Skip));
        assert!(decision.reason.contains("Circuit breaker"));
    }

    #[test]
    fn test_hold_signal_skips() {
        let pm = Arc::new(Mutex::new(PositionManager::new(
            10000.0,
            CircuitBreakers::default(),
        )));
        let mut executor = Executor::new(pm);

        let decision = executor
            .process_signal(&Signal::Hold, "SOL", 100.0)
            .unwrap();

        assert!(matches!(decision.action, ExecutionAction::Skip));
        assert!(decision.reason.contains("Hold"));
    }

    #[test]
    fn test_sell_signal_closes_position_when_profitable() {
        let pm = Arc::new(Mutex::new(PositionManager::new(
            10000.0,
            CircuitBreakers::default(),
        )));

        // Open a position first at $100
        let position_id = pm
            .lock()
            .unwrap()
            .open_position("SOL".to_string(), 100.0, 5.0)
            .unwrap();

        let mut executor = Executor::new(pm);

        // Process sell signal at $110 (10% profit, > 5% threshold)
        let decision = executor
            .process_signal(&Signal::Sell, "SOL", 110.0)
            .unwrap();

        assert!(matches!(
            decision.action,
            ExecutionAction::Close { position_id: id, exit_reason: _ } if id == position_id
        ));
        assert!(decision.reason.contains("profit"));
    }

    #[test]
    fn test_sell_signal_skips_when_profit_below_threshold() {
        let pm = Arc::new(Mutex::new(PositionManager::new(
            10000.0,
            CircuitBreakers::default(),
        )));

        // Open position at $100
        pm.lock()
            .unwrap()
            .open_position("SOL".to_string(), 100.0, 5.0)
            .unwrap();

        let mut executor = Executor::new(pm);

        // Sell signal at $103 (only 3% profit, < 5% threshold)
        let decision = executor
            .process_signal(&Signal::Sell, "SOL", 103.0)
            .unwrap();

        assert!(matches!(decision.action, ExecutionAction::Skip));
        assert!(decision.reason.contains("only 3.0% profit"));
    }

    #[test]
    fn test_sell_signal_skips_when_losing() {
        let pm = Arc::new(Mutex::new(PositionManager::new(
            10000.0,
            CircuitBreakers::default(),
        )));

        // Open position at $100
        pm.lock()
            .unwrap()
            .open_position("SOL".to_string(), 100.0, 5.0)
            .unwrap();

        let mut executor = Executor::new(pm);

        // Sell signal at $98 (losing -2%)
        let decision = executor
            .process_signal(&Signal::Sell, "SOL", 98.0)
            .unwrap();

        assert!(matches!(decision.action, ExecutionAction::Skip));
        assert!(decision.reason.contains("-2.0% profit"));
    }

    #[test]
    fn test_full_trade_cycle() {
        let pm = Arc::new(Mutex::new(PositionManager::new(
            10000.0,
            CircuitBreakers::default(),
        )));
        let mut executor = Executor::new(pm.clone());

        // Buy signal - should execute
        let decision = executor.process_signal(&Signal::Buy, "SOL", 100.0).unwrap();
        assert!(matches!(decision.action, ExecutionAction::Execute { .. }));

        // Create position (simulating execution)
        pm.lock()
            .unwrap()
            .open_position("SOL".to_string(), 100.0, 5.0)
            .unwrap();

        // Buy signal again - should skip (already have position)
        let decision = executor.process_signal(&Signal::Buy, "SOL", 105.0).unwrap();
        assert!(matches!(decision.action, ExecutionAction::Skip));

        // Sell signal - should close position
        let decision = executor
            .process_signal(&Signal::Sell, "SOL", 110.0)
            .unwrap();
        assert!(matches!(decision.action, ExecutionAction::Close { .. }));

        // Close the position (simulating execution)
        if let ExecutionAction::Close { position_id, exit_reason } = decision.action {
            pm.lock()
                .unwrap()
                .close_position(position_id, 110.0, exit_reason)
                .unwrap();
        }

        // Sell signal again - should skip (no position)
        let decision = executor
            .process_signal(&Signal::Sell, "SOL", 110.0)
            .unwrap();
        assert!(matches!(decision.action, ExecutionAction::Skip));

        // Buy signal again - should execute (no position)
        let decision = executor.process_signal(&Signal::Buy, "SOL", 110.0).unwrap();
        assert!(matches!(decision.action, ExecutionAction::Execute { .. }));
    }
}
