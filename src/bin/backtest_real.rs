use cryptobot::backtest::BacktestRunner;
use cryptobot::persistence::RedisPersistence;
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::buy_and_hold::BuyAndHoldStrategy;
use cryptobot::strategy::momentum::MomentumStrategy;
use cryptobot::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("cryptobot=info")
        .init();

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║     CRYPTOBOT STRATEGY COMPARISON SUITE               ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("Comparing strategies on real CoinGecko data from Redis\n");

    // Configuration
    let initial_portfolio_value = 10000.0;
    let circuit_breakers = CircuitBreakers::default();

    // Strategies to test
    let buy_and_hold = BuyAndHoldStrategy::default();
    let momentum = MomentumStrategy::default();

    // Connect to Redis
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    println!("📡 Connecting to Redis at {}...", redis_url);
    let mut redis = RedisPersistence::new(&redis_url).await?;

    // Tokens to backtest (must have data in Redis)
    //let tokens = vec![("SOL", "Solana"), ("JUP", "Jupiter"), ("Bonk", "Bonk")];
    //  1) "snapshots:KMNO"
    //  2) "snapshots:USELESS"
    //  3) "snapshots:PUMP"
    //  4) "snapshots:WBTC"
    //  5) "snapshots:URANUS"
    //  6) "snapshots:PENGU"
    //  7) "snapshots:WETH"
    //  8) "snapshots:BOT"
    //  9) "snapshots:SPX"
    // 10) "snapshots:RAY"
    // 11) "snapshots:SOL"
    // 12) "snapshots:TROLL"
    // 13) "snapshots:Bonk"
    // 14) "snapshots:JUP"
    // 15) "snapshots:TRUMP"
    let tokens = vec![
        ("SOL", "Solana"),
        ("JUP", "Jupiter"),
        ("Bonk", "Bonk"),
        ("TRUMP", "Trump"),
        ("USELESS", "USELESS"),
        ("PUMP", "PUMP"),
        ("WBTC", "WBTC"),
        ("URANUS", "URANUS"),
        ("PENGU", "PENGU"),
        ("WETH", "WETH"),
        ("BOT", "BOT"),
        ("SPX", "SPX"),
        ("RAY", "RAY"),
        ("TROLL", "TROLL"),
        ("KMNO", "KMNO"),
    ];

    let mut all_results: Vec<(String, String, cryptobot::backtest::BacktestMetrics)> = Vec::new();

    for (symbol, name) in &tokens {
        println!("\n📊 Loading data for {}...", name);

        // Load candles from Redis (7 days = 2016 candles at 5-min intervals)
        match redis.load_candles(symbol, 2016).await {
            Ok(candles) => {
                if candles.is_empty() {
                    println!("⚠️  No data available for {} - skipping", symbol);
                    println!("   Run: cargo run backfill {} <address> --days 1", symbol);
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
                    candles
                        .iter()
                        .map(|c| c.high)
                        .fold(f64::NEG_INFINITY, f64::max)
                );

                // Test Buy & Hold strategy
                {
                    let runner =
                        BacktestRunner::new(initial_portfolio_value, circuit_breakers.clone());
                    println!("\n  🔬 Testing Buy & Hold strategy...");

                    match runner.run(&buy_and_hold, candles.clone(), symbol) {
                        Ok(metrics) => {
                            all_results.push((
                                name.to_string(),
                                "Buy & Hold".to_string(),
                                metrics.clone(),
                            ));
                            println!(
                                "     Result: {:+.2}% ({} trades)",
                                metrics.total_return_pct, metrics.total_trades
                            );
                        }
                        Err(e) => {
                            eprintln!("     ❌ Failed: {}", e);
                        }
                    }
                }

                // Test Momentum strategy
                {
                    let runner =
                        BacktestRunner::new(initial_portfolio_value, circuit_breakers.clone());
                    println!("\n  🔬 Testing Momentum strategy...");

                    match runner.run(&momentum, candles.clone(), symbol) {
                        Ok(metrics) => {
                            all_results.push((
                                name.to_string(),
                                "Momentum".to_string(),
                                metrics.clone(),
                            ));
                            println!(
                                "     Result: {:+.2}% ({} trades)",
                                metrics.total_return_pct, metrics.total_trades
                            );
                        }
                        Err(e) => {
                            eprintln!("     ❌ Failed: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                println!("❌ Failed to load data for {}: {}", symbol, e);
                println!("   Run: cargo run backfill {} <address> --days 1", symbol);
            }
        }
    }

    if all_results.is_empty() {
        println!("\n⚠️  No backtests could run - no data available in Redis");
        println!("\nTo populate data, run:");
        println!("  cargo run backfill SOL So11111111111111111111111111111111111111112 --days 1");
        println!("  cargo run backfill JUP JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN --days 1");
        return Ok(());
    }

    // Print comparison table
    print_strategy_comparison(&all_results);

    Ok(())
}

fn print_strategy_comparison(results: &[(String, String, cryptobot::backtest::BacktestMetrics)]) {
    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║           STRATEGY COMPARISON RESULTS                 ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    // Group results by token
    let tokens: Vec<String> = results
        .iter()
        .map(|(token, _, _)| token.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    for token in &tokens {
        println!("\n📈 {}:", token);
        println!("{}", "─".repeat(70));
        println!(
            "{:<20} {:>10} {:>10} {:>8} {:>8}",
            "Strategy", "P&L", "Return%", "Trades", "Win%"
        );
        println!("{}", "─".repeat(70));

        let token_results: Vec<_> = results.iter().filter(|(t, _, _)| t == token).collect();

        for (_, strategy, metrics) in &token_results {
            println!(
                "{:<20} {:>10.2} {:>10.2} {:>8} {:>8.1}",
                strategy,
                metrics.total_pnl,
                metrics.total_return_pct,
                metrics.total_trades,
                metrics.win_rate
            );
        }

        // Find best strategy for this token
        if let Some(best) = token_results.iter().max_by(|a, b| {
            a.2.total_return_pct
                .partial_cmp(&b.2.total_return_pct)
                .unwrap()
        }) {
            println!(
                "\n   🏆 Best for {}: {} ({:+.2}%)",
                token, best.1, best.2.total_return_pct
            );
        }
    }

    // Overall summary
    println!("\n\n╔═══════════════════════════════════════════════════════╗");
    println!("║              OVERALL SUMMARY                          ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    let strategies: Vec<String> = results
        .iter()
        .map(|(_, strategy, _)| strategy.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    for strategy in &strategies {
        let strategy_results: Vec<_> = results.iter().filter(|(_, s, _)| s == strategy).collect();

        let avg_return: f64 = strategy_results
            .iter()
            .map(|(_, _, m)| m.total_return_pct)
            .sum::<f64>()
            / strategy_results.len() as f64;

        let total_trades: usize = strategy_results
            .iter()
            .map(|(_, _, m)| m.total_trades)
            .sum();

        let avg_win_rate: f64 = strategy_results
            .iter()
            .map(|(_, _, m)| m.win_rate)
            .sum::<f64>()
            / strategy_results.len() as f64;

        let win_count = strategy_results
            .iter()
            .filter(|(_, _, m)| m.total_return_pct > 0.0)
            .count();

        println!("📊 {} Strategy:", strategy);
        println!("   Average Return: {:+.2}%", avg_return);
        println!("   Total Trades: {}", total_trades);
        println!("   Average Win Rate: {:.1}%", avg_win_rate);
        println!(
            "   Profitable Tokens: {}/{} ({:.0}%)",
            win_count,
            strategy_results.len(),
            (win_count as f64 / strategy_results.len() as f64) * 100.0
        );
        println!();
    }

    // Verdict
    if let Some(best_strategy) = strategies.iter().max_by_key(|s| {
        let strategy_results: Vec<_> = results.iter().filter(|(_, strat, _)| strat == *s).collect();
        let avg_return = strategy_results
            .iter()
            .map(|(_, _, m)| m.total_return_pct)
            .sum::<f64>()
            / strategy_results.len() as f64;
        (avg_return * 100.0) as i64 // Convert to basis points for integer comparison
    }) {
        println!(
            "\n🎯 VERDICT: {} strategy performs best on average",
            best_strategy
        );
    }

    println!("\n═══════════════════════════════════════════════════════\n");
}
