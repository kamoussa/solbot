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
    println!("║     MEAN REVERSION PARAMETER TUNING                   ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("Testing different parameter combinations on SOL\n");

    // Connect to Redis
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let mut redis = RedisPersistence::new(&redis_url).await?;

    // Load SOL candles (90 days hourly)
    println!("📊 Loading SOL data...");
    let candles = redis.load_candles("SOL", 2160).await?;
    println!(
        "✓ Loaded {} candles from {} to {}",
        candles.len(),
        candles.first().unwrap().timestamp,
        candles.last().unwrap().timestamp
    );

    // Configuration
    let initial_portfolio_value = 10000.0;
    let circuit_breakers = CircuitBreakers::default();
    let poll_interval = 60; // hourly
    let transaction_cost = 0.0075; // 0.75% realistic costs

    // Parameter sweep
    // RSI and oversold: Test MORE aggressive since they don't constrain trades
    let rsi_thresholds = vec![20.0, 25.0, 30.0, 35.0, 40.0];
    let oversold_thresholds = vec![-0.12, -0.10, -0.08, -0.06, -0.04];
    // Volume: Test finer granularity to find the cutoff
    let volume_multipliers = vec![2.2, 2.0, 1.8, 1.6, 1.4, 1.2];

    println!(
        "\n🔬 Testing {} parameter combinations...\n",
        rsi_thresholds.len() * oversold_thresholds.len() * volume_multipliers.len()
    );

    let mut results = Vec::new();

    for rsi in &rsi_thresholds {
        for oversold in &oversold_thresholds {
            for vol_mult in &volume_multipliers {
                let config = MeanReversionConfig {
                    ma_period: 20,
                    oversold_threshold: *oversold,
                    rsi_extreme: *rsi,
                    volume_multiplier: *vol_mult,
                    profit_target_pct: 0.08,
                    max_hold_days: 7,
                    rsi_period: 14,
                };

                let strategy = MeanReversionStrategy::new(config).with_poll_interval(poll_interval);

                let runner = BacktestRunner::new(initial_portfolio_value, circuit_breakers.clone());

                match runner.run(
                    &strategy,
                    candles.clone(),
                    "SOL",
                    poll_interval,
                    transaction_cost,
                ) {
                    Ok(metrics) => {
                        results.push((
                            *rsi,
                            *oversold,
                            *vol_mult,
                            metrics.net_return_pct,
                            metrics.total_trades,
                            metrics.win_rate,
                            metrics.largest_loss,
                        ));
                    }
                    Err(e) => {
                        eprintln!("Failed RSI={}, OS={}, Vol={}: {}", rsi, oversold, vol_mult, e);
                    }
                }
            }
        }
    }

    // Sort by return (descending)
    results.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());

    // Print results
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                        PARAMETER SWEEP RESULTS                               ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝\n");
    println!(
        "{:<8} {:<10} {:<10} {:>10} {:>8} {:>8} {:>10}",
        "RSI", "Oversold%", "VolMult", "Return%", "Trades", "Win%", "MaxLoss$"
    );
    println!("{}", "─".repeat(80));

    for (rsi, oversold, vol_mult, return_pct, trades, win_rate, max_loss) in &results {
        // Highlight configs with 3+ trades and positive returns
        let marker = if *trades >= 3 && *return_pct > 0.0 {
            "✓"
        } else if *trades >= 3 {
            "○"
        } else {
            " "
        };

        println!(
            "{} {:<6.0} {:<10.0}% {:<10.1}x {:>10.2} {:>8} {:>8.1} {:>10.2}",
            marker,
            rsi,
            oversold * 100.0,
            vol_mult,
            return_pct,
            trades,
            win_rate,
            max_loss
        );
    }

    println!("\n{}", "─".repeat(80));
    println!("✓ = Profitable with 3+ trades (good validation)");
    println!("○ = 3+ trades but unprofitable");
    println!("  = < 3 trades (insufficient data)");

    // Find best configs by trade count brackets
    println!("\n╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                    RECOMMENDED CONFIGURATIONS                                ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝\n");

    // Best with 3-5 trades (sweet spot)
    let medium_freq: Vec<_> = results
        .iter()
        .filter(|r| r.4 >= 3 && r.4 <= 5)
        .collect();

    if let Some(best) = medium_freq.first() {
        println!("🎯 BEST MEDIUM FREQUENCY (3-5 trades per 90 days):");
        println!(
            "   RSI < {:.0}, Price {:.0}% below MA, Volume {:.1}x average",
            best.0,
            best.1 * 100.0,
            best.2
        );
        println!(
            "   Return: {:+.2}% | Trades: {} | Win Rate: {:.1}% | Max Loss: ${:.2}",
            best.3, best.4, best.5, best.6
        );
        println!();
    }

    // Best with 6+ trades (higher frequency)
    let high_freq: Vec<_> = results.iter().filter(|r| r.4 >= 6).collect();

    if let Some(best) = high_freq.first() {
        println!("⚡ BEST HIGH FREQUENCY (6+ trades per 90 days):");
        println!(
            "   RSI < {:.0}, Price {:.0}% below MA, Volume {:.1}x average",
            best.0,
            best.1 * 100.0,
            best.2
        );
        println!(
            "   Return: {:+.2}% | Trades: {} | Win Rate: {:.1}% | Max Loss: ${:.2}",
            best.3, best.4, best.5, best.6
        );
        println!();
    }

    // Best with 1-2 trades (ultra conservative)
    let low_freq: Vec<_> = results.iter().filter(|r| r.4 >= 1 && r.4 <= 2).collect();

    if let Some(best) = low_freq.first() {
        println!("🛡️  BEST ULTRA CONSERVATIVE (1-2 trades per 90 days):");
        println!(
            "   RSI < {:.0}, Price {:.0}% below MA, Volume {:.1}x average",
            best.0,
            best.1 * 100.0,
            best.2
        );
        println!(
            "   Return: {:+.2}% | Trades: {} | Win Rate: {:.1}% | Max Loss: ${:.2}",
            best.3, best.4, best.5, best.6
        );
        println!();
    }

    Ok(())
}
