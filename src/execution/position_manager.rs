use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

use crate::risk::{CircuitBreakers, TradingState};

#[derive(Debug, Clone, PartialEq)]
pub enum PositionStatus {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExitReason {
    StopLoss,
    TakeProfit,
    TimeStop,
    Manual,
    StrategySell, // Strategy-driven sell signal (e.g., overbought conditions)
}

#[derive(Debug, Clone)]
pub struct Position {
    pub id: Uuid,
    pub token: String,
    pub entry_price: f64,          // Average entry price for accumulated positions
    pub quantity: f64,
    pub entry_time: DateTime<Utc>, // First entry time
    pub stop_loss: f64,            // -8% from (average) entry
    pub take_profit: Option<f64>,  // Trailing stop
    pub trailing_high: f64,        // Track highest price for trailing stop
    pub status: PositionStatus,
    pub realized_pnl: Option<f64>,
    pub exit_price: Option<f64>,
    pub exit_time: Option<DateTime<Utc>>,
    pub exit_reason: Option<ExitReason>,
    pub allow_accumulation: bool, // If true, can add to this position (for DCA)
    pub total_cost_basis: f64,    // Total $ invested (for tracking average price)
}

pub struct PositionManager {
    positions: Vec<Position>,
    circuit_breakers: CircuitBreakers,
    trading_state: TradingState,
    initial_portfolio_value: f64,
    total_pnl: f64, // Track total P&L across all trades
}

impl PositionManager {
    pub fn new(initial_portfolio_value: f64, circuit_breakers: CircuitBreakers) -> Self {
        Self {
            positions: Vec::new(),
            circuit_breakers,
            trading_state: TradingState::new(initial_portfolio_value),
            initial_portfolio_value,
            total_pnl: 0.0,
        }
    }

    /// Create PositionManager and restore from loaded positions
    ///
    /// Recalculates total_pnl from closed positions
    pub fn with_positions(
        initial_portfolio_value: f64,
        circuit_breakers: CircuitBreakers,
        positions: Vec<Position>,
    ) -> Self {
        // Calculate total_pnl from closed positions
        let total_pnl: f64 = positions
            .iter()
            .filter(|p| p.status == PositionStatus::Closed)
            .filter_map(|p| p.realized_pnl)
            .sum();

        tracing::info!(
            "Restored {} positions from persistence (total P&L: ${:.2})",
            positions.len(),
            total_pnl
        );

        Self {
            positions,
            circuit_breakers,
            trading_state: TradingState::new(initial_portfolio_value),
            initial_portfolio_value,
            total_pnl,
        }
    }

    /// Get all positions (both open and closed)
    pub fn all_positions(&self) -> &[Position] {
        &self.positions
    }

    /// Get total realized P&L
    pub fn total_pnl(&self) -> f64 {
        self.total_pnl
    }

    /// Create new position
    ///
    /// # Arguments
    /// * `timestamp` - Optional timestamp for backtesting. If None, uses Utc::now() for live trading
    pub fn open_position(
        &mut self,
        token: String,
        entry_price: f64,
        quantity: f64,
    ) -> anyhow::Result<Uuid> {
        self.open_position_at(token, entry_price, quantity, None, false)
    }

