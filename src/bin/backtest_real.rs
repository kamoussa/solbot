use cryptobot::backtest::BacktestRunner;
use cryptobot::persistence::RedisPersistence;
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::momentum::MomentumStrategy;
use cryptobot::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("cryptobot=info")
        .init();

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║     CRYPTOBOT REAL DATA BACKTESTING SUITE            ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("Using real CoinGecko data from Redis\n");

    // Configuration
    let initial_portfolio_value = 10000.0;
    let circuit_breakers = CircuitBreakers::default();
    let strategy = MomentumStrategy::default();

    // Connect to Redis
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    println!("📡 Connecting to Redis at {}...", redis_url);
    let mut redis = RedisPersistence::new(&redis_url).await?;

    // Tokens to backtest (must have data in Redis)
    let tokens = vec![
        ("SOL", "Solana"),
        ("JUP", "Jupiter"),
        ("Bonk", "Bonk"),
    ];

    let runner = BacktestRunner::new(initial_portfolio_value, circuit_breakers);
    let mut all_metrics = Vec::new();

    for (symbol, name) in &tokens {
        println!("\n🔍 Loading data for {}...", name);

        // Load candles from Redis (7 days = 2016 candles at 5-min intervals)
        match redis.load_candles(symbol, 2016).await {
            Ok(candles) => {
                if candles.is_empty() {
                    println!("⚠️  No data available for {} - skipping", symbol);
                    println!("   Run: cargo run --bin cryptobot backfill {} <address> --days 1", symbol);
                    continue;
                }

                println!("✓ Loaded {} candles for {}", candles.len(), name);
                println!(
                    "  Period: {} to {}",
                    candles.first().unwrap().timestamp,
                    candles.last().unwrap().timestamp
                );
                println!(
                    "  Price range: ${:.2} - ${:.2}",
                    candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min),
                    candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max)
                );

                // Run backtest
                match runner.run_and_report(&strategy, candles, symbol, name) {
                    Ok(metrics) => {
                        all_metrics.push((format!("{} ({})", name, symbol), metrics));
                    }
                    Err(e) => {
                        eprintln!("❌ Backtest failed for {}: {}", name, e);
                    }
                }
            }
            Err(e) => {
                println!("❌ Failed to load data for {}: {}", symbol, e);
                println!("   Run: cargo run --bin cryptobot backfill {} <address> --days 1", symbol);
            }
        }
    }

    if all_metrics.is_empty() {
        println!("\n⚠️  No backtests could run - no data available in Redis");
        println!("\nTo populate data, run:");
        println!("  cargo run --bin cryptobot backfill SOL So11111111111111111111111111111111111111112 --days 1");
        println!("  cargo run --bin cryptobot backfill JUP JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN --days 1");
        return Ok(());
    }

    // Summary comparison
    print_summary_comparison(&all_metrics);

    Ok(())
}

fn print_summary_comparison(results: &[(String, cryptobot::backtest::BacktestMetrics)]) {
    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║              REAL DATA COMPARISON                     ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    println!(
        "{:<30} {:>10} {:>10} {:>8} {:>8}",
        "Token", "P&L", "Return%", "Trades", "Win%"
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
            "🏆 Best Performer: {} ({:+.2}%)",
            best_name, best_metrics.total_return_pct
        );
    }

    if let Some((worst_name, worst_metrics)) = results.iter().min_by(|a, b| {
        a.1.total_return_pct
            .partial_cmp(&b.1.total_return_pct)
            .unwrap()
    }) {
        println!(
            "⚠️  Worst Performer: {} ({:+.2}%)",
            worst_name, worst_metrics.total_return_pct
        );
    }

    // Overall statistics
    let total_trades: usize = results.iter().map(|(_, m)| m.total_trades).sum();
    let avg_return: f64 = if !results.is_empty() {
        results.iter().map(|(_, m)| m.total_return_pct).sum::<f64>() / results.len() as f64
    } else {
        0.0
    };
    let avg_win_rate: f64 = if !results.is_empty() {
        results.iter().map(|(_, m)| m.win_rate).sum::<f64>() / results.len() as f64
    } else {
        0.0
    };

    println!("\n📊 Overall Statistics:");
    println!("   Total Trades Across All Tokens: {}", total_trades);
    println!("   Average Return: {:+.2}%", avg_return);
    println!("   Average Win Rate: {:.1}%", avg_win_rate);

    // Strategy health check
    println!("\n🏥 Strategy Health Check:");
    let profitable_count = results.iter().filter(|(_, m)| m.total_pnl > 0.0).count();
    let health_ratio = if !results.is_empty() {
        (profitable_count as f64 / results.len() as f64) * 100.0
    } else {
        0.0
    };

    if health_ratio >= 66.0 {
        println!("   ✅ HEALTHY: {}/{} tokens profitable ({:.0}%)",
            profitable_count, results.len(), health_ratio);
    } else if health_ratio >= 33.0 {
        println!("   ⚠️  MARGINAL: {}/{} tokens profitable ({:.0}%)",
            profitable_count, results.len(), health_ratio);
    } else {
        println!("   ❌ POOR: {}/{} tokens profitable ({:.0}%)",
            profitable_count, results.len(), health_ratio);
    }

    println!("\n═══════════════════════════════════════════════════════\n");
}
