use cryptobot::backtest::BacktestRunner;
use cryptobot::execution::ExitReason;
use cryptobot::persistence::RedisPersistence;
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::momentum::MomentumStrategy;
use cryptobot::strategy::signals::SignalConfig;
use cryptobot::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("cryptobot=warn")
        .init();

    println!("\n═══════════════════════════════════════════════════════");
    println!("         DETAILED TRADE ANALYSIS: 365-Day SOL");
    println!("═══════════════════════════════════════════════════════\n");

    // Configuration
    let initial_portfolio_value = 10000.0;
    let circuit_breakers = CircuitBreakers::default();

    // Connect to Postgres to load RSI threshold
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cryptobot:cryptobot_dev_password@localhost:5432/cryptobot".to_string());
    let postgres = cryptobot::db::PostgresPersistence::new(&database_url, None).await?;

    let rsi_threshold = postgres
        .get_rsi_threshold("SOL")
        .await
        .unwrap_or(50.0);

    // Connect to Redis
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let mut redis = RedisPersistence::new(&redis_url).await?;

    // Load 365 days of SOL candles
    println!("Loading 365 days of SOL data from Redis...");
    let candles = redis.load_candles("SOL", 8760).await?;

    if candles.is_empty() {
        eprintln!("No data available for SOL");
        return Ok(());
    }

    // Detect granularity
    let (granularity, poll_interval) = if candles.len() > 1 {
        let interval_secs = (candles[1].timestamp - candles[0].timestamp).num_seconds();
        if interval_secs >= 43200 {
            ("daily", 1440)
        } else if interval_secs >= 3000 {
            ("hourly", 60)
        } else {
            ("5-minute", 5)
        }
    } else {
        ("unknown", 5)
    };

    println!("✓ Loaded {} {} candles", candles.len(), granularity);
    println!("Period: {} to {}\n", candles.first().unwrap().timestamp, candles.last().unwrap().timestamp);

    // Create momentum strategy
    let mut config = SignalConfig::default();
    config.rsi_oversold = rsi_threshold;
    let momentum = MomentumStrategy::new(config).with_poll_interval(poll_interval);

    // Run backtest with no fees to see gross performance
    let runner = BacktestRunner::new(initial_portfolio_value, circuit_breakers.clone());
    let metrics = runner.run(&momentum, candles.clone(), "SOL", poll_interval, 0.0)?;

    // Print summary
    println!("═══════════════════════════════════════════════════════");
    println!("                  BACKTEST SUMMARY");
    println!("═══════════════════════════════════════════════════════\n");
    println!("Total Trades:        {}", metrics.total_trades);
    println!("Winning Trades:      {} ({:.1}%)", metrics.winning_trades, metrics.win_rate);
    println!("Losing Trades:       {}", metrics.losing_trades);
    println!("Gross Return:        {:+.2}%", metrics.total_return_pct);
    println!("Average Win:         ${:.2}", metrics.avg_win);
    println!("Average Loss:        ${:.2}", metrics.avg_loss);
    println!("Profit Factor:       {:.2}", metrics.profit_factor);
    println!("Sharpe Ratio:        {:.2}", metrics.sharpe_ratio);
    println!("Max Drawdown:        ${:.2} ({:.2}%)", metrics.max_drawdown, metrics.max_drawdown_pct);
    println!("Avg Hold Time:       {:.1} days", metrics.avg_holding_period_minutes / 1440.0);

    // Analyze trades by exit reason
    println!("\n═══════════════════════════════════════════════════════");
    println!("               EXIT REASON BREAKDOWN");
    println!("═══════════════════════════════════════════════════════\n");

    // Load positions to get exit reasons
    let database_url_reuse = database_url.clone();
    let pg = cryptobot::db::PostgresPersistence::new(&database_url_reuse, None).await?;

    // Get recent positions (this is a workaround since we don't have direct access to backtest positions)
    // Instead, let's analyze the metrics.trades Vec which has all the data

    let mut exit_reasons_count = std::collections::HashMap::new();
    let mut exit_reasons_pnl = std::collections::HashMap::new();

    // Note: We can't directly get exit_reason from TradeRecord, but we can infer some patterns
    // Let's analyze the trades by characteristics:

    for (i, trade) in metrics.trades.iter().enumerate() {
        let holding_days = trade.holding_period_minutes as f64 / 1440.0;
        let pnl_pct = trade.pnl_pct;

        // Infer exit reason based on characteristics
        let reason = if holding_days >= 14.0 {
            "TimeStop (14+ days)"
        } else if pnl_pct >= 12.0 {
            "TakeProfit (>12%)"
        } else if pnl_pct <= -8.0 {
            "StopLoss (<-8%)"
        } else if pnl_pct > 0.0 && holding_days < 3.0 {
            "StrategySell (quick win)"
        } else if pnl_pct < 0.0 && holding_days < 7.0 {
            "StrategySell (early loss)"
        } else {
            "StrategySell (normal)"
        };

        *exit_reasons_count.entry(reason).or_insert(0) += 1;
        *exit_reasons_pnl.entry(reason).or_insert(0.0) += trade.pnl;

        println!("Trade #{}: {:.1} days | {:+.2}% | ${:+.2} | [{}]",
            i + 1,
            holding_days,
            pnl_pct,
            trade.pnl,
            reason
        );
    }

    println!("\n─────────────────────────────────────────────────────────");
    println!("Summary by Exit Reason:\n");

    for (reason, count) in exit_reasons_count.iter() {
        let total_pnl = exit_reasons_pnl.get(reason).unwrap_or(&0.0);
        let avg_pnl = total_pnl / *count as f64;
        println!("{:<30} {:>3} trades | Avg: ${:+7.2} | Total: ${:+8.2}",
            reason, count, avg_pnl, total_pnl);
    }

    // Detailed trade breakdown
    println!("\n═══════════════════════════════════════════════════════");
    println!("                DETAILED TRADE LIST");
    println!("═══════════════════════════════════════════════════════\n");
    println!("{:<5} {:<20} {:<20} {:<10} {:<10} {:<8} {:<10}",
        "#", "Entry", "Exit", "Entry$", "Exit$", "P&L%", "Hold(days)");
    println!("{}", "─".repeat(100));

    for (i, trade) in metrics.trades.iter().enumerate() {
        println!("{:<5} {:<20} {:<20} ${:<9.2} ${:<9.2} {:+7.2}% {:>8.1}d",
            i + 1,
            trade.entry_time.format("%Y-%m-%d %H:%M"),
            trade.exit_time.format("%Y-%m-%d %H:%M"),
            trade.entry_price,
            trade.exit_price,
            trade.pnl_pct,
            trade.holding_period_minutes as f64 / 1440.0
        );
    }

    Ok(())
}
