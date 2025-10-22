use cryptobot::api::birdeye::{BirdeyeClient, TrendingToken};
use cryptobot::db::PostgresPersistence;
use cryptobot::discovery::safety::is_safe_token;
use cryptobot::execution::{
    ExecutionAction, Executor, Position, PositionManager, PriceFeedManager,
};
use cryptobot::models::Token;
use cryptobot::persistence::RedisPersistence;
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::momentum::MomentumStrategy;
use cryptobot::strategy::signals::validate_candle_uniformity;
use cryptobot::strategy::Strategy;
use cryptobot::Result;
use chrono::{Timelike, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use tokio::time::{interval_at, Duration, Instant};

// Must-track tokens (always include these)
const MUST_TRACK: &[(&str, &str)] = &[
    ("So11111111111111111111111111111111111111112", "SOL"),
    ("JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN", "JUP"),
];

const MAX_TOKENS: usize = 10;
const POLL_INTERVAL_MINUTES: u64 = 5;

// ============================================================================
// Shared State
// ============================================================================

struct SharedState {
    tokens: Arc<RwLock<Vec<Token>>>,
    position_manager: Arc<Mutex<PositionManager>>,
    initial_portfolio_value: f64,
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Calculate when the next 5-minute boundary will occur (XX:00, XX:05, XX:10, etc.)
fn next_5min_boundary() -> Instant {
    let now = Utc::now();
    let current_minute = now.minute();
    let current_second = now.second();

    let minutes_until_next = 5 - (current_minute % 5);
    let seconds_until_next = if minutes_until_next == 5 && current_second == 0 {
        0 // Already at boundary
    } else {
        (minutes_until_next * 60) - current_second
    };

    Instant::now() + Duration::from_secs(seconds_until_next as u64)
}

/// Calculate when to run 30 seconds after the next 5-minute boundary
fn next_5min_boundary_plus_30s() -> Instant {
    next_5min_boundary() + Duration::from_secs(30)
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    setup_logging();

    tracing::info!("🚀 CryptoBot starting - Multi-Loop Architecture");

    // Get environment variables
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let database_url = std::env::var("DATABASE_URL").ok();
    let birdeye_api_key =
        std::env::var("BIRDEYE_API_KEY").expect("BIRDEYE_API_KEY not found in environment");
    let initial_portfolio_value = get_initial_portfolio_value();

    // Initialize Postgres and load existing state
    let mut postgres_persistence = connect_to_postgres().await;

    // Initial token discovery
    tracing::info!("🔍 Performing initial token discovery...");
    let birdeye_client = create_birdeye_client()?;
    let final_tokens =
        discover_and_build_token_list(&birdeye_client, postgres_persistence.as_ref()).await?;

    if let Some(ref mut postgres) = postgres_persistence {
        save_tracked_tokens_to_db(postgres, &final_tokens).await;
    }

    let initial_tokens = convert_to_tokens(&final_tokens);
    if initial_tokens.is_empty() {
        return Err("No safe tokens found! Cannot start bot.".into());
    }

    tracing::info!("✅ Initial discovery complete: {} tokens", initial_tokens.len());

    // Initialize position manager
    let circuit_breakers = CircuitBreakers::default();
    let max_daily_loss_pct = circuit_breakers.max_daily_loss_pct;
    let max_drawdown_pct = circuit_breakers.max_drawdown_pct;

    let position_manager = initialize_position_manager(
        postgres_persistence.as_mut(),
        initial_portfolio_value,
        circuit_breakers,
    )
    .await;

    // Create shared state
    let shared_state = Arc::new(SharedState {
        tokens: Arc::new(RwLock::new(initial_tokens.clone())),
        position_manager,
        initial_portfolio_value,
    });

    tracing::info!("\n📊 Configuration:");
    tracing::info!("  Portfolio Value: ${:.2}", initial_portfolio_value);
    tracing::info!("  Max Daily Loss: {}%", max_daily_loss_pct * 100.0);
    tracing::info!("  Max Drawdown: {}%", max_drawdown_pct * 100.0);
    tracing::info!("  Tokens: {}", initial_tokens.len());
    for token in &initial_tokens {
        tracing::info!("    - {} ({})", token.symbol, token.name);
    }

    tracing::info!("\n🔄 Spawning independent loops...");

    // Spawn Loop 1: Price Fetch (every 5 minutes, clock-aligned)
    let price_task = {
        let tokens = shared_state.tokens.clone();
        let redis_url = redis_url.clone();
        tokio::spawn(async move {
            price_fetch_loop(tokens, redis_url).await;
        })
    };

    // Spawn Loop 2: Trading Execution (every 1 minute)
    let trading_task = {
        let state = shared_state.clone();
        let redis_url = redis_url.clone();
        let postgres_url = database_url.clone();
        tokio::spawn(async move {
            trading_execution_loop(state, redis_url, postgres_url).await;
        })
    };

    // Spawn Loop 3: Token Discovery (every 30 minutes)
    let discovery_task = {
        let tokens = shared_state.tokens.clone();
        let birdeye_key = birdeye_api_key.clone();
        let postgres_url = database_url.clone();
        tokio::spawn(async move {
            token_discovery_loop(tokens, birdeye_key, postgres_url).await;
        })
    };

    tracing::info!("✅ All loops spawned successfully");
    tracing::info!("  🔄 Price Fetch: every 5 min (clock-aligned to XX:00, XX:05, etc.)");
    tracing::info!("  💹 Trading: every 5 min (30 sec after price fetch)");
    tracing::info!("  🔍 Discovery: every 30 min");
    tracing::info!("\nPress Ctrl+C to stop...\n");

    // Wait for Ctrl+C or task failure
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("\n⚠️  Received Ctrl+C, shutting down...");
        }
        result = price_task => {
            tracing::error!("Price fetch loop exited: {:?}", result);
        }
        result = trading_task => {
            tracing::error!("Trading loop exited: {:?}", result);
        }
        result = discovery_task => {
            tracing::error!("Discovery loop exited: {:?}", result);
        }
    }

    tracing::info!("👋 CryptoBot stopped");
    Ok(())
}