    /// Create new position with explicit timestamp (for backtesting)
    ///
    /// If `allow_accumulation` is true on an existing position, this will add to that position
    /// instead of creating a new one (for DCA strategies).
    ///
    /// # Arguments
    /// * `allow_accumulation` - If true, allows adding to this position later (for DCA)
    pub fn open_position_at(
        &mut self,
        token: String,
        entry_price: f64,
        quantity: f64,
        timestamp: Option<DateTime<Utc>>,
        allow_accumulation: bool,
    ) -> anyhow::Result<Uuid> {
        // Check if we have an open position that allows accumulation
        if let Some(existing_pos) = self.get_open_position(&token) {
            if existing_pos.allow_accumulation {
                // Add to existing position (DCA accumulation)
                let position_id = existing_pos.id;
                let position = self.get_position_mut(position_id)?;

                // Update accumulated values
                position.total_cost_basis += entry_price * quantity;
                position.quantity += quantity;
                position.entry_price = position.total_cost_basis / position.quantity; // Average price

                // Update stop loss and trailing high based on new average entry
                position.stop_loss = position.entry_price * 0.92;
                position.trailing_high = position.trailing_high.max(entry_price);

                tracing::info!(
                    "Accumulated {} @ ${:.2} (avg: ${:.2}, total qty: {:.4})",
                    token, entry_price, position.entry_price, position.quantity
                );

                return Ok(position_id);
            } else {
                anyhow::bail!("Already have open position for {}", token);
            }
        }

        let id = Uuid::new_v4();
        let stop_loss = entry_price * 0.92; // -8% from entry
        let entry_time = timestamp.unwrap_or_else(Utc::now);
        let total_cost_basis = entry_price * quantity;

        let position = Position {
            id,
            token,
            entry_price,
            quantity,
            entry_time,
            stop_loss,
            take_profit: None,
            trailing_high: entry_price,
            status: PositionStatus::Open,
            realized_pnl: None,
            exit_price: None,
            exit_time: None,
            exit_reason: None,
            allow_accumulation, // Use the parameter
            total_cost_basis,
        };

        self.positions.push(position);
        Ok(id)
    }

    /// Check if we have open position for token
    pub fn has_open_position(&self, token: &str) -> bool {
        self.positions
            .iter()
            .any(|p| p.token == token && p.status == PositionStatus::Open)
    }

    /// Get open position for token
    pub fn get_open_position(&self, token: &str) -> Option<&Position> {
        self.positions
            .iter()
            .find(|p| p.token == token && p.status == PositionStatus::Open)
    }

    /// Get mutable reference to position by ID
    fn get_position_mut(&mut self, position_id: Uuid) -> anyhow::Result<&mut Position> {
        self.positions
            .iter_mut()
            .find(|p| p.id == position_id)
            .ok_or_else(|| anyhow::anyhow!("Position not found"))
    }

    /// Get position by ID
    fn get_position(&self, position_id: Uuid) -> anyhow::Result<&Position> {
        self.positions
            .iter()
            .find(|p| p.id == position_id)
            .ok_or_else(|| anyhow::anyhow!("Position not found"))
    }

    /// Calculate current P&L for position
    pub fn calculate_pnl(&self, position_id: Uuid, current_price: f64) -> anyhow::Result<f64> {
        let position = self.get_position(position_id)?;
        let pnl = (current_price - position.entry_price) * position.quantity;
        Ok(pnl)
    }

    /// Update trailing stop if price hit new high
    fn update_trailing_stop(
        &mut self,
        position_id: Uuid,
        current_price: f64,
    ) -> anyhow::Result<()> {
        let position = self.get_position_mut(position_id)?;

        // Activation price: +12% from entry
        let activation_price = position.entry_price * 1.12;

        if current_price >= activation_price {
            // Update high water mark
            if current_price > position.trailing_high {
                position.trailing_high = current_price;
            }

            // Set trailing stop at 5% below high water mark
            position.take_profit = Some(position.trailing_high * 0.95);
        }

        Ok(())
    }

    /// Check if position should exit (returns exit reason if yes)
    /// Check if position should exit (for live trading - uses current time)
    pub fn should_exit(
        &mut self,
        position_id: Uuid,
        current_price: f64,
    ) -> anyhow::Result<Option<ExitReason>> {
        self.should_exit_at(position_id, current_price, None)
    }

    /// Check if position should exit with explicit timestamp (for backtesting)
    pub fn should_exit_at(
        &mut self,
        position_id: Uuid,
        current_price: f64,
        current_time: Option<DateTime<Utc>>,
    ) -> anyhow::Result<Option<ExitReason>> {
        // Update trailing stop first
        self.update_trailing_stop(position_id, current_price)?;

        let position = self.get_position(position_id)?;

        // Check stop loss
        if current_price <= position.stop_loss {
            return Ok(Some(ExitReason::StopLoss));
        }

        // Check take profit (trailing stop)
        if let Some(tp) = position.take_profit {
            if current_price <= tp {
                return Ok(Some(ExitReason::TakeProfit));
            }
        }

        // Check time stop (14 days)
        let now = current_time.unwrap_or_else(Utc::now);
        let days_open = (now - position.entry_time).num_days();
        if days_open >= 14 {
            return Ok(Some(ExitReason::TimeStop));
        }

        Ok(None)
    }

