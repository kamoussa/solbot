use cryptobot::execution::PriceFeedManager;
use cryptobot::models::Token;
use cryptobot::persistence::RedisPersistence;
use cryptobot::strategy::momentum::MomentumStrategy;
use cryptobot::strategy::Strategy;
use cryptobot::Result;
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

    // Initialize price feed manager (keep 100 snapshots per token)
    let mut price_manager = PriceFeedManager::new(tokens.clone(), 100);

    // Initialize simple momentum strategy
    let strategy = MomentumStrategy::default();

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

    // Polling configuration - adjust these for testing vs production
    let poll_interval_minutes = 5; // For testing: 5 min. Production: 30 min
    let poll_interval_secs = poll_interval_minutes * 60;

    // Calculate required samples based on lookback period
    let samples_needed = strategy.samples_needed(poll_interval_minutes);

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

    // Main event loop - fetch prices at configured interval
    let mut ticker = interval(Duration::from_secs(poll_interval_secs));

    loop {
        ticker.tick().await;

        tracing::info!("=== Tick ===");

        // Fetch all prices
        let results = price_manager.fetch_all().await;

        // Process results - log prices and generate signals
        for (i, result) in results.iter().enumerate() {
            let token = &tokens[i];

            match result {
                Ok(snapshot) => {
                    // Save new snapshot to Redis if available
                    if let Some(ref mut persistence) = persistence {
                        // Get the most recent candle (the one we just added)
                        if let Ok(candles) = price_manager.buffer().get_candles(&token.symbol) {
                            if let Some(latest_candle) = candles.last() {
                                if let Err(e) = persistence.save_candles(&token.symbol, &[latest_candle.clone()]).await {
                                    tracing::warn!("Failed to save snapshot to Redis for {}: {}", token.symbol, e);
                                }
                            }
                        }
                    }

                    // Get historical data
                    let candles = price_manager.buffer()
                        .get_candles(&token.symbol)
                        .unwrap_or_default();

                    tracing::info!(
                        "{}: ${:.4} ({} snapshots)",
                        token.symbol,
                        snapshot.price,
                        candles.len()
                    );

                    // Generate signal if we have enough data
                    if candles.len() >= samples_needed {
                        match strategy.generate_signal(&candles) {
                            Ok(signal) => {
                                tracing::info!("  → {:?}", signal);
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
                Err(e) => {
                    tracing::error!("{}: Failed to fetch price: {}", token.symbol, e);
                }
            }
        }

        tracing::info!("=== End Tick ===\n");
    }
}
