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
use cryptobot::strategy::Strategy;
use cryptobot::Result;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::time::{interval, Duration};

// Must-track tokens (always include these)
const MUST_TRACK: &[(&str, &str)] = &[
    ("So11111111111111111111111111111111111111112", "SOL"),
    ("JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN", "JUP"),
];

const MAX_TOKENS: usize = 10;
const POLL_INTERVAL_MINUTES: u64 = 5;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    setup_logging();

    tracing::info!("CryptoBot starting - Discovery Mode");

    // Initialize persistence layers
    let mut postgres_persistence = connect_to_postgres().await;
    let mut redis_persistence = connect_to_redis().await;

    // Initialize Birdeye client
    let birdeye_client = create_birdeye_client()?;

    // Discover and build token list
    let final_tokens =
        discover_and_build_token_list(&birdeye_client, postgres_persistence.as_ref()).await?;

    // Save discovered tokens to database
    if let Some(ref mut postgres) = postgres_persistence {
        save_tracked_tokens_to_db(postgres, &final_tokens).await;
    }

    // Convert to Token structs for trading
    let tokens = convert_to_tokens(&final_tokens);
    if tokens.is_empty() {
        return Err("No safe tokens found! Cannot start bot.".into());
    }

    // Initialize strategy and price manager
    let strategy = MomentumStrategy::default().with_poll_interval(POLL_INTERVAL_MINUTES);
    let samples_needed = strategy.samples_needed(POLL_INTERVAL_MINUTES);
    let buffer_size = (samples_needed as f64 * 1.2) as usize;
    let mut price_manager = PriceFeedManager::new(tokens.clone(), buffer_size);

    // Load historical candles from Redis
    if let Some(ref mut redis) = redis_persistence {
        load_historical_candles(redis, &mut price_manager, &tokens, &strategy).await;
    }

    // Initialize position manager and executor
    let initial_portfolio_value = get_initial_portfolio_value();
    let circuit_breakers = CircuitBreakers::default();

    log_startup_info(
        &tokens,
        &strategy,
        samples_needed,
        initial_portfolio_value,
        &circuit_breakers,
    );

    let position_manager = initialize_position_manager(
        postgres_persistence.as_mut(),
        initial_portfolio_value,
        circuit_breakers,
    )
    .await;

    let mut executor = Executor::new(position_manager.clone());

    // Main event loop
    run_main_loop(
        &mut price_manager,
        &tokens,
        &strategy,
        samples_needed,
        position_manager,
        &mut executor,
        redis_persistence.as_mut(),
        postgres_persistence.as_mut(),
        initial_portfolio_value,
    )
    .await
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