    /// Close position
    ///
    /// # Arguments
    /// * `timestamp` - Optional timestamp for backtesting. If None, uses Utc::now() for live trading
    pub fn close_position(
        &mut self,
        position_id: Uuid,
        exit_price: f64,
        reason: ExitReason,
    ) -> anyhow::Result<()> {
        self.close_position_at(position_id, exit_price, reason, None)
    }

    /// Close position with explicit timestamp (for backtesting)
    pub fn close_position_at(
        &mut self,
        position_id: Uuid,
        exit_price: f64,
        reason: ExitReason,
        timestamp: Option<DateTime<Utc>>,
    ) -> anyhow::Result<()> {
        let position = self.get_position_mut(position_id)?;

        if position.status == PositionStatus::Closed {
            anyhow::bail!("Position already closed");
        }

        let pnl = (exit_price - position.entry_price) * position.quantity;
        let exit_time = timestamp.unwrap_or_else(Utc::now);

        position.status = PositionStatus::Closed;
        position.realized_pnl = Some(pnl);
        position.exit_price = Some(exit_price);
        position.exit_time = Some(exit_time);
        position.exit_reason = Some(reason);

        // Update trading state
        self.trading_state.daily_pnl += pnl;
        self.total_pnl += pnl;
        self.trading_state.daily_trades += 1;

        if pnl < 0.0 {
            self.trading_state.consecutive_losses += 1;
        } else {
            self.trading_state.consecutive_losses = 0;
        }

        Ok(())
    }

    /// Check all open positions for exits (live trading)
    pub fn check_exits(&mut self, prices: &HashMap<String, f64>) -> anyhow::Result<Vec<Uuid>> {
        self.check_exits_at(prices, None)
    }

    /// Check all open positions for exits with explicit timestamp (backtesting)
    pub fn check_exits_at(
        &mut self,
        prices: &HashMap<String, f64>,
        current_time: Option<DateTime<Utc>>,
    ) -> anyhow::Result<Vec<Uuid>> {
        // First, collect position IDs and prices to check
        let positions_to_check: Vec<(Uuid, String, f64)> = self
            .positions
            .iter()
            .filter(|p| p.status == PositionStatus::Open)
            .filter_map(|p| {
                prices
                    .get(&p.token)
                    .map(|&price| (p.id, p.token.clone(), price))
            })
            .collect();

        // Now check each position for exits
        let mut to_close = Vec::new();
        for (position_id, _token, current_price) in positions_to_check {
            if let Some(reason) = self.should_exit_at(position_id, current_price, current_time)? {
                to_close.push((position_id, current_price, reason));
            }
        }

        // Close positions
        let mut closed_ids = Vec::new();
        for (position_id, exit_price, reason) in to_close {
            self.close_position_at(position_id, exit_price, reason, current_time)?;
            closed_ids.push(position_id);
        }

        Ok(closed_ids)
    }

    /// Get portfolio value (cash + position values)
    pub fn portfolio_value(&self, prices: &HashMap<String, f64>) -> anyhow::Result<f64> {
        let mut total_value = self.initial_portfolio_value;

        // Add P&L from closed positions
        total_value += self.total_pnl;

        // Add unrealized P&L from open positions
        for position in self
            .positions
            .iter()
            .filter(|p| p.status == PositionStatus::Open)
        {
            if let Some(&current_price) = prices.get(&position.token) {
                let unrealized_pnl = (current_price - position.entry_price) * position.quantity;
                total_value += unrealized_pnl;
            }
        }

        Ok(total_value)
    }

    /// Get current trading state
    pub fn trading_state(&self) -> &TradingState {
        &self.trading_state
    }

    /// Get circuit breakers configuration
    pub fn circuit_breakers(&self) -> &CircuitBreakers {
        &self.circuit_breakers
    }

