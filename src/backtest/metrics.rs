use crate::execution::position_manager::Position;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Record of a single trade for analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    pub entry_time: DateTime<Utc>,
    pub exit_time: DateTime<Utc>,
    pub entry_price: f64,
    pub exit_price: f64,
    pub quantity: f64,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub holding_period_minutes: i64,
    pub transaction_cost: f64,  // Total fees for this trade (entry + exit)
    pub net_pnl: f64,            // P&L after transaction costs
}

impl TradeRecord {
    pub fn from_position(position: &Position, transaction_cost: f64) -> Option<Self> {
        if let (Some(exit_price), Some(exit_time), Some(realized_pnl)) = (
            position.exit_price,
            position.exit_time,
            position.realized_pnl,
        ) {
            let holding_period = (exit_time - position.entry_time).num_minutes();
            let pnl_pct = ((exit_price - position.entry_price) / position.entry_price) * 100.0;
            let net_pnl = realized_pnl - transaction_cost;

            Some(Self {
                entry_time: position.entry_time,
                exit_time,
                entry_price: position.entry_price,
                exit_price,
                quantity: position.quantity,
                pnl: realized_pnl,
                pnl_pct,
                holding_period_minutes: holding_period,
                transaction_cost,
                net_pnl,
            })
        } else {
            None
        }
    }
}

/// Complete backtest performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestMetrics {
    // P&L Metrics
    pub total_pnl: f64,
    pub total_return_pct: f64,
    pub initial_portfolio_value: f64,
    pub final_portfolio_value: f64,

    // Trade Statistics
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: f64,

    // P&L Distribution
    pub avg_win: f64,
    pub avg_loss: f64,
    pub largest_win: f64,
    pub largest_loss: f64,
    pub profit_factor: f64, // Total wins / Total losses

    // Risk Metrics
    pub max_drawdown: f64,
    pub max_drawdown_pct: f64,
    pub sharpe_ratio: f64,

    // Holding Period
    pub avg_holding_period_minutes: f64,
    pub max_holding_period_minutes: i64,
    pub min_holding_period_minutes: i64,

    // Transaction Costs
    pub total_transaction_costs: f64,
    pub net_pnl: f64,  // total_pnl - total_transaction_costs
    pub net_return_pct: f64,

    // Circuit Breakers
    pub circuit_breaker_hits: usize,

    // Trade Records
    pub trades: Vec<TradeRecord>,
}

impl BacktestMetrics {
    /// Calculate metrics from completed positions
    pub fn from_positions(
        positions: Vec<Position>,
        initial_portfolio_value: f64,
        final_portfolio_value: f64,
        circuit_breaker_hits: usize,
        transaction_cost_pct: f64,  // Round-trip cost as percentage (e.g., 0.01 = 1%)
    ) -> Self {
        let trades: Vec<TradeRecord> = positions
            .iter()
            .filter_map(|pos| {
                // Calculate transaction cost for this trade (entry + exit)
                let cost = if let Some(exit_price) = pos.exit_price {
                    let entry_cost = pos.entry_price * pos.quantity * (transaction_cost_pct / 2.0);
                    let exit_cost = exit_price * pos.quantity * (transaction_cost_pct / 2.0);
                    entry_cost + exit_cost
                } else {
                    0.0
                };
                TradeRecord::from_position(pos, cost)
            })
            .collect();

        let total_trades = trades.len();

        if total_trades == 0 {
            return Self::empty(
                initial_portfolio_value,
                final_portfolio_value,
                circuit_breaker_hits,
            );
        }

        // P&L calculations
        let total_pnl: f64 = trades.iter().map(|t| t.pnl).sum();
        let total_return_pct =
            ((final_portfolio_value - initial_portfolio_value) / initial_portfolio_value) * 100.0;

        // Win/Loss statistics
        let winning_trades: Vec<&TradeRecord> = trades.iter().filter(|t| t.pnl > 0.0).collect();
        let losing_trades: Vec<&TradeRecord> = trades.iter().filter(|t| t.pnl <= 0.0).collect();

        let winning_count = winning_trades.len();
        let losing_count = losing_trades.len();
        let win_rate = if total_trades > 0 {
            (winning_count as f64 / total_trades as f64) * 100.0
        } else {
            0.0
        };

        // Average wins/losses
        let total_wins: f64 = winning_trades.iter().map(|t| t.pnl).sum();
        let total_losses: f64 = losing_trades.iter().map(|t| t.pnl.abs()).sum();

        let avg_win = if winning_count > 0 {
            total_wins / winning_count as f64
        } else {
            0.0
        };

        let avg_loss = if losing_count > 0 {
            total_losses / losing_count as f64
        } else {
            0.0
        };

        // Largest win/loss
        let largest_win = winning_trades
            .iter()
            .map(|t| t.pnl)
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);