async fn connect_to_redis() -> Option<RedisPersistence> {
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    match RedisPersistence::new(&redis_url).await {
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

async fn load_historical_candles(
    redis: &mut RedisPersistence,
    price_manager: &mut PriceFeedManager,
    tokens: &[Token],
    strategy: &MomentumStrategy,
) {
    tracing::info!("Loading historical candle data from Redis...");

    for token in tokens {
        match redis
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

fn log_startup_info(
    tokens: &[Token],
    strategy: &MomentumStrategy,
    samples_needed: usize,
    initial_portfolio_value: f64,
    circuit_breakers: &CircuitBreakers,
) {
    tracing::info!("Watching {} tokens:", tokens.len());
    for token in tokens {
        tracing::info!("  - {} ({})", token.symbol, token.name);
    }

    tracing::info!(
        "Strategy: {} | Lookback: {}h | Poll: {}min | Need {} snapshots",
        strategy.name(),
        strategy.lookback_hours(),
        POLL_INTERVAL_MINUTES,
        samples_needed
    );

    tracing::info!(
        "Portfolio: ${:.2} | Circuit Breakers: max_daily_loss={}%, max_drawdown={}%",
        initial_portfolio_value,
        circuit_breakers.max_daily_loss_pct * 100.0,
        circuit_breakers.max_drawdown_pct * 100.0
    );
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
    build_final_token_list(birdeye_client, existing_tracked, safe_tokens).await
}

async fn load_existing_tracked_tokens(
    postgres: Option<&PostgresPersistence>,
) -> Vec<(String, String, String)> {
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
    birdeye_client: &BirdeyeClient,
    existing_tracked: Vec<(String, String, String)>,
    safe_tokens: Vec<&TrendingToken>,
) -> Result<Vec<TrendingToken>> {
    let mut final_tokens = Vec::new();
    let mut tracked_addresses = HashSet::new();

    // Restore existing tracked tokens
    restore_existing_tokens(
        birdeye_client,
        &existing_tracked,
        &safe_tokens,
        &mut final_tokens,
        &mut tracked_addresses,
    )
    .await;

    // Add must-track tokens
    add_must_track_tokens(
        birdeye_client,
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

async fn restore_existing_tokens(
    birdeye_client: &BirdeyeClient,
    existing_tracked: &[(String, String, String)],
    safe_tokens: &[&TrendingToken],
    final_tokens: &mut Vec<TrendingToken>,
    tracked_addresses: &mut HashSet<String>,
) {
    for (symbol, address, name) in existing_tracked {
        // Try to find updated data in safe_tokens
        if let Some(token) = safe_tokens.iter().find(|t| &t.address == address) {
            final_tokens.push((*token).clone());
            tracked_addresses.insert(address.clone());
            tracing::info!("✓ Keeping tracked token {} (found in discovery)", symbol);
        } else {
            // Fetch fresh data for this token
            if let Some(token) = fetch_token_data(birdeye_client, address, symbol, name).await {
                final_tokens.push(token);
                tracked_addresses.insert(address.clone());
            }
        }
    }
}

async fn add_must_track_tokens(
    birdeye_client: &BirdeyeClient,
    safe_tokens: &[&TrendingToken],
    final_tokens: &mut Vec<TrendingToken>,
    tracked_addresses: &mut HashSet<String>,
) {
    for (address, symbol) in MUST_TRACK {
        if tracked_addresses.contains(*address) {
            continue; // Already added
        }

        if let Some(token) = safe_tokens
            .iter()
            .find(|t| &t.address == address || &t.symbol == symbol)
        {
            final_tokens.push((*token).clone());
            tracked_addresses.insert(address.to_string());
            tracing::info!("✓ Must-track token {} found in discovery", symbol);
        } else {
            // Fetch manually
            if let Some(token) = fetch_must_track_token(birdeye_client, address, symbol).await {
                final_tokens.push(token);
                tracked_addresses.insert(address.to_string());
            }
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

async fn fetch_token_data(
    birdeye_client: &BirdeyeClient,
    address: &str,
    symbol: &str,
    name: &str,
) -> Option<TrendingToken> {
    tracing::info!("📡 Fetching fresh data for tracked token {}...", symbol);

    match birdeye_client.get_price(address).await {
        Ok((price, liquidity)) => {
            let token = TrendingToken {
                address: address.to_string(),
                symbol: symbol.to_string(),
                name: name.to_string(),
                decimals: 9,
                liquidity_usd: liquidity.unwrap_or(0.0),
                volume_24h_usd: 1_000_000.0,
                volume_24h_change_percent: 0.0,
                fdv: 10_000_000.0,
                marketcap: 10_000_000.0,
                rank: 999,
                price,
                price_24h_change_percent: 0.0,
            };
            tracing::info!(
                "✓ Keeping tracked token {} (fetched at ${:.2})",
                symbol,
                price
            );
            Some(token)
        }
        Err(e) => {
            tracing::warn!("⚠ Failed to fetch data for {}, skipping: {}", symbol, e);
            None
        }
    }
}

async fn fetch_must_track_token(
    birdeye_client: &BirdeyeClient,
    address: &str,
    symbol: &str,
) -> Option<TrendingToken> {
    tracing::warn!(
        "⚠ Must-track token {} not found in discovery, fetching manually...",
        symbol
    );

    match birdeye_client.get_price(address).await {
        Ok((price, liquidity)) => {
            let token = TrendingToken {
                address: address.to_string(),
                symbol: symbol.to_string(),
                name: match symbol {
                    "SOL" => "Solana".to_string(),
                    "JUP" => "Jupiter".to_string(),
                    _ => symbol.to_string(),
                },
                decimals: match symbol {
                    "SOL" => 9,
                    "JUP" => 6,
                    _ => 9,
                },
                liquidity_usd: liquidity.unwrap_or(0.0),
                volume_24h_usd: 1_000_000_000.0,
                volume_24h_change_percent: 0.0,
                fdv: 100_000_000_000.0,
                marketcap: 100_000_000_000.0,
                rank: 1,
                price,
                price_24h_change_percent: 0.0,
            };
            tracing::info!("✓ Manually added {} at ${:.2}", symbol, price);
            Some(token)
        }
        Err(e) => {
            tracing::error!("✗ Failed to fetch must-track token {}: {}", symbol, e);
            None
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
// Main Event Loop
// ============================================================================

#[allow(clippy::too_many_arguments)]
async fn run_main_loop(
    price_manager: &mut PriceFeedManager,
    tokens: &[Token],
    strategy: &MomentumStrategy,
    samples_needed: usize,
    position_manager: Arc<Mutex<PositionManager>>,
    executor: &mut Executor,
    mut redis_persistence: Option<&mut RedisPersistence>,
    mut postgres_persistence: Option<&mut PostgresPersistence>,
    initial_portfolio_value: f64,
) -> Result<()> {
    let poll_interval_secs = POLL_INTERVAL_MINUTES * 60;
    let mut ticker = interval(Duration::from_secs(poll_interval_secs));

    loop {
        ticker.tick().await;
        tracing::info!("=== Tick ===");

        // Fetch all prices and collect into HashMap
        let prices =
            fetch_prices_and_save_to_redis(price_manager, tokens, redis_persistence.as_deref_mut())
                .await;

        // Check for exit conditions and close positions
        let closed_positions = check_and_close_positions(&position_manager, &prices);
        save_positions_to_db(postgres_persistence.as_deref_mut(), &closed_positions).await;

        // Process signals for each token
        process_all_token_signals(
            price_manager,
            tokens,
            strategy,
            samples_needed,
            &prices,
            &position_manager,
            executor,
            postgres_persistence.as_deref_mut(),
        )
        .await;

        // Log portfolio summary
        log_portfolio_summary(&position_manager, &prices, initial_portfolio_value);

        tracing::info!("=== End Tick ===\n");
    }
}

async fn fetch_prices_and_save_to_redis(
    price_manager: &mut PriceFeedManager,
    tokens: &[Token],
    mut redis_persistence: Option<&mut RedisPersistence>,
) -> HashMap<String, f64> {
    let results = price_manager.fetch_all().await;
    let mut prices = HashMap::new();

    for (i, result) in results.iter().enumerate() {
        let token = &tokens[i];

        match result {
            Ok(snapshot) => {
                prices.insert(token.symbol.clone(), snapshot.price);

                // Save to Redis if available
                if let Some(redis) = redis_persistence.as_deref_mut() {
                    save_latest_candle_to_redis(redis, price_manager, &token.symbol).await;
                }
            }
            Err(e) => {
                tracing::error!("{}: Failed to fetch price: {}", token.symbol, e);
            }
        }
    }

    prices
}

async fn save_latest_candle_to_redis(
    redis: &mut RedisPersistence,
    price_manager: &PriceFeedManager,
    symbol: &str,
) {
    if let Ok(candles) = price_manager.buffer().get_candles(symbol) {
        if let Some(latest_candle) = candles.last() {
            match redis
                .save_candles(symbol, std::slice::from_ref(latest_candle))
                .await
            {
                Ok(_) => {
                    tracing::debug!(
                        "Saved {} snapshot to Redis (total: {})",
                        symbol,
                        candles.len()
                    );
                }
                Err(e) => {
                    tracing::warn!("Failed to save snapshot to Redis for {}: {}", symbol, e);
                }
            }
        }
    }
}

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

#[allow(clippy::too_many_arguments)]
async fn process_all_token_signals(
    price_manager: &PriceFeedManager,
    tokens: &[Token],
    strategy: &MomentumStrategy,
    samples_needed: usize,
    prices: &HashMap<String, f64>,
    position_manager: &Arc<Mutex<PositionManager>>,
    executor: &mut Executor,
    mut postgres_persistence: Option<&mut PostgresPersistence>,
) {
    for token in tokens {
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

            if candles.len() >= samples_needed {
                process_token_signal(
                    strategy,
                    token,
                    &candles,
                    current_price,
                    position_manager,
                    executor,
                    postgres_persistence.as_deref_mut(),
                )
                .await;
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