    /// Get mutable reference to trading state (for testing)
    #[cfg(test)]
    pub fn trading_state_mut(&mut self) -> &mut TradingState {
        &mut self.trading_state
    }

    /// Get all open positions
    pub fn open_positions(&self) -> Vec<&Position> {
        self.positions
            .iter()
            .filter(|p| p.status == PositionStatus::Open)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_position() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
        let id = pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();

        assert!(pm.has_open_position("SOL"));
        assert_eq!(pm.positions.len(), 1);

        let position = pm.get_position(id).unwrap();
        assert_eq!(position.token, "SOL");
        assert_eq!(position.entry_price, 100.0);
        assert_eq!(position.quantity, 1.0);
        assert_eq!(position.stop_loss, 92.0); // -8%
        assert_eq!(position.status, PositionStatus::Open);
    }

    #[test]
    fn test_prevent_duplicate_positions() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
        pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();

        let result = pm.open_position("SOL".to_string(), 105.0, 1.0);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Already have open position"));
    }

    #[test]
    fn test_pnl_calculation() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
        let id = pm.open_position("SOL".to_string(), 100.0, 2.0).unwrap();

        // Price went from 100 to 110, bought 2 tokens
        let pnl = pm.calculate_pnl(id, 110.0).unwrap();
        assert_eq!(pnl, 20.0); // 2 * (110 - 100)