        let largest_loss = losing_trades
            .iter()
            .map(|t| t.pnl)
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);

        // Profit factor
        let profit_factor = if total_losses > 0.0 {
            total_wins / total_losses
        } else if total_wins > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };

        // Calculate drawdown
        let (max_drawdown, max_drawdown_pct) =
            Self::calculate_drawdown(&trades, initial_portfolio_value);

        // Sharpe ratio (simplified - using trade returns)
        let sharpe_ratio = Self::calculate_sharpe_ratio(&trades);

        // Holding periods
        let holding_periods: Vec<i64> = trades.iter().map(|t| t.holding_period_minutes).collect();
        let avg_holding_period_minutes = if !holding_periods.is_empty() {
            holding_periods.iter().sum::<i64>() as f64 / holding_periods.len() as f64
        } else {
            0.0
        };

        let max_holding_period_minutes = *holding_periods.iter().max().unwrap_or(&0);
        let min_holding_period_minutes = *holding_periods.iter().min().unwrap_or(&0);

        // Transaction costs
        let total_transaction_costs: f64 = trades.iter().map(|t| t.transaction_cost).sum();
        let net_pnl = total_pnl - total_transaction_costs;
        let net_return_pct =
            ((final_portfolio_value - total_transaction_costs - initial_portfolio_value) / initial_portfolio_value) * 100.0;

        Self {
            total_pnl,
            total_return_pct,
            initial_portfolio_value,
            final_portfolio_value,
            total_trades,
            winning_trades: winning_count,
            losing_trades: losing_count,
            win_rate,
            avg_win,
            avg_loss,
            largest_win,
            largest_loss,
            profit_factor,
            max_drawdown,
            max_drawdown_pct,
            sharpe_ratio,
            avg_holding_period_minutes,
            max_holding_period_minutes,
            min_holding_period_minutes,
            total_transaction_costs,
            net_pnl,
            net_return_pct,
            circuit_breaker_hits,
            trades,
        }
    }

    /// Empty metrics for when no trades occurred
    fn empty(
        initial_portfolio_value: f64,
        final_portfolio_value: f64,
        circuit_breaker_hits: usize,
    ) -> Self {
        Self {
            total_pnl: 0.0,
            total_return_pct: 0.0,
            initial_portfolio_value,
            final_portfolio_value,
            total_trades: 0,
            winning_trades: 0,
            losing_trades: 0,
            win_rate: 0.0,
            avg_win: 0.0,
            avg_loss: 0.0,
            largest_win: 0.0,
            largest_loss: 0.0,
            profit_factor: 0.0,
            max_drawdown: 0.0,
            max_drawdown_pct: 0.0,
            sharpe_ratio: 0.0,
            avg_holding_period_minutes: 0.0,
            max_holding_period_minutes: 0,
            min_holding_period_minutes: 0,
            total_transaction_costs: 0.0,
            net_pnl: 0.0,
            net_return_pct: 0.0,
            circuit_breaker_hits,
            trades: vec![],
        }
    }

    /// Calculate maximum drawdown from trades
    fn calculate_drawdown(trades: &[TradeRecord], initial_value: f64) -> (f64, f64) {
        let mut peak = initial_value;
        let mut max_dd = 0.0;
        let mut current_value = initial_value;

        for trade in trades {
            current_value += trade.pnl;

            if current_value > peak {
                peak = current_value;
            }

            let drawdown = peak - current_value;
            if drawdown > max_dd {
                max_dd = drawdown;
            }
        }

        let max_dd_pct = if peak > 0.0 {
            (max_dd / peak) * 100.0
        } else {
            0.0
        };

        (max_dd, max_dd_pct)
    }

    /// Calculate Sharpe ratio (simplified)
    /// Assumes risk-free rate of 0 for simplicity
    fn calculate_sharpe_ratio(trades: &[TradeRecord]) -> f64 {
        if trades.is_empty() {
            return 0.0;
        }

        let returns: Vec<f64> = trades.iter().map(|t| t.pnl_pct).collect();

        let mean_return = returns.iter().sum::<f64>() / returns.len() as f64;

        // Calculate standard deviation
        let variance = returns
            .iter()
            .map(|r| {
                let diff = r - mean_return;
                diff * diff
            })
            .sum::<f64>()
            / returns.len() as f64;

        let std_dev = variance.sqrt();

        if std_dev > 0.0 {
            mean_return / std_dev
        } else {
            0.0
        }
    }

    /// Print a formatted report to stdout
    pub fn print_report(&self) {
        println!("\n╔═══════════════════════════════════════════════════════╗");
        println!("║              BACKTEST PERFORMANCE REPORT              ║");
        println!("╚═══════════════════════════════════════════════════════╝\n");

        println!("📊 P&L SUMMARY");
        println!(
            "  Initial Portfolio:     ${:.2}",
            self.initial_portfolio_value
        );
        println!(
            "  Final Portfolio:       ${:.2}",
            self.final_portfolio_value
        );
        println!(
            "  Gross P&L:             ${:.2} ({:+.2}%)",
            self.total_pnl, self.total_return_pct
        );
        println!(
            "  Transaction Costs:     ${:.2}",
            self.total_transaction_costs
        );
        println!(
            "  Net P&L:               ${:.2} ({:+.2}%)",
            self.net_pnl, self.net_return_pct
        );

        println!("\n📈 TRADE STATISTICS");
        println!("  Total Trades:          {}", self.total_trades);
        println!(
            "  Winning Trades:        {} ({:.1}%)",
            self.winning_trades, self.win_rate
        );
        println!("  Losing Trades:         {}", self.losing_trades);

        if self.total_trades > 0 {
            println!("\n💰 WIN/LOSS ANALYSIS");
            println!("  Average Win:           ${:.2}", self.avg_win);
            println!("  Average Loss:          ${:.2}", self.avg_loss);
            println!("  Largest Win:           ${:.2}", self.largest_win);
            println!("  Largest Loss:          ${:.2}", self.largest_loss);
            println!("  Profit Factor:         {:.2}", self.profit_factor);

            println!("\n⚠️  RISK METRICS");
            println!(
                "  Max Drawdown:          ${:.2} ({:.2}%)",
                self.max_drawdown, self.max_drawdown_pct
            );
            println!("  Sharpe Ratio:          {:.2}", self.sharpe_ratio);

            println!("\n⏱️  HOLDING PERIODS");
            println!(
                "  Average:               {:.1} minutes ({:.1} hours)",
                self.avg_holding_period_minutes,
                self.avg_holding_period_minutes / 60.0
            );
            println!(
                "  Max:                   {} minutes ({:.1} hours)",
                self.max_holding_period_minutes,
                self.max_holding_period_minutes as f64 / 60.0
            );
            println!(
                "  Min:                   {} minutes ({:.1} hours)",
                self.min_holding_period_minutes,
                self.min_holding_period_minutes as f64 / 60.0
            );
        }

        println!("\n🛑 CIRCUIT BREAKERS");
        println!(
            "  Triggered:             {} times",
            self.circuit_breaker_hits
        );

        println!("\n═══════════════════════════════════════════════════════\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::position_manager::PositionStatus;

    fn create_test_position(pnl: f64, holding_minutes: i64) -> Position {
        let entry_time = Utc::now();
        let exit_time = entry_time + chrono::Duration::minutes(holding_minutes);

        let entry_price = 100.0;
        let quantity = 1.0;
        let exit_price = entry_price + (pnl / quantity);

        Position {
            id: uuid::Uuid::new_v4(),
            token: "TEST".to_string(),
            entry_price,
            quantity,
            entry_time,
            stop_loss: entry_price * 0.92,
            take_profit: Some(entry_price * 1.12),
            trailing_high: entry_price,
            status: PositionStatus::Closed,
            realized_pnl: Some(pnl),
            exit_price: Some(exit_price),
            exit_time: Some(exit_time),
            exit_reason: Some(crate::execution::ExitReason::TakeProfit),
        }
    }

    #[test]
    fn test_metrics_with_winning_trades() {
        let positions = vec![
            create_test_position(100.0, 60), // $100 profit
            create_test_position(50.0, 120), // $50 profit
            create_test_position(-30.0, 90), // $30 loss
        ];

        let metrics = BacktestMetrics::from_positions(positions, 10000.0, 10120.0, 0, 0.0);

        assert_eq!(metrics.total_trades, 3);
        assert_eq!(metrics.winning_trades, 2);
        assert_eq!(metrics.losing_trades, 1);
        assert!((metrics.win_rate - 66.66).abs() < 0.1);
        assert!((metrics.total_pnl - 120.0).abs() < 0.01);
    }

    #[test]
    fn test_metrics_with_no_trades() {
        let positions = vec![];
        let metrics = BacktestMetrics::from_positions(positions, 10000.0, 10000.0, 0, 0.0);

        assert_eq!(metrics.total_trades, 0);
        assert_eq!(metrics.win_rate, 0.0);
        assert_eq!(metrics.total_pnl, 0.0);
    }

    #[test]
    fn test_profit_factor_calculation() {
        let positions = vec![
            create_test_position(200.0, 60), // $200 win
            create_test_position(100.0, 60), // $100 win
            create_test_position(-50.0, 60), // $50 loss
        ];

        let metrics = BacktestMetrics::from_positions(positions, 10000.0, 10250.0, 0, 0.0);

        // Profit factor = Total wins / Total losses = 300 / 50 = 6.0
        assert!((metrics.profit_factor - 6.0).abs() < 0.01);
    }

    #[test]
    fn test_drawdown_calculation() {
        let positions = vec![
            create_test_position(100.0, 60),  // Peak at 10100
            create_test_position(-200.0, 60), // Down to 9900 (drawdown: 200)
            create_test_position(50.0, 60),   // Back to 9950
        ];

        let metrics = BacktestMetrics::from_positions(positions, 10000.0, 9950.0, 0, 0.0);

        assert!((metrics.max_drawdown - 200.0).abs() < 0.01);
    }
}
