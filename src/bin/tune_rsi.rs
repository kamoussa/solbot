use cryptobot::backtest::BacktestRunner;
use cryptobot::db::PostgresPersistence;
use cryptobot::persistence::RedisPersistence;
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::momentum::MomentumStrategy;
use cryptobot::strategy::signals::SignalConfig;
use cryptobot::Result;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<()> {
    println!("\n╔══════════════════════════════════════════════════════╗");
    println!("║     RSI THRESHOLD PARAMETER SWEEP                    ║");
    println!("╚══════════════════════════════════════════════════════╝\n");

    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let mut redis = RedisPersistence::new(&redis_url).await?;

    // Connect to Postgres to save results
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/cryptobot".to_string());
    let postgres = PostgresPersistence::new(&database_url, None).await?;

    // Track optimal thresholds for each token
    let mut optimal_thresholds: HashMap<String, f64> = HashMap::new();

    // Test all 15 tokens from production
    let test_tokens = vec![
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

    for (symbol, name) in &test_tokens {
        println!("\n═══════════════════════════════════════════════════════");
        println!("Testing {}: RSI Threshold Sweep", name);
        println!("═══════════════════════════════════════════════════════\n");

        // Load candles
        let candles = match redis.load_candles(symbol, 2016).await {
            Ok(c) if !c.is_empty() => c,
            _ => {
                println!("⚠️  No data for {} - skipping\n", symbol);
                continue;
            }
        };

        println!("✓ Loaded {} candles for {}\n", candles.len(), name);

        // Test same RSI thresholds for all tokens
        let rsi_thresholds = vec![30.0, 35.0, 40.0, 45.0, 50.0];

        println!("RSI Threshold | Return % | Trades | Win Rate | P&L");
        println!("--------------|----------|--------|----------|----------");

        let mut best_return = f64::NEG_INFINITY;
        let mut best_threshold = 0.0;

        for &rsi_threshold in &rsi_thresholds {
            let config = SignalConfig {
                rsi_period: 14,
                rsi_oversold: rsi_threshold,
                rsi_overbought: 70.0,
                short_ma_period: 10,
                long_ma_period: 20,
                volume_threshold: 1.5,
                lookback_hours: 24,
                enable_panic_buy: true,
                panic_rsi_threshold: 30.0,
                panic_volume_multiplier: 2.0,
                panic_price_drop_pct: 8.0,
                panic_drop_window_candles: 12,
            };

            let strategy = MomentumStrategy::new(config).with_poll_interval(60);
            let runner = BacktestRunner::new(10000.0, CircuitBreakers::default());

            match runner.run(&strategy, candles.clone(), symbol, 60) {
                Ok(metrics) => {
                    println!(
                        "{:^14.1} | {:>8.2} | {:>6} | {:>7.1}% | ${:>8.2}",
                        rsi_threshold,
                        metrics.total_return_pct,
                        metrics.total_trades,
                        metrics.win_rate,
                        metrics.total_pnl
                    );

                    if metrics.total_return_pct > best_return {
                        best_return = metrics.total_return_pct;
                        best_threshold = rsi_threshold;
                    }
                }
                Err(e) => {
                    println!("{:^14.1} | ERROR: {}", rsi_threshold, e);
                }
            }
        }

        println!(
            "\n🏆 Best threshold for {}: RSI < {:.1} ({:+.2}%)",
            name, best_threshold, best_return
        );

        // Store optimal threshold
        optimal_thresholds.insert(symbol.to_string(), best_threshold);
    }

    // Save optimal thresholds to database
    println!("\n═══════════════════════════════════════════════════════");
    println!("Saving optimal RSI thresholds to database...");
    println!("═══════════════════════════════════════════════════════\n");

    for (symbol, threshold) in &optimal_thresholds {
        match postgres.update_rsi_threshold(symbol, *threshold).await {
            Ok(_) => println!("  ✓ Updated {} → RSI < {:.1}", symbol, threshold),
            Err(e) => println!("  ✗ Failed to update {}: {}", symbol, e),
        }
    }

    println!("\n✅ All optimal thresholds saved to database!");
    println!("═══════════════════════════════════════════════════════\n");

    Ok(())
}
