use cryptobot::backtest::{BacktestRunner, MarketScenario, SyntheticDataGenerator};
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::momentum::MomentumStrategy;
use cryptobot::Result;

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("cryptobot=info")
        .init();

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║          CRYPTOBOT BACKTESTING SUITE                 ║");
    println!("╚═══════════════════════════════════════════════════════╝");

    // Configuration
    let initial_portfolio_value = 10000.0;
    let circuit_breakers = CircuitBreakers::default();
    let strategy = MomentumStrategy::default();

    let runner = BacktestRunner::new(initial_portfolio_value, circuit_breakers);

    // Test scenarios
    let scenarios = vec![
        (MarketScenario::Uptrend, "📈 Uptrend (+2% daily)"),
        (MarketScenario::Downtrend, "📉 Downtrend (-2% daily)"),
        (MarketScenario::Sideways, "↔️  Sideways (mean-reverting)"),
        (MarketScenario::Volatile, "⚡ Volatile (±5% swings)"),
        (MarketScenario::WithGaps, "🕳️  With Time Gaps"),
        (MarketScenario::DrawdownTest, "💥 Drawdown Test (25% drop)"),
    ];

    let mut all_metrics = Vec::new();

    for (scenario, name) in scenarios {
        // Generate synthetic data
        let mut generator = SyntheticDataGenerator::new(42);
        let candles = generator.generate(scenario, 500, 5);

        // Run backtest
        match runner.run_and_report(&strategy, candles, "SYNTH", name) {
            Ok(metrics) => {
                all_metrics.push((name.to_string(), metrics));
            }
            Err(e) => {
                eprintln!("❌ Backtest failed for {}: {}", name, e);
            }
        }
    }

    // Summary comparison
    print_summary_comparison(&all_metrics);

    Ok(())
}

fn print_summary_comparison(results: &[(String, cryptobot::backtest::BacktestMetrics)]) {
    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║              SCENARIO COMPARISON                      ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    println!(
        "{:<30} {:>10} {:>10} {:>8} {:>8}",
        "Scenario", "P&L", "Return%", "Trades", "Win%"
    );
    println!("{}", "─".repeat(70));

    for (name, metrics) in results {
        println!(
            "{:<30} {:>10.2} {:>10.2} {:>8} {:>8.1}",
            name,
            metrics.total_pnl,
            metrics.total_return_pct,
            metrics.total_trades,
            metrics.win_rate
        );
    }

    println!("\n");

    // Find best/worst
    if let Some((best_name, best_metrics)) = results.iter().max_by(|a, b| {
        a.1.total_return_pct
            .partial_cmp(&b.1.total_return_pct)
            .unwrap()
    }) {
        println!(
            "🏆 Best Scenario: {} ({:+.2}%)",
            best_name, best_metrics.total_return_pct
        );
    }

    if let Some((worst_name, worst_metrics)) = results.iter().min_by(|a, b| {
        a.1.total_return_pct
            .partial_cmp(&b.1.total_return_pct)
            .unwrap()
    }) {
        println!(
            "⚠️  Worst Scenario: {} ({:+.2}%)",
            worst_name, worst_metrics.total_return_pct
        );
    }

    // Overall statistics
    let total_trades: usize = results.iter().map(|(_, m)| m.total_trades).sum();
    let avg_win_rate: f64 = if !results.is_empty() {
        results.iter().map(|(_, m)| m.win_rate).sum::<f64>() / results.len() as f64
    } else {
        0.0
    };

    println!("\n📊 Overall Statistics:");
    println!("   Total Trades Across All Scenarios: {}", total_trades);
    println!("   Average Win Rate: {:.1}%", avg_win_rate);

    println!("\n═══════════════════════════════════════════════════════\n");
}
