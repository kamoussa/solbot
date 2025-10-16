use cryptobot::db::PostgresPersistence;
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

    // Initialize Redis persistence for candles
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    let mut redis_persistence = match RedisPersistence::new(&redis_url).await {
        Ok(p) => {
            tracing::info!("Redis persistence enabled at {} (candles)", redis_url);
            Some(p)
        }
        Err(e) => {
            tracing::warn!(
                "Failed to connect to Redis ({}), continuing without candle persistence",
                e
            );
            None
        }
    };

    // Initialize Postgres persistence for positions
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/cryptobot".to_string());

    let mut postgres_persistence = match PostgresPersistence::new(&database_url, None).await {
        Ok(p) => {
            tracing::info!(
                "Postgres persistence enabled at {} (positions)",
                database_url
            );
            Some(p)
        }
        Err(e) => {
            tracing::warn!(
                "Failed to connect to Postgres ({}), continuing without position persistence",
                e
            );
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

    // Load historical candle data from Redis if available
    if let Some(ref mut redis_persistence) = redis_persistence {
        tracing::info!("Loading historical candle data from Redis...");

        for token in &tokens {
            match redis_persistence
                .load_candles(&token.symbol, strategy.lookback_hours())
                .await
            {
                Ok(historical) => {
                    if !historical.is_empty() {
                        for candle in &historical {
                            if let Err(e) = price_manager.buffer().add_candle(candle.clone()) {
                                tracing::warn!(
                                    "Failed to add historical candle for {}: {}",
                                    token.symbol,
                                    e
                                );
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

    tracing::info!(
        "Portfolio: ${:.2} | Circuit Breakers: max_daily_loss={}%, max_drawdown={}%",
        initial_portfolio_value,
        circuit_breakers.max_daily_loss_pct * 100.0,
        circuit_breakers.max_drawdown_pct * 100.0
    );

    // Load positions from Postgres if available
    let loaded_positions = if let Some(ref mut postgres_persistence) = postgres_persistence {
        tracing::info!("Loading historical positions from Postgres...");
        match postgres_persistence.load_positions().await {
            Ok(positions) => {
                if !positions.is_empty() {
                    tracing::info!("✓ Loaded {} positions from Postgres", positions.len());
                    Some(positions)
                } else {
                    tracing::info!("  No historical positions found in Postgres");
                    None
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load positions from Postgres: {}", e);
                None
            }
        }
    } else {
        None
    };

    // Create PositionManager with loaded state or fresh
    let position_manager = Arc::new(Mutex::new(if let Some(positions) = loaded_positions {
        PositionManager::with_positions(initial_portfolio_value, circuit_breakers, positions)
    } else {
        PositionManager::new(initial_portfolio_value, circuit_breakers)
    }));

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
                    if let Some(ref mut redis_persistence) = redis_persistence {
                        if let Ok(candles) = price_manager.buffer().get_candles(&token.symbol) {
                            if let Some(latest_candle) = candles.last() {
                                match redis_persistence
                                    .save_candles(&token.symbol, &[latest_candle.clone()])
                                    .await
                                {
                                    Ok(_) => {
                                        tracing::debug!(
                                            "Saved {} snapshot to Redis (total: {})",
                                            token.symbol,
                                            candles.len()
                                        );
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to save snapshot to Redis for {}: {}",
                                            token.symbol,
                                            e
                                        );
                                    }
                                }
                            } else {
                                tracing::debug!("No candles in buffer yet for {}", token.symbol);
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

                        // Save closed position to Postgres
                        if let Some(position) =
                            pm.all_positions().iter().find(|p| p.id == position_id)
                        {
                            if let Some(ref mut postgres_persistence) = postgres_persistence {
                                if let Err(e) = postgres_persistence.save_position(position).await {
                                    tracing::warn!(
                                        "Failed to save closed position to Postgres: {}",
                                        e
                                    );
                                }
                            }
                        }
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
                let candles = price_manager
                    .buffer()
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
                                    tracing::info!(
                                        "  Decision: {:?} - {}",
                                        decision.action,
                                        decision.reason
                                    );

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
                                            match pm.open_position(
                                                token.symbol.clone(),
                                                current_price,
                                                quantity,
                                            ) {
                                                Ok(position_id) => {
                                                    tracing::info!(
                                                        "  ✓ Opened position {} for {}",
                                                        position_id,
                                                        token.symbol
                                                    );

                                                    // Save position to Postgres
                                                    if let Some(position) = pm
                                                        .all_positions()
                                                        .iter()
                                                        .find(|p| p.id == position_id)
                                                    {
                                                        if let Some(ref mut postgres_persistence) =
                                                            postgres_persistence
                                                        {
                                                            if let Err(e) = postgres_persistence
                                                                .save_position(position)
                                                                .await
                                                            {
                                                                tracing::warn!("Failed to save position to Postgres: {}", e);
                                                            }
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::error!(
                                                        "  ✗ Failed to open position: {}",
                                                        e
                                                    );
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

                                                    // Save updated position to Postgres
                                                    if let Some(position) = pm
                                                        .all_positions()
                                                        .iter()
                                                        .find(|p| p.id == position_id)
                                                    {
                                                        if let Some(ref mut postgres_persistence) =
                                                            postgres_persistence
                                                        {
                                                            if let Err(e) = postgres_persistence
                                                                .save_position(position)
                                                                .await
                                                            {
                                                                tracing::warn!("Failed to save closed position to Postgres: {}", e);
                                                            }
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::error!(
                                                        "  ✗ Failed to close position: {}",
                                                        e
                                                    );
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
            let portfolio_value = pm
                .portfolio_value(&prices)
                .unwrap_or(initial_portfolio_value);
            let open_positions = pm.open_positions();

            tracing::info!("\n📊 Portfolio Summary:");
            tracing::info!("  Value: ${:.2}", portfolio_value);
            tracing::info!(
                "  P&L: ${:.2} ({:.2}%)",
                portfolio_value - initial_portfolio_value,
                ((portfolio_value - initial_portfolio_value) / initial_portfolio_value) * 100.0
            );
            tracing::info!("  Open Positions: {}", open_positions.len());

            for position in open_positions {
                if let Some(&current_price) = prices.get(&position.token) {
                    let unrealized_pnl = (current_price - position.entry_price) * position.quantity;
                    let unrealized_pnl_pct =
                        ((current_price - position.entry_price) / position.entry_price) * 100.0;
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
