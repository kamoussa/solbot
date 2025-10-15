use cryptobot::execution::{ExecutionAction, Executor, PositionManager, PriceFeedManager};
use cryptobot::models::Token;
use cryptobot::persistence::RedisPersistence;
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::momentum::MomentumStrategy;
use cryptobot::strategy::Strategy;
use cryptobot::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::{interval, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing with debug for strategy
    tracing_subscriber::fmt()
        .with_env_filter("cryptobot=info,cryptobot::strategy=debug")
        .init();

    tracing::info!("CryptoBot starting - KISS mode (Keep It Simple Stupid)");

    // Start with simple tokens - just SOL and JUP
    let tokens = vec![
        Token {
            symbol: "SOL".to_string(),
            mint_address: "So11111111111111111111111111111111111111112".to_string(),
            name: "Solana".to_string(),
            decimals: 9,
        },
        Token {
            symbol: "JUP".to_string(),
            mint_address: "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN".to_string(),
            name: "Jupiter".to_string(),
            decimals: 6,
        },
    ];

    // Polling configuration - adjust these for testing vs production
    let poll_interval_minutes = 5; // For testing: 5 min. Production: 30 min

    // Initialize simple momentum strategy
    let strategy = MomentumStrategy::default().with_poll_interval(poll_interval_minutes);

    // Calculate buffer size based on strategy needs (add 20% buffer for safety)
    let samples_needed = strategy.samples_needed(poll_interval_minutes);
    let buffer_size = (samples_needed as f64 * 1.2) as usize; // 288 * 1.2 = 345

    // Initialize price feed manager with calculated buffer size
    let mut price_manager = PriceFeedManager::new(tokens.clone(), buffer_size);

    // Initialize Redis persistence
    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    let mut persistence = match RedisPersistence::new(&redis_url).await {
        Ok(p) => {
            tracing::info!("Redis persistence enabled at {}", redis_url);
            Some(p)
        }
        Err(e) => {
            tracing::warn!("Failed to connect to Redis ({}), continuing without persistence", e);
            None
        }
    };

    let poll_interval_secs = poll_interval_minutes * 60;

    tracing::info!("Watching {} tokens:", tokens.len());
    for token in &tokens {
        tracing::info!("  - {} ({})", token.symbol, token.name);
    }
    tracing::info!(
        "Strategy: {} | Lookback: {}h | Poll: {}min | Need {} snapshots",
        strategy.name(),
        strategy.lookback_hours(),
        poll_interval_minutes,
        samples_needed
    );

    // Load historical data from Redis if available
    if let Some(ref mut persistence) = persistence {
        tracing::info!("Loading historical data from Redis...");

        for token in &tokens {
            match persistence.load_candles(&token.symbol, strategy.lookback_hours()).await {
                Ok(historical) => {
                    if !historical.is_empty() {
                        for candle in &historical {
                            if let Err(e) = price_manager.buffer().add_candle(candle.clone()) {
                                tracing::warn!("Failed to add historical candle for {}: {}", token.symbol, e);
                            }
                        }
                        tracing::info!(
                            "✓ Loaded {} historical snapshots for {} from Redis",
                            historical.len(),
                            token.symbol
                        );
                    } else {
                        tracing::info!("  No historical data for {} in Redis", token.symbol);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to load historical data for {}: {}", token.symbol, e);
                }
            }
        }
    }

    // Initialize position manager and executor
    let initial_portfolio_value = std::env::var("INITIAL_PORTFOLIO_VALUE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(10000.0);

    let circuit_breakers = CircuitBreakers::default();

    tracing::info!("Portfolio: ${:.2} | Circuit Breakers: max_daily_loss={}%, max_drawdown={}%",
        initial_portfolio_value,
        circuit_breakers.max_daily_loss_pct * 100.0,
        circuit_breakers.max_drawdown_pct * 100.0
    );

    let position_manager = Arc::new(Mutex::new(PositionManager::new(
        initial_portfolio_value,
        circuit_breakers,
    )));

    let mut executor = Executor::new(position_manager.clone());

    // Main event loop - fetch prices at configured interval
    let mut ticker = interval(Duration::from_secs(poll_interval_secs));

    loop {
        ticker.tick().await;

        tracing::info!("=== Tick ===");

        // Fetch all prices
        let results = price_manager.fetch_all().await;

        // Collect prices into HashMap for position manager
        let mut prices: HashMap<String, f64> = HashMap::new();

        // Process results - save to Redis and collect prices
        for (i, result) in results.iter().enumerate() {
            let token = &tokens[i];

            match result {
                Ok(snapshot) => {
                    prices.insert(token.symbol.clone(), snapshot.price);

                    // Save new snapshot to Redis if available
                    if let Some(ref mut persistence) = persistence {
                        if let Ok(candles) = price_manager.buffer().get_candles(&token.symbol) {
                            if let Some(latest_candle) = candles.last() {
                                if let Err(e) = persistence.save_candles(&token.symbol, &[latest_candle.clone()]).await {
                                    tracing::warn!("Failed to save snapshot to Redis for {}: {}", token.symbol, e);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("{}: Failed to fetch price: {}", token.symbol, e);
                }
            }
        }

        // Check for exit conditions on existing positions FIRST
        {
            let mut pm = position_manager.lock().unwrap();
            match pm.check_exits(&prices) {
                Ok(closed_ids) => {
                    for position_id in closed_ids {
                        tracing::info!("✓ Position {} closed by exit condition", position_id);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to check exits: {}", e);
                }
            }
        }

        // Process each token - generate signals and execute
        for token in &tokens {
            if let Some(&current_price) = prices.get(&token.symbol) {
                let candles = price_manager.buffer()
                    .get_candles(&token.symbol)
                    .unwrap_or_default();

                tracing::info!(
                    "{}: ${:.4} ({} snapshots)",
                    token.symbol,
                    current_price,
                    candles.len()
                );

                // Generate signal if we have enough data
                if candles.len() >= samples_needed {
                    match strategy.generate_signal(&candles) {
                        Ok(signal) => {
                            tracing::info!("  Signal: {:?}", signal);

                            // Process signal with executor
                            match executor.process_signal(&signal, &token.symbol, current_price) {
                                Ok(decision) => {
                                    tracing::info!("  Decision: {:?} - {}", decision.action, decision.reason);

                                    // Execute decision
                                    match decision.action {
                                        ExecutionAction::Execute { quantity } => {
                                            // For now: just log, later: send transaction
                                            tracing::info!(
                                                "  → Would BUY {:.4} {} @ ${:.4} (total: ${:.2})",
                                                quantity,
                                                token.symbol,
                                                current_price,
                                                quantity * current_price
                                            );

                                            // Simulate opening the position
                                            let mut pm = position_manager.lock().unwrap();
                                            match pm.open_position(token.symbol.clone(), current_price, quantity) {
                                                Ok(position_id) => {
                                                    tracing::info!("  ✓ Opened position {} for {}", position_id, token.symbol);
                                                }
                                                Err(e) => {
                                                    tracing::error!("  ✗ Failed to open position: {}", e);
                                                }
                                            }
                                        }
                                        ExecutionAction::Close { position_id } => {
                                            // For now: just log, later: send transaction
                                            let mut pm = position_manager.lock().unwrap();
                                            match pm.close_position(
                                                position_id,
                                                current_price,
                                                cryptobot::execution::ExitReason::Manual,
                                            ) {
                                                Ok(()) => {
                                                    tracing::info!(
                                                        "  ✓ Closed position {} @ ${:.4}",
                                                        position_id,
                                                        current_price
                                                    );
                                                }
                                                Err(e) => {
                                                    tracing::error!("  ✗ Failed to close position: {}", e);
                                                }
                                            }
                                        }
                                        ExecutionAction::Skip => {
                                            // Do nothing - already logged
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("  Failed to process signal: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("  → Failed to generate signal: {}", e);
                        }
                    }
                } else {
                    tracing::info!(
                        "  → Collecting data... ({}/{} for {}h lookback)",
                        candles.len(),
                        samples_needed,
                        strategy.lookback_hours()
                    );
                }
            }
        }

        // Log portfolio state
        {
            let pm = position_manager.lock().unwrap();
            let portfolio_value = pm.portfolio_value(&prices).unwrap_or(initial_portfolio_value);
            let open_positions = pm.open_positions();

            tracing::info!("\n📊 Portfolio Summary:");
            tracing::info!("  Value: ${:.2}", portfolio_value);
            tracing::info!("  P&L: ${:.2} ({:.2}%)",
                portfolio_value - initial_portfolio_value,
                ((portfolio_value - initial_portfolio_value) / initial_portfolio_value) * 100.0
            );
            tracing::info!("  Open Positions: {}", open_positions.len());

            for position in open_positions {
                if let Some(&current_price) = prices.get(&position.token) {
                    let unrealized_pnl = (current_price - position.entry_price) * position.quantity;
                    let unrealized_pnl_pct = ((current_price - position.entry_price) / position.entry_price) * 100.0;
                    tracing::info!(
                        "    {} | Entry: ${:.4} | Current: ${:.4} | P&L: ${:.2} ({:.2}%)",
                        position.token,
                        position.entry_price,
                        current_price,
                        unrealized_pnl,
                        unrealized_pnl_pct
                    );
                }
            }
        }

        tracing::info!("=== End Tick ===\n");
    }
}