        // Price went down to 95
        let pnl = pm.calculate_pnl(id, 95.0).unwrap();
        assert_eq!(pnl, -10.0); // 2 * (95 - 100)
    }

    #[test]
    fn test_stop_loss_triggered() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
        let id = pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();

        // Price drops to 91 - should trigger stop loss (-8% is at 92)
        let reason = pm.should_exit(id, 91.0).unwrap();
        assert_eq!(reason, Some(ExitReason::StopLoss));

        // Price at 93 - should not trigger
        let reason = pm.should_exit(id, 93.0).unwrap();
        assert_eq!(reason, None);
    }

    #[test]
    fn test_take_profit_trailing() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
        let id = pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();

        // Price at 110 - not high enough to activate trailing stop (needs +12%)
        let reason = pm.should_exit(id, 110.0).unwrap();
        assert_eq!(reason, None);

        // Price goes up to 115 (+15%) - activates trailing stop
        let reason = pm.should_exit(id, 115.0).unwrap();
        assert_eq!(reason, None); // Still holding, just activated trailing stop

        // Check that trailing stop is set at 95% of 115 = 109.25
        let position = pm.get_position(id).unwrap();
        assert_eq!(position.take_profit, Some(109.25));

        // Price drops to 109 - should trigger take profit
        let reason = pm.should_exit(id, 109.0).unwrap();
        assert_eq!(reason, Some(ExitReason::TakeProfit));
    }

    #[test]
    fn test_trailing_stop_updates_on_new_high() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
        let id = pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();

        // Price goes to 115 - activates trailing stop
        pm.should_exit(id, 115.0).unwrap();
        let position = pm.get_position(id).unwrap();
        assert_eq!(position.take_profit, Some(109.25)); // 95% of 115

        // Price goes to 120 - should update trailing stop
        pm.should_exit(id, 120.0).unwrap();
        let position = pm.get_position(id).unwrap();
        assert_eq!(position.take_profit, Some(114.0)); // 95% of 120

        // Price drops to 118 - still above trailing stop, shouldn't trigger
        let reason = pm.should_exit(id, 118.0).unwrap();
        assert_eq!(reason, None);

        // Price drops to 113 - should trigger
        let reason = pm.should_exit(id, 113.0).unwrap();
        assert_eq!(reason, Some(ExitReason::TakeProfit));
    }

    #[test]
    fn test_time_stop() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
        let id = pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();

        // Manually set entry time to 15 days ago
        {
            let position = pm.get_position_mut(id).unwrap();
            position.entry_time = Utc::now() - chrono::Duration::days(15);
        }

        let reason = pm.should_exit(id, 105.0).unwrap();
        assert_eq!(reason, Some(ExitReason::TimeStop));
    }

    #[test]
    fn test_close_position() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
        let id = pm.open_position("SOL".to_string(), 100.0, 2.0).unwrap();

        // Close position at 110 with take profit
        pm.close_position(id, 110.0, ExitReason::TakeProfit)
            .unwrap();

        let position = pm.get_position(id).unwrap();
        assert_eq!(position.status, PositionStatus::Closed);
        assert_eq!(position.exit_price, Some(110.0));
        assert_eq!(position.realized_pnl, Some(20.0)); // 2 * (110 - 100)
        assert_eq!(position.exit_reason, Some(ExitReason::TakeProfit));

        // Trading state should be updated
        assert_eq!(pm.trading_state.daily_pnl, 20.0);
        assert_eq!(pm.total_pnl, 20.0);
        assert_eq!(pm.trading_state.daily_trades, 1);
        assert_eq!(pm.trading_state.consecutive_losses, 0);
    }

    #[test]
    fn test_close_position_tracks_losses() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
        let id = pm.open_position("SOL".to_string(), 100.0, 2.0).unwrap();

        // Close position at 95 (loss)
        pm.close_position(id, 95.0, ExitReason::StopLoss).unwrap();

        let position = pm.get_position(id).unwrap();
        assert_eq!(position.realized_pnl, Some(-10.0)); // 2 * (95 - 100)

        // Trading state should track loss
        assert_eq!(pm.trading_state.daily_pnl, -10.0);
        assert_eq!(pm.trading_state.consecutive_losses, 1);
    }

    #[test]
    fn test_cannot_close_already_closed_position() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
        let id = pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();

        pm.close_position(id, 110.0, ExitReason::TakeProfit)
            .unwrap();

        let result = pm.close_position(id, 115.0, ExitReason::Manual);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already closed"));
    }

    #[test]
    fn test_check_exits() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());

        // Open two positions
        let id1 = pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();
        let id2 = pm.open_position("JUP".to_string(), 1.0, 100.0).unwrap();

        let mut prices = HashMap::new();
        prices.insert("SOL".to_string(), 91.0); // Below stop loss (92)
        prices.insert("JUP".to_string(), 1.05); // Still good

        let closed = pm.check_exits(&prices).unwrap();

        assert_eq!(closed.len(), 1);
        assert!(closed.contains(&id1));
        assert!(!closed.contains(&id2));

        // Verify SOL position is closed
        let sol_position = pm.get_position(id1).unwrap();
        assert_eq!(sol_position.status, PositionStatus::Closed);
        assert_eq!(sol_position.exit_reason, Some(ExitReason::StopLoss));

        // Verify JUP position is still open
        let jup_position = pm.get_position(id2).unwrap();
        assert_eq!(jup_position.status, PositionStatus::Open);
    }

    #[test]
    fn test_portfolio_value() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());

        // Open position: Buy 2 SOL at 100 each (cost: 200)
        let id = pm.open_position("SOL".to_string(), 100.0, 2.0).unwrap();

        let mut prices = HashMap::new();
        prices.insert("SOL".to_string(), 110.0); // +10 per token = +20 total

        let portfolio_value = pm.portfolio_value(&prices).unwrap();

        // Initial: 10000
        // Unrealized P&L: +20
        // Total: 10020
        assert_eq!(portfolio_value, 10020.0);

        // Close position
        pm.close_position(id, 110.0, ExitReason::TakeProfit)
            .unwrap();

        // Now portfolio should include realized P&L
        let portfolio_value = pm.portfolio_value(&prices).unwrap();
        assert_eq!(portfolio_value, 10020.0);
    }

    #[test]
    fn test_open_positions() {
        let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());

        let id1 = pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();
        let _id2 = pm.open_position("JUP".to_string(), 1.0, 100.0).unwrap();

        assert_eq!(pm.open_positions().len(), 2);

        // Close one position
        pm.close_position(id1, 110.0, ExitReason::TakeProfit)
            .unwrap();

        assert_eq!(pm.open_positions().len(), 1);
        assert_eq!(pm.open_positions()[0].token, "JUP");
    }
}