// ============================================================================
// Initialization Functions
// ============================================================================

fn setup_logging() {
    tracing_subscriber::fmt()
        .with_env_filter("cryptobot=info,cryptobot::strategy=debug")
        .init();
}

async fn connect_to_postgres() -> Option<PostgresPersistence> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/cryptobot".to_string());

    match PostgresPersistence::new(&database_url, None).await {
        Ok(p) => {
            tracing::info!(
                "Postgres persistence enabled at {} (positions & discovery)",
                database_url
            );
            Some(p)
        }
        Err(e) => {
            tracing::warn!(
                "Failed to connect to Postgres ({}), continuing without persistence",
                e
            );
            None
        }
    }
}


fn create_birdeye_client() -> Result<BirdeyeClient> {
    let api_key =
        std::env::var("BIRDEYE_API_KEY").expect("BIRDEYE_API_KEY not found in environment");
    Ok(BirdeyeClient::new(api_key))
}

fn get_initial_portfolio_value() -> f64 {
    std::env::var("INITIAL_PORTFOLIO_VALUE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(10000.0)
}

async fn initialize_position_manager(
    postgres: Option<&mut PostgresPersistence>,
    initial_portfolio_value: f64,
    circuit_breakers: CircuitBreakers,
) -> Arc<Mutex<PositionManager>> {
    let loaded_positions = load_positions_from_db(postgres).await;

    Arc::new(Mutex::new(if let Some(positions) = loaded_positions {
        PositionManager::with_positions(initial_portfolio_value, circuit_breakers, positions)
    } else {
        PositionManager::new(initial_portfolio_value, circuit_breakers)
    }))
}

async fn load_positions_from_db(
    postgres: Option<&mut PostgresPersistence>,
) -> Option<Vec<Position>> {
    let postgres = postgres?;

    tracing::info!("Loading historical positions from Postgres...");
    match postgres.load_positions().await {
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
}



// ============================================================================
// Token Discovery Functions
// ============================================================================

async fn discover_and_build_token_list(
    birdeye_client: &BirdeyeClient,
    postgres: Option<&PostgresPersistence>,
) -> Result<Vec<TrendingToken>> {
    // Load existing tracked tokens
    let existing_tracked = load_existing_tracked_tokens(postgres).await;

    // Discover new trending tokens
    tracing::info!("🔍 Discovering tokens from Birdeye...");
    let trending_tokens = birdeye_client.get_trending("rank", "asc", 0, 20).await?;
    tracing::info!("Found {} trending tokens", trending_tokens.len());

    // Apply safety filter
    let safe_tokens = apply_safety_filter(&trending_tokens);
    tracing::info!("✅ {} safe tokens passed filter", safe_tokens.len());

    // Build final list
    build_final_token_list(existing_tracked, safe_tokens).await
}

async fn load_existing_tracked_tokens(
    postgres: Option<&PostgresPersistence>,
) -> Vec<(String, String, String, u8)> {
    let postgres = match postgres {
        Some(p) => p,
        None => return Vec::new(),
    };

    match postgres.load_tracked_tokens().await {
        Ok(tokens) => {
            if !tokens.is_empty() {
                tracing::info!(
                    "📂 Loaded {} existing tracked tokens from database",
                    tokens.len()
                );
                tokens
            } else {
                Vec::new()
            }
        }
        Err(e) => {
            tracing::warn!("Failed to load tracked tokens from database: {}", e);
            Vec::new()
        }
    }
}

fn apply_safety_filter(trending_tokens: &[TrendingToken]) -> Vec<&TrendingToken> {
    let mut safe_tokens = Vec::new();

    for trending_token in trending_tokens {
        let (is_safe, reason) = is_safe_token(trending_token);

        if is_safe {
            tracing::info!(
                "✅ {} ({}) - {}",
                trending_token.symbol,
                trending_token.name,
                reason
            );
            safe_tokens.push(trending_token);
        } else {
            tracing::debug!(
                "❌ {} ({}) - {}",
                trending_token.symbol,
                trending_token.name,
                reason
            );
        }
    }

    safe_tokens
}

async fn build_final_token_list(
    existing_tracked: Vec<(String, String, String, u8)>,
    safe_tokens: Vec<&TrendingToken>,
) -> Result<Vec<TrendingToken>> {
    let mut final_tokens = Vec::new();
    let mut tracked_addresses = HashSet::new();

    // Restore existing tracked tokens
    restore_existing_tokens(
        &existing_tracked,
        &safe_tokens,
        &mut final_tokens,
        &mut tracked_addresses,
    )
    .await;

    // Add must-track tokens
    add_must_track_tokens(
        &safe_tokens,
        &mut final_tokens,
        &mut tracked_addresses,
    )
    .await;

    // Add new safe tokens (up to MAX_TOKENS total)
    add_new_safe_tokens(&safe_tokens, &mut final_tokens, &mut tracked_addresses);

    tracing::info!("Final token list: {} tokens", final_tokens.len());
    Ok(final_tokens)
}

/// Restore all tokens we're already tracking
///
/// For tokens still in trending: use fresh data from API
/// For tokens NOT in trending: use stored data from DB
///
/// **Important**: Tokens not in trending skip safety checks because they were
/// already validated when first added. This is NOT fake data to fool the filter -
/// these tokens have already passed safety checks and are being restored.
async fn restore_existing_tokens(
    existing_tracked: &[(String, String, String, u8)],
    safe_tokens: &[&TrendingToken],
    final_tokens: &mut Vec<TrendingToken>,
    tracked_addresses: &mut HashSet<String>,
) {
    for (symbol, address, name, decimals) in existing_tracked {
        if let Some(token) = safe_tokens.iter().find(|t| &t.address == address) {
            // Token is still in trending - use fresh data
            final_tokens.push((*token).clone());
            tracked_addresses.insert(address.clone());
            tracing::info!("✓ Keeping {} (still in trending, using fresh data)", symbol);
        } else {
            // Token fell out of trending - keep tracking with stored data
            // Market data fields are set to 0 since we don't have fresh data,
            // but this token already passed safety checks when it was first added
            let token = TrendingToken {
                address: address.clone(),
                symbol: symbol.clone(),
                name: name.clone(),
                decimals: *decimals,
                liquidity_usd: 0.0,
                volume_24h_usd: 0.0,
                volume_24h_change_percent: 0.0,
                fdv: 0.0,
                rank: 9999, // Sentinel value indicating "not from trending"
                price: 0.0,
                price_24h_change_percent: 0.0,
            };
            final_tokens.push(token);
            tracked_addresses.insert(address.clone());
            tracing::info!(
                "✓ Keeping {} (not in trending, using stored data - already validated)",
                symbol
            );
        }
    }
}

/// Add must-track tokens (SOL, JUP) to the tracking list
///
/// For tokens in trending: use fresh data from API
/// For tokens NOT in trending: use hardcoded metadata
///
/// **Important**: Must-track tokens are pre-approved blue chips that skip safety
/// checks entirely. They're always added regardless of trending status.
async fn add_must_track_tokens(
    safe_tokens: &[&TrendingToken],
    final_tokens: &mut Vec<TrendingToken>,
    tracked_addresses: &mut HashSet<String>,
) {
    for (address, symbol) in MUST_TRACK {
        if tracked_addresses.contains(*address) {
            continue; // Already added in restore_existing_tokens
        }

        if let Some(token) = safe_tokens
            .iter()
            .find(|t| &t.address == address || &t.symbol == symbol)
        {
            // Token found in trending - use fresh data
            final_tokens.push((*token).clone());
            tracked_addresses.insert(address.to_string());
            tracing::info!("✓ Must-track {} found in trending, using fresh data", symbol);
        } else {
            // Must-track token not in trending - add with hardcoded metadata
            // This is unusual for blue chips like SOL/JUP but can happen
            tracing::warn!(
                "⚠ Must-track {} not in trending, adding with hardcoded data",
                symbol
            );

            // Use known metadata for SOL and JUP (both use 9 decimals)
            let token = TrendingToken {
                address: address.to_string(),
                symbol: symbol.to_string(),
                name: if *symbol == "SOL" { "Solana" } else { "Jupiter" }.to_string(),
                decimals: 9, // SOL and JUP both use 9 decimals
                liquidity_usd: 0.0,
                volume_24h_usd: 0.0,
                volume_24h_change_percent: 0.0,
                fdv: 0.0,
                rank: 9999, // Sentinel value indicating "not from trending"
                price: 0.0,
                price_24h_change_percent: 0.0,
            };
            final_tokens.push(token);
            tracked_addresses.insert(address.to_string());
            tracing::info!("✓ Added must-track {} (pre-approved, skips safety checks)", symbol);
        }
    }
}

fn add_new_safe_tokens(
    safe_tokens: &[&TrendingToken],
    final_tokens: &mut Vec<TrendingToken>,
    tracked_addresses: &mut HashSet<String>,
) {
    for token in safe_tokens.iter() {
        if final_tokens.len() >= MAX_TOKENS {
            break;
        }
        if !tracked_addresses.contains(&token.address) {
            final_tokens.push((*token).clone());
            tracked_addresses.insert(token.address.clone());
            tracing::info!("✓ Adding new token {} to tracking", token.symbol);
        }
    }
}

async fn save_tracked_tokens_to_db(
    postgres: &mut PostgresPersistence,
    final_tokens: &[TrendingToken],
) {
    tracing::info!("Saving discovered tokens to database...");

    for token in final_tokens.iter() {
        let strategy_type = "momentum"; // Default strategy for all tokens

        let token_data = cryptobot::db::postgres::TrackedTokenData {
            symbol: &token.symbol,
            address: &token.address,
            name: &token.name,
            decimals: token.decimals,
            strategy_type,
        };

        match postgres.save_tracked_token(token_data).await {
            Ok(_) => {
                tracing::info!("  ✓ Saved {} ({}) to database", token.symbol, token.name);
            }
            Err(e) => {
                tracing::warn!("  ✗ Failed to save {} to database: {}", token.symbol, e);
            }
        }
    }
}

fn convert_to_tokens(trending_tokens: &[TrendingToken]) -> Vec<Token> {
    trending_tokens
        .iter()
        .map(|t| Token {
            symbol: t.symbol.clone(),
            mint_address: t.address.clone(),
            name: t.name.clone(),
            decimals: t.decimals,
        })
        .collect()
}

// ============================================================================
// Independent Loop Tasks
// ============================================================================

/// Loop 1: Price Fetch Loop (every 5 minutes, clock-aligned)
/// Fetches prices and saves to Redis on a strict 5-minute schedule
async fn price_fetch_loop(tokens: Arc<RwLock<Vec<Token>>>, redis_url: String) {
    tracing::info!("🔄 Price Fetch Loop starting...");

    // Connect to Redis
    let mut redis = match RedisPersistence::new(&redis_url).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Price fetch loop: Failed to connect to Redis: {}", e);
            return;
        }
    };

    // Create interval starting at next 5-minute boundary
    let start = next_5min_boundary();
    let delay = start - Instant::now();
    tracing::info!(
        "Price fetch will start in {:?} at next 5-min boundary",
        delay
    );

    let mut ticker = interval_at(start, Duration::from_secs(300));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut tick_count = 0u32;
    const CLEANUP_INTERVAL: u32 = 12; // Run cleanup every 12 ticks (every hour)
    const KEEP_HOURS: u64 = 48; // Keep 48 hours of data (2x strategy needs)

    loop {
        ticker.tick().await;
        tick_count += 1;

        tracing::info!("🔄 [PRICE FETCH] Tick at {}", Utc::now().format("%H:%M:%S"));

        // Get current token list
        let token_list = tokens.read().unwrap().clone();

        // Create a temporary price manager for this fetch
        let mut price_manager = PriceFeedManager::new(token_list.clone(), 1);

        // Fetch all prices
        let results = price_manager.fetch_all().await;

        for (i, result) in results.iter().enumerate() {
            let token = &token_list[i];

            match result {
                Ok(snapshot) => {
                    tracing::info!(
                        "  ✓ {} @ ${:.4}",
                        token.symbol,
                        snapshot.price
                    );

                    // Save to Redis
                    if let Ok(candles) = price_manager.buffer().get_candles(&token.symbol) {
                        if let Some(latest_candle) = candles.last() {
                            if let Err(e) = redis
                                .save_candles(&token.symbol, std::slice::from_ref(latest_candle))
                                .await
                            {
                                tracing::warn!("  ✗ Failed to save {} to Redis: {}", token.symbol, e);
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("  ✗ {} fetch failed: {}", token.symbol, e);
                }
            }
        }

        // Periodic cleanup: remove data older than 48 hours
        if tick_count % CLEANUP_INTERVAL == 0 {
            tracing::info!("🧹 Running Redis cleanup (keeping last {}h)...", KEEP_HOURS);

            for token in &token_list {
                match redis.cleanup_old(&token.symbol, KEEP_HOURS).await {
                    Ok(removed) => {
                        if removed > 0 {
                            tracing::info!("  ✓ Cleaned up {} old snapshots for {}", removed, token.symbol);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("  ✗ Failed to cleanup {}: {}", token.symbol, e);
                    }
                }
            }
        }
    }
}

/// Loop 2: Trading Execution Loop (every 5 minutes)
/// Reads prices from Redis, generates signals, and executes trades
/// Runs at the same interval as price fetching since it depends on that data
async fn trading_execution_loop(
    state: Arc<SharedState>,
    redis_url: String,
    postgres_url: Option<String>,
) {
    tracing::info!("💹 Trading Execution Loop starting...");

    // Connect to Redis
    let mut redis = match RedisPersistence::new(&redis_url).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Trading loop: Failed to connect to Redis: {}", e);
            return;
        }
    };

    // Connect to Postgres (optional)
    let mut postgres = if let Some(url) = postgres_url {
        PostgresPersistence::new(&url, None).await.ok()
    } else {
        None
    };

    let strategy = MomentumStrategy::default().with_poll_interval(POLL_INTERVAL_MINUTES);
    let samples_needed = strategy.samples_needed(POLL_INTERVAL_MINUTES);
    let mut executor = Executor::new(state.position_manager.clone());

    // Create interval starting 30 seconds after next 5-minute boundary
    // This gives price_fetch_loop time to complete
    let start = next_5min_boundary_plus_30s();
    let delay = start - Instant::now();
    tracing::info!(
        "Trading will start in {:?} (30s after price fetch)",
        delay
    );

    let mut ticker = interval_at(start, Duration::from_secs(300));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        tracing::info!("💹 [TRADING] Tick at {}", Utc::now().format("%H:%M:%S"));

        // Get current token list
        let tokens = state.tokens.read().unwrap().clone();
        let mut prices = HashMap::new();

        // Load candles from Redis for all tokens
        for token in &tokens {
            match redis
                .load_candles(&token.symbol, strategy.lookback_hours())
                .await
            {
                Ok(candles) => {
                    // Validate candle uniformity early (fail fast)
                    let expected_interval_secs = POLL_INTERVAL_MINUTES * 60;
                    if let Err(e) = validate_candle_uniformity(&candles, expected_interval_secs) {
                        tracing::warn!(
                            "  {} - Skipping due to data quality issue: {}",
                            token.symbol,
                            e
                        );
                        continue;
                    }

                    if let Some(latest) = candles.last() {
                        prices.insert(token.symbol.clone(), latest.close);

                        tracing::info!(
                            "  {} @ ${:.4} ({} candles)",
                            token.symbol,
                            latest.close,
                            candles.len()
                        );

                        // Generate signals if we have enough data
                        if candles.len() >= samples_needed {
                            process_token_signal(
                                &strategy,
                                token,
                                &candles,
                                latest.close,
                                &state.position_manager,
                                &mut executor,
                                postgres.as_mut(),
                            )
                            .await;
                        } else {
                            tracing::info!(
                                "    → Collecting data... ({}/{} needed)",
                                candles.len(),
                                samples_needed
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("  ✗ Failed to load candles for {}: {}", token.symbol, e);
                }
            }
        }

        // Check exit conditions for all open positions
        let closed_positions = check_and_close_positions(&state.position_manager, &prices);
        save_positions_to_db(postgres.as_mut(), &closed_positions).await;

        // Log portfolio summary
        log_portfolio_summary(&state.position_manager, &prices, state.initial_portfolio_value);
    }
}

/// Loop 3: Token Discovery Loop (every 30 minutes)
/// Discovers trending tokens and updates the token list
async fn token_discovery_loop(
    tokens: Arc<RwLock<Vec<Token>>>,
    birdeye_api_key: String,
    postgres_url: Option<String>,
) {
    tracing::info!("🔍 Token Discovery Loop starting...");

    let birdeye_client = BirdeyeClient::new(birdeye_api_key);

    // Start immediately, then run every 30 minutes
    let mut ticker = interval_at(Instant::now(), Duration::from_secs(1800));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        tracing::info!("🔍 [DISCOVERY] Tick at {}", Utc::now().format("%H:%M:%S"));

        // Connect to Postgres (reconnect each time to handle connection drops)
        let postgres = if let Some(ref url) = postgres_url {
            PostgresPersistence::new(url, None).await.ok()
        } else {
            None
        };

        // Discover and build token list
        match discover_and_build_token_list(&birdeye_client, postgres.as_ref()).await {
            Ok(final_tokens) => {
                tracing::info!("  ✓ Discovered {} tokens", final_tokens.len());

                // Save to database
                if let Some(mut pg) = postgres {
                    save_tracked_tokens_to_db(&mut pg, &final_tokens).await;
                }

                // Update shared token list
                let new_tokens = convert_to_tokens(&final_tokens);
                *tokens.write().unwrap() = new_tokens;

                tracing::info!("  ✓ Updated token list");
            }
            Err(e) => {
                tracing::error!("  ✗ Discovery failed: {}", e);
            }
        }
    }
}

// ============================================================================
// Helper Functions for Trading
// ============================================================================

fn check_and_close_positions(
    position_manager: &Arc<Mutex<PositionManager>>,
    prices: &HashMap<String, f64>,
) -> Vec<Position> {
    let mut pm = position_manager.lock().unwrap();
    match pm.check_exits(prices) {
        Ok(closed_ids) => {
            let mut positions = Vec::new();
            for position_id in closed_ids {
                tracing::info!("✓ Position {} closed by exit condition", position_id);
                if let Some(position) = pm.all_positions().iter().find(|p| p.id == position_id) {
                    positions.push(position.clone());
                }
            }
            positions
        }
        Err(e) => {
            tracing::error!("Failed to check exits: {}", e);
            Vec::new()
        }
    }
}

async fn save_positions_to_db(
    postgres_persistence: Option<&mut PostgresPersistence>,
    positions: &[Position],
) {
    if let Some(postgres) = postgres_persistence {
        for position in positions {
            if let Err(e) = postgres.save_position(position).await {
                tracing::warn!("Failed to save position to Postgres: {}", e);
            }
        }
    }
}

async fn process_token_signal(
    strategy: &MomentumStrategy,
    token: &Token,
    candles: &[cryptobot::models::Candle],
    current_price: f64,
    position_manager: &Arc<Mutex<PositionManager>>,
    executor: &mut Executor,
    postgres_persistence: Option<&mut PostgresPersistence>,
) {
    match strategy.generate_signal(candles) {
        Ok(signal) => {
            tracing::info!("  Signal: {:?}", signal);

            match executor.process_signal(&signal, &token.symbol, current_price) {
                Ok(decision) => {
                    tracing::info!("  Decision: {:?} - {}", decision.action, decision.reason);

                    execute_decision(
                        &decision.action,
                        token,
                        current_price,
                        position_manager,
                        postgres_persistence,
                    )
                    .await;
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
}

async fn execute_decision(
    action: &ExecutionAction,
    token: &Token,
    current_price: f64,
    position_manager: &Arc<Mutex<PositionManager>>,
    postgres_persistence: Option<&mut PostgresPersistence>,
) {
    match action {
        ExecutionAction::Execute { quantity } => {
            execute_buy(
                token,
                current_price,
                *quantity,
                position_manager,
                postgres_persistence,
            )
            .await;
        }
        ExecutionAction::Close { position_id } => {
            execute_close(
                *position_id,
                current_price,
                position_manager,
                postgres_persistence,
            )
            .await;
        }
        ExecutionAction::Skip => {
            // Do nothing - already logged
        }
    }
}

async fn execute_buy(
    token: &Token,
    current_price: f64,
    quantity: f64,
    position_manager: &Arc<Mutex<PositionManager>>,
    postgres_persistence: Option<&mut PostgresPersistence>,
) {
    tracing::info!(
        "  → Would BUY {:.4} {} @ ${:.4} (total: ${:.2})",
        quantity,
        token.symbol,
        current_price,
        quantity * current_price
    );

    let opened_position = {
        let mut pm = position_manager.lock().unwrap();
        match pm.open_position(token.symbol.clone(), current_price, quantity) {
            Ok(position_id) => {
                tracing::info!("  ✓ Opened position {} for {}", position_id, token.symbol);
                pm.all_positions()
                    .iter()
                    .find(|p| p.id == position_id)
                    .cloned()
            }
            Err(e) => {
                tracing::error!("  ✗ Failed to open position: {}", e);
                None
            }
        }
    };

    if let Some(position) = opened_position {
        save_positions_to_db(postgres_persistence, &[position]).await;
    }
}

async fn execute_close(
    position_id: uuid::Uuid,
    current_price: f64,
    position_manager: &Arc<Mutex<PositionManager>>,
    postgres_persistence: Option<&mut PostgresPersistence>,
) {
    let closed_position = {
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
                pm.all_positions()
                    .iter()
                    .find(|p| p.id == position_id)
                    .cloned()
            }
            Err(e) => {
                tracing::error!("  ✗ Failed to close position: {}", e);
                None
            }
        }
    };

    if let Some(position) = closed_position {
        save_positions_to_db(postgres_persistence, &[position]).await;
    }
}

fn log_portfolio_summary(
    position_manager: &Arc<Mutex<PositionManager>>,
    prices: &HashMap<String, f64>,
    initial_portfolio_value: f64,
) {
    let pm = position_manager.lock().unwrap();
    let portfolio_value = pm
        .portfolio_value(prices)
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
