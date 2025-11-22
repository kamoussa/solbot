use cryptobot::backtest::BacktestRunner;
use cryptobot::persistence::RedisPersistence;
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::mean_reversion::{MeanReversionConfig, MeanReversionStrategy};
use cryptobot::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("cryptobot=warn")
        .init();

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║     MEAN REVERSION PARAMETER OPTIMIZATION            ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("Testing parameter combinations on 90-day hourly SOL data\n");

    // Configuration
    let initial_portfolio_value = 10000.0;
    let circuit_breakers = CircuitBreakers::default();
    let transaction_cost_pct = 0.0075; // 0.75% realistic fees

    // Connect to Redis
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let mut redis = RedisPersistence::new(&redis_url).await?;

    // Load 90-day hourly data
    println!("📡 Loading SOL data from Redis...");
    let candles = redis.load_all_candles("SOL").await?;

    if candles.is_empty() {
        println!("❌ No data available in Redis");
        println!("   Run backfill first: cargo run backfill SOL <address> --days 90");
        return Ok(());
    }

    // Detect poll interval
    let poll_interval = if candles.len() > 1 {
        let interval_secs = (candles[1].timestamp - candles[0].timestamp).num_seconds();
        if interval_secs >= 3000 {
            60 // Hourly
        } else {
            5 // 5-minute
        }
    } else {
        60
    };

    println!("✓ Loaded {} candles", candles.len());
    println!("  Period: {} to {}",
        candles.first().unwrap().timestamp,
        candles.last().unwrap().timestamp
    );
    println!("  Poll interval: {} minutes\n", poll_interval);

    // Parameter grid to test
    let oversold_thresholds = vec![-0.08, -0.10, -0.12, -0.15]; // % below MA
    let rsi_extremes = vec![20.0, 25.0, 30.0];
    let profit_targets = vec![0.06, 0.08, 0.10, 0.12]; // % profit
    let volume_multipliers = vec![1.5, 2.0, 2.5]; // x average

    let total_combinations = oversold_thresholds.len()
        * rsi_extremes.len()
        * profit_targets.len()
        * volume_multipliers.len();

    println!("🔬 Testing {} parameter combinations...\n", total_combinations);

    let mut results = Vec::new();
    let mut tested = 0;

    for &oversold in &oversold_thresholds {
        for &rsi_extreme in &rsi_extremes {
            for &profit_target in &profit_targets {
                for &volume_mult in &volume_multipliers {
                    tested += 1;

                    // Create config with these parameters
                    let config = MeanReversionConfig {
                        ma_period: 20,
                        oversold_threshold: oversold,
                        rsi_extreme,
                        volume_multiplier: volume_mult,
                        profit_target_pct: profit_target,
                        max_hold_days: 7,
                        rsi_period: 14,
                    };

                    let strategy = MeanReversionStrategy::new(config.clone())
                        .with_poll_interval(poll_interval);

                    // Run backtest
                    let runner = BacktestRunner::new(initial_portfolio_value, circuit_breakers.clone());

                    match runner.run(&strategy, candles.clone(), "SOL", poll_interval, transaction_cost_pct) {
                        Ok(metrics) => {
                            results.push((config, metrics));

                            // Print progress every 10 tests
                            if tested % 10 == 0 {
                                println!("  Tested {}/{} combinations...", tested, total_combinations);
                            }
                        }
                        Err(_e) => {
                            // Skip failed backtests (e.g., insufficient data)
                            continue;
                        }
                    }
                }
            }
        }
    }

    println!("\n✓ Completed {} backtests\n", results.len());

    // Sort by net return (descending)
    results.sort_by(|a, b| {
        b.1.net_return_pct
            .partial_cmp(&a.1.net_return_pct)
            .unwrap()
    });

    // Print top 10 results
    println!("╔═══════════════════════════════════════════════════════╗");
    println!("║              TOP 10 PARAMETER COMBINATIONS            ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    for (i, (config, metrics)) in results.iter().take(10).enumerate() {
        println!("#{} Return: {:+.2}% (Gross: {:+.2}%)",
            i + 1,
            metrics.net_return_pct,
            metrics.total_return_pct
        );
        println!("   Oversold: {:.0}% below MA | RSI: <{:.0} | Profit: {:.0}% | Volume: {:.1}x",
            config.oversold_threshold * 100.0,
            config.rsi_extreme,
            config.profit_target_pct * 100.0,
            config.volume_multiplier
        );
        println!("   Trades: {} | Win Rate: {:.1}% | Max DD: {:.2}%",
            metrics.total_trades,
            metrics.win_rate,
            metrics.max_drawdown_pct
        );
        println!();
    }

    // Show baseline (default params) for comparison
    if let Some((baseline_idx, _)) = results.iter().enumerate().find(|(_, (cfg, _))| {
        cfg.oversold_threshold == -0.10
            && cfg.rsi_extreme == 25.0
            && cfg.profit_target_pct == 0.08
            && cfg.volume_multiplier == 2.0
    }) {
        println!("📊 Default parameters ranked: #{}", baseline_idx + 1);
    }

    // Print recommendation
    if !results.is_empty() {
        let best = &results[0];
        println!("\n╔═══════════════════════════════════════════════════════╗");
        println!("║              RECOMMENDED PARAMETERS                   ║");
        println!("╚═══════════════════════════════════════════════════════╝\n");
        println!("  oversold_threshold: {:.2} ({:.0}% below MA)",
            best.0.oversold_threshold,
            best.0.oversold_threshold * 100.0
        );
        println!("  rsi_extreme: {:.0}", best.0.rsi_extreme);
        println!("  profit_target_pct: {:.2} ({:.0}%)",
            best.0.profit_target_pct,
            best.0.profit_target_pct * 100.0
        );
        println!("  volume_multiplier: {:.1}x", best.0.volume_multiplier);
        println!("\n  Expected Performance:");
        println!("    Net Return: {:+.2}%", best.1.net_return_pct);
        println!("    Trades: {}", best.1.total_trades);
        println!("    Win Rate: {:.1}%", best.1.win_rate);
        println!("    Max Drawdown: {:.2}%", best.1.max_drawdown_pct);
    }

    println!("\n═══════════════════════════════════════════════════════\n");

    Ok(())
}
