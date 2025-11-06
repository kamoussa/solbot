use chrono::{Timelike, Utc};
use clap::{Parser, Subcommand};
use cryptobot::api::birdeye::{BirdeyeClient, TrendingToken};
use cryptobot::api::CoinGeckoClient;
use cryptobot::backfill::backfill_token;
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
// CLI
// ============================================================================

/// CryptoBot - Cryptocurrency trading bot
#[derive(Parser, Debug)]
#[command(name = "cryptobot")]
#[command(about = "Cryptocurrency trading bot for Solana tokens", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Backfill historical data for a token
    Backfill {
        /// Token symbol (e.g., SOL, JUP)
        symbol: String,

        /// Token mint address
        address: String,

        /// Number of days to backfill (default: 7)
        #[arg(short, long, default_value = "7")]
        days: u32,

        /// Force overwrite existing data
        #[arg(short, long)]
        force: bool,

        /// Use range API for chunked backfill (better for 90+ days)
        #[arg(long)]
        chunked: bool,

        /// Chunk size in days when using chunked mode (default: 90)
        #[arg(long, default_value = "90")]
        chunk_size: u32,
    },
}

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

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Backfill {
            symbol,
            address,
            days,
            force,
            chunked,
            chunk_size,
        }) => run_backfill(&symbol, &address, days, force, chunked, chunk_size).await,
        None => run_bot().await,
    }
}

async fn run_bot() -> Result<()> {
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

    tracing::info!(
        "✅ Initial discovery complete: {} tokens",
        initial_tokens.len()
    );

    // Initialize and run backfill for tokens with insufficient data
    initialize_and_run_backfill(&final_tokens, &redis_url).await;

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

    // Spawn Loop 3: Token Discovery (every 4 hours)
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
    tracing::info!("  🔍 Discovery: every 4 hours");
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

async fn run_backfill(symbol: &str, address: &str, days: u32, force: bool, chunked: bool, chunk_size: u32) -> Result<()> {
    tracing::info!(
        "📥 Backfill Mode: {} ({}) for {} days",
        symbol,
        address,
        days
    );
    if force {
        tracing::info!("  Force mode: will overwrite existing data");
    }
    if chunked {
        tracing::info!("  Chunked mode: using {}-day chunks with range API", chunk_size);
    }

    // Get required environment variables
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let coingecko_api_key =
        std::env::var("COINGECKO_API_KEY").expect("COINGECKO_API_KEY not found in environment");

    // Initialize CoinGecko client
    tracing::info!("🔄 Initializing CoinGecko client...");
    let coingecko = CoinGeckoClient::new(coingecko_api_key).await?;

    // Initialize Redis persistence
    tracing::info!("🔄 Connecting to Redis at {}...", redis_url);
    let mut redis = RedisPersistence::new(&redis_url).await?;

    if chunked {
        // Chunked backfill using range API
        run_chunked_backfill(symbol, address, days, chunk_size, force, &coingecko, &mut redis).await
    } else {
        // Standard backfill
        let stats = backfill_token(symbol, address, days, force, &coingecko, &mut redis).await?;

        // Print results
        tracing::info!("\n📊 Backfill Results:");
        tracing::info!("  Fetched data points: {}", stats.fetched_points);
        tracing::info!("  Converted candles: {}", stats.converted_candles);
        tracing::info!("  Skipped existing: {}", stats.skipped_existing);
        tracing::info!("  Stored new: {}", stats.stored_new);
        tracing::info!("  Validation failures: {}", stats.validation_failures);

        if stats.stored_new > 0 {
            tracing::info!("\n✅ Backfill complete!");
        } else if stats.skipped_existing > 0 {
            tracing::info!("\n✅ All data already present, nothing to backfill");
        } else {
            tracing::warn!("\n⚠️  No data was stored");
        }

        Ok(())
    }
}

async fn run_chunked_backfill(
    symbol: &str,
    address: &str,
    total_days: u32,
    chunk_size: u32,
    force: bool,
    coingecko: &CoinGeckoClient,
    redis: &mut RedisPersistence,
) -> Result<()> {
    use chrono::Utc;

    tracing::info!("\n🔄 Starting chunked backfill for {} ({} days in {}-day chunks)...", symbol, total_days, chunk_size);

    // Find CoinGecko coin_id
    let coin_id = coingecko.find_coin_id(symbol, address).await?;
    tracing::info!("  Found coin_id: {}", coin_id);

    // Calculate chunks (working backwards from now)
    let now = Utc::now().timestamp();
    let mut chunks = Vec::new();
    let mut remaining_days = total_days;
    let mut current_end = now;

    while remaining_days > 0 {
        let chunk_days = remaining_days.min(chunk_size);
        let chunk_start = current_end - (chunk_days as i64 * 24 * 60 * 60);
        chunks.push((chunk_start, current_end, chunk_days));
        current_end = chunk_start;
        remaining_days = remaining_days.saturating_sub(chunk_size);
    }

    chunks.reverse(); // Process oldest to newest

    tracing::info!("  Split into {} chunks", chunks.len());

    let mut total_stats = cryptobot::backfill::BackfillStats {
        fetched_points: 0,
        converted_candles: 0,
        skipped_existing: 0,
        stored_new: 0,
        validation_failures: 0,
    };

    for (i, (from_ts, to_ts, chunk_days)) in chunks.iter().enumerate() {
        tracing::info!("\n  📦 Chunk {}/{}: {} days (from {} to {})",
            i + 1, chunks.len(), chunk_days,
            chrono::DateTime::from_timestamp(*from_ts, 0).unwrap().format("%Y-%m-%d"),
            chrono::DateTime::from_timestamp(*to_ts, 0).unwrap().format("%Y-%m-%d")
        );

        // Fetch data for this chunk using range API
        let market_data = coingecko.get_market_chart_range(&coin_id, *from_ts, *to_ts).await?;
        let fetched_count = market_data.prices.len();
        tracing::info!("    Fetched {} price points", fetched_count);

        // Convert to candles (hourly for ranges within 365 days)
        let converter = cryptobot::backfill::CandleConverter::for_hourly();
        let candles = converter.convert_to_candles(symbol, market_data)?;
        tracing::info!("    Converted to {} hourly candles", candles.len());

        // Get existing timestamps for overlap detection
        let existing_timestamps = if !force {
            redis.load_candles(symbol, *chunk_days as u64 * 24)
                .await?
                .into_iter()
                .map(|c| c.timestamp)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        // Validate and filter candles
        let validator = cryptobot::backfill::CandleValidator::new();
        let mut candles_to_store = Vec::new();
        let mut chunk_skipped = 0;
        let mut chunk_failed = 0;

        for candle in candles {
            // Validate
            if let Err(e) = validator.validate(&candle) {
                tracing::warn!("Validation failed for candle at {}: {}", candle.timestamp, e);
                chunk_failed += 1;
                continue;
            }

            // Check for overlap
            if !force {
                let is_duplicate = existing_timestamps
                    .iter()
                    .any(|&ts| (candle.timestamp - ts).num_seconds().abs() < 60);

                if is_duplicate {
                    chunk_skipped += 1;
                    continue;
                }
            }

            candles_to_store.push(candle);
        }

        // Store candles
        let chunk_stored = if !candles_to_store.is_empty() {
            redis.save_candles(symbol, &candles_to_store).await?;
            candles_to_store.len()
        } else {
            0
        };

        tracing::info!("    ✓ Stored {} new candles (skipped {}, failed validation {})",
            chunk_stored, chunk_skipped, chunk_failed);

        // Aggregate stats
        total_stats.fetched_points += fetched_count;
        total_stats.converted_candles += candles_to_store.len() + chunk_skipped + chunk_failed;
        total_stats.skipped_existing += chunk_skipped;
        total_stats.stored_new += chunk_stored;
        total_stats.validation_failures += chunk_failed;
    }

    // Print final results
    tracing::info!("\n📊 Total Backfill Results:");
    tracing::info!("  Fetched data points: {}", total_stats.fetched_points);
    tracing::info!("  Converted candles: {}", total_stats.converted_candles);
    tracing::info!("  Skipped existing: {}", total_stats.skipped_existing);
    tracing::info!("  Stored new: {}", total_stats.stored_new);
    tracing::info!("  Validation failures: {}", total_stats.validation_failures);

    if total_stats.stored_new > 0 {
        tracing::info!("\n✅ Chunked backfill complete!");
    } else if total_stats.skipped_existing > 0 {
        tracing::info!("\n✅ All data already present, nothing to backfill");
    } else {
        tracing::warn!("\n⚠️  No data was stored");
    }

    Ok(())
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
    add_must_track_tokens(&safe_tokens, &mut final_tokens, &mut tracked_addresses).await;

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
            tracing::info!(
                "✓ Must-track {} found in trending, using fresh data",
                symbol
            );
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
                name: if *symbol == "SOL" {
                    "Solana"
                } else {
                    "Jupiter"
                }
                .to_string(),
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
            tracing::info!(
                "✓ Added must-track {} (pre-approved, skips safety checks)",
                symbol
            );
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

/// Run token rotation: mark stale and removed tokens
///
/// - Stale: not seen in trending for > 24h (stops price fetching)
/// - Removed: not seen in trending for > 7d (archived)
/// - Protects: must-track tokens (SOL, JUP) and tokens with open positions
async fn run_token_rotation(postgres: &PostgresPersistence) {
    // Extract must-track symbols from MUST_TRACK constant
    let must_track_symbols: Vec<&str> = MUST_TRACK.iter().map(|(_, symbol)| *symbol).collect();

    // Mark tokens stale if not seen in 24h
    match postgres.mark_stale_tokens(&must_track_symbols).await {
        Ok(count) => {
            if count > 0 {
                tracing::info!("  🔄 Marked {} tokens as stale (not seen in 24h)", count);
            }
        }
        Err(e) => {
            tracing::warn!("  ⚠️  Failed to mark stale tokens: {}", e);
        }
    }

    // Mark tokens removed if not seen in 7 days
    match postgres.mark_removed_tokens(&must_track_symbols).await {
        Ok(count) => {
            if count > 0 {
                tracing::warn!("  🗑️  Marked {} tokens as removed (not seen in 7d)", count);
            }
        }
        Err(e) => {
            tracing::warn!("  ⚠️  Failed to mark removed tokens: {}", e);
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
                    tracing::info!("  ✓ {} @ ${:.4}", token.symbol, snapshot.price);

                    // Save to Redis
                    if let Ok(candles) = price_manager.buffer().get_candles(&token.symbol) {
                        if let Some(latest_candle) = candles.last() {
                            if let Err(e) = redis
                                .save_candles(&token.symbol, std::slice::from_ref(latest_candle))
                                .await
                            {
                                tracing::warn!(
                                    "  ✗ Failed to save {} to Redis: {}",
                                    token.symbol,
                                    e
                                );
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
        // TODO: reenable cleanup when appropriate. we want to accumulate data for backtesting, not just keep the last 48 hours.
        //if tick_count % CLEANUP_INTERVAL == 0 {
        //    tracing::info!("🧹 Running Redis cleanup (keeping last {}h)...", KEEP_HOURS);

        //    for token in &token_list {
        //        match redis.cleanup_old(&token.symbol, KEEP_HOURS).await {
        //            Ok(removed) => {
        //                if removed > 0 {
        //                    tracing::info!(
        //                        "  ✓ Cleaned up {} old snapshots for {}",
        //                        removed,
        //                        token.symbol
        //                    );
        //                }
        //            }
        //            Err(e) => {
        //                tracing::warn!("  ✗ Failed to cleanup {}: {}", token.symbol, e);
        //            }
        //        }
        //    }
        //}
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

    // We'll create strategies per-token with their specific RSI thresholds
    // Default strategy is used for samples_needed calculation
    let default_strategy = MomentumStrategy::default().with_poll_interval(POLL_INTERVAL_MINUTES);
    let samples_needed = default_strategy.samples_needed(POLL_INTERVAL_MINUTES);
    let mut executor = Executor::new(state.position_manager.clone());

    // Create interval starting 30 seconds after next 5-minute boundary
    // This gives price_fetch_loop time to complete
    let start = next_5min_boundary_plus_30s();
    let delay = start - Instant::now();
    tracing::info!("Trading will start in {:?} (30s after price fetch)", delay);

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
            // Load per-token RSI threshold from database
            let rsi_threshold = if let Some(ref pg) = postgres {
                pg.get_rsi_threshold(&token.symbol).await.unwrap_or(45.0)
            } else {
                45.0 // Default if no database
            };

            // Create token-specific strategy with its optimal RSI threshold
            let token_strategy = {
                use cryptobot::strategy::signals::SignalConfig;
                let config = SignalConfig {
                    rsi_oversold: rsi_threshold,
                    ..Default::default()
                };
                MomentumStrategy::new(config).with_poll_interval(POLL_INTERVAL_MINUTES)
            };

            match redis
                .load_candles(&token.symbol, token_strategy.lookback_hours())
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
                            "  {} @ ${:.4} ({} candles, RSI < {:.0})",
                            token.symbol,
                            latest.close,
                            candles.len(),
                            rsi_threshold
                        );

                        // Generate signals if we have enough data
                        if candles.len() >= samples_needed {
                            process_token_signal(
                                &token_strategy,
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
        log_portfolio_summary(
            &state.position_manager,
            &prices,
            state.initial_portfolio_value,
        );
    }
}

/// Loop 3: Token Discovery Loop (every 4 hours)
/// Discovers trending tokens and updates the token list
async fn token_discovery_loop(
    tokens: Arc<RwLock<Vec<Token>>>,
    birdeye_api_key: String,
    postgres_url: Option<String>,
) {
    tracing::info!("🔍 Token Discovery Loop starting...");

    let birdeye_client = BirdeyeClient::new(birdeye_api_key);

    // Initialize CoinGecko client if API key is available (for backfilling)
    let coingecko_client = match std::env::var("COINGECKO_API_KEY") {
        Ok(api_key) => match CoinGeckoClient::new(api_key).await {
            Ok(client) => {
                tracing::info!("✓ CoinGecko backfill enabled");
                Some(client)
            }
            Err(e) => {
                tracing::warn!("Failed to initialize CoinGecko client: {}", e);
                None
            }
        },
        Err(_) => {
            tracing::info!("CoinGecko backfill disabled (COINGECKO_API_KEY not set)");
            None
        }
    };

    // Get Redis URL for backfilling
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    // Start immediately, then run every 4 hours 
    let mut ticker = interval_at(Instant::now(), Duration::from_secs(14400));
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

                // Run token rotation BEFORE saving new tokens
                // This marks stale/removed tokens, then saving reactivates trending ones
                if let Some(ref pg) = postgres {
                    run_token_rotation(pg).await;
                }

                // Check which tokens need backfilling (new OR insufficient data)
                let tokens_to_backfill = if coingecko_client.is_some() {
                    identify_tokens_needing_backfill(&final_tokens, &redis_url).await
                } else {
                    Vec::new()
                };

                if !tokens_to_backfill.is_empty() {
                    tracing::info!(
                        "  📥 {} tokens need backfill (new or insufficient data)",
                        tokens_to_backfill.len()
                    );

                    // Trigger backfill (non-blocking)
                    if let Some(ref cg_client) = coingecko_client {
                        spawn_backfill_tasks(tokens_to_backfill, cg_client.clone(), &redis_url)
                            .await;
                    }
                }

                // Save to database
                // This will reactivate tokens that were previously stale/removed if they're trending again
                if let Some(mut pg) = postgres {
                    save_tracked_tokens_to_db(&mut pg, &final_tokens).await;

                    // Calculate optimal RSI thresholds for tokens that need it
                    // (new tokens or tokens with sufficient data that haven't been tuned yet)
                    tune_rsi_for_new_tokens(&pg, &final_tokens, &redis_url).await;
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

/// Initialize CoinGecko client and run backfill for tokens with insufficient data
async fn initialize_and_run_backfill(tokens: &[TrendingToken], redis_url: &str) {
    // Try to initialize CoinGecko client
    let coingecko_client = match std::env::var("COINGECKO_API_KEY") {
        Ok(api_key) => match CoinGeckoClient::new(api_key).await {
            Ok(client) => {
                tracing::info!("✓ CoinGecko backfill enabled");
                Some(client)
            }
            Err(e) => {
                tracing::warn!("Failed to initialize CoinGecko client: {}", e);
                None
            }
        },
        Err(_) => {
            tracing::info!("COINGECKO_API_KEY not set, backfill disabled");
            None
        }
    };

    // Early return if no CoinGecko client
    let Some(cg_client) = coingecko_client else {
        return;
    };

    // Identify tokens needing backfill
    let tokens_to_backfill = identify_tokens_needing_backfill(tokens, redis_url).await;

    if tokens_to_backfill.is_empty() {
        return;
    }

    tracing::info!(
        "  📥 {} tokens need backfill (new or insufficient data)",
        tokens_to_backfill.len()
    );

    // Spawn backfill tasks (non-blocking)
    spawn_backfill_tasks(tokens_to_backfill, cg_client, redis_url).await;
}

/// Identify tokens that need backfilling (new tokens OR existing tokens with insufficient data)
///
/// Checks Redis for candle count and returns tokens with < 200 candles (roughly 1 day of 5-min data)
async fn identify_tokens_needing_backfill(
    tokens: &[TrendingToken],
    redis_url: &str,
) -> Vec<(String, String)> {
    const MIN_CANDLES_THRESHOLD: u64 = 200; // ~1 day of 5-min candles

    let mut tokens_needing_backfill = Vec::new();

    // Connect to Redis to check candle counts
    let mut redis = match RedisPersistence::new(redis_url).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to connect to Redis for backfill check: {}", e);
            return tokens_needing_backfill;
        }
    };

    for token in tokens {
        // Load existing candles for this token
        match redis
            .load_candles(&token.symbol, MIN_CANDLES_THRESHOLD)
            .await
        {
            Ok(candles) => {
                let candle_count = candles.len() as u64;

                if candle_count < MIN_CANDLES_THRESHOLD {
                    tracing::debug!(
                        "Token {} has {} candles (< {} threshold), needs backfill",
                        token.symbol,
                        candle_count,
                        MIN_CANDLES_THRESHOLD
                    );
                    tokens_needing_backfill.push((token.symbol.clone(), token.address.clone()));
                } else {
                    tracing::debug!(
                        "Token {} has {} candles, sufficient data",
                        token.symbol,
                        candle_count
                    );
                }
            }
            Err(e) => {
                tracing::debug!(
                    "Failed to load candles for {} (will backfill): {}",
                    token.symbol,
                    e
                );
                // If we can't load, assume it needs backfill
                tokens_needing_backfill.push((token.symbol.clone(), token.address.clone()));
            }
        }
    }

    tokens_needing_backfill
}

/// Spawn background tasks to backfill newly discovered tokens
///
/// Uses a shared CoinGecko client (with shared rate limiter) and limits
/// concurrent backfills to avoid overwhelming the API or Redis.
async fn spawn_backfill_tasks(
    new_tokens: Vec<(String, String)>,
    coingecko_client: CoinGeckoClient,
    redis_url: &str,
) {
    const BACKFILL_DAYS: u32 = 1; // 1 day = ~287 candles at 5-min resolution
    const MAX_CONCURRENT: usize = 3; // Limit concurrent backfills

    use std::sync::Arc;
    use tokio::sync::Semaphore;

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));

    for (symbol, address) in new_tokens {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let cg_client = coingecko_client.clone(); // Shares rate limiter!
        let redis_url_clone = redis_url.to_string();

        // Spawn a background task for backfilling
        tokio::spawn(async move {
            let _permit = permit; // Hold permit until task completes
            tracing::info!("  📥 Starting backfill for {} ({}d)", symbol, BACKFILL_DAYS);

            // Connect to Redis
            let mut redis = match RedisPersistence::new(&redis_url_clone).await {
                Ok(redis) => redis,
                Err(e) => {
                    tracing::error!("  ✗ Failed to connect to Redis for {}: {}", symbol, e);
                    return;
                }
            };

            // Run backfill with retry logic
            const MAX_RETRIES: u32 = 3;
            let mut last_error = None;

            for attempt in 1..=MAX_RETRIES {
                match backfill_token(
                    &symbol,
                    &address,
                    BACKFILL_DAYS,
                    false, // Don't force overwrite
                    &cg_client,
                    &mut redis,
                )
                .await
                {
                    Ok(stats) => {
                        tracing::info!(
                            "  ✓ Backfill complete for {}: stored {} candles (skipped {}, failed validation {})",
                            symbol,
                            stats.stored_new,
                            stats.skipped_existing,
                            stats.validation_failures
                        );
                        return; // Success!
                    }
                    Err(e) => {
                        last_error = Some(e);
                        if attempt < MAX_RETRIES {
                            let backoff_secs = 2u64.pow(attempt);
                            tracing::warn!(
                                "  ⚠️  Backfill attempt {}/{} failed for {}, retrying in {}s...",
                                attempt,
                                MAX_RETRIES,
                                symbol,
                                backoff_secs
                            );
                            tokio::time::sleep(tokio::time::Duration::from_secs(backoff_secs))
                                .await;
                        }
                    }
                }
            }

            // All retries failed
            if let Some(e) = last_error {
                tracing::error!(
                    "  ✗ Backfill failed for {} after {} attempts: {}",
                    symbol,
                    MAX_RETRIES,
                    e
                );
            }
        });
    }
}

// ============================================================================
// RSI Threshold Calculation
// ============================================================================

/// Tune RSI thresholds for newly discovered tokens with sufficient data
async fn tune_rsi_for_new_tokens(
    postgres: &PostgresPersistence,
    tokens: &[TrendingToken],
    redis_url: &str,
) {
    // Connect to Redis
    let mut redis = match RedisPersistence::new(redis_url).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to connect to Redis for RSI tuning: {}", e);
            return;
        }
    };

    for token in tokens {
        // Check if token already has a non-default threshold
        match postgres.get_rsi_threshold(&token.symbol).await {
            Ok(threshold) if (threshold - 45.0).abs() < 0.1 => {
                // Has default threshold (45.0), needs tuning
                tracing::debug!("{} has default RSI threshold, checking for tuning", token.symbol);
            }
            Ok(_) => {
                // Already has custom threshold, skip
                continue;
            }
            Err(_) => {
                // Error loading, skip
                continue;
            }
        }

        // Calculate optimal threshold
        let optimal_threshold = calculate_optimal_rsi_threshold(&token.symbol, &mut redis).await;

        // Only update if different from default
        if (optimal_threshold - 45.0).abs() > 0.1 {
            if let Err(e) = postgres
                .update_rsi_threshold(&token.symbol, optimal_threshold)
                .await
            {
                tracing::warn!(
                    "Failed to save RSI threshold for {}: {}",
                    token.symbol,
                    e
                );
            } else {
                tracing::info!(
                    "  ✓ {} - Saved optimal RSI < {:.0}",
                    token.symbol,
                    optimal_threshold
                );
            }
        }
    }
}

/// Calculate optimal RSI threshold for a newly discovered token
/// Returns the best threshold based on backtest or default 45.0 if insufficient data
async fn calculate_optimal_rsi_threshold(
    symbol: &str,
    redis: &mut RedisPersistence,
) -> f64 {
    const MIN_CANDLES_FOR_TUNING: usize = 500; // Need ~2 days of 5-min data minimum
    const RSI_THRESHOLDS: [f64; 5] = [30.0, 35.0, 40.0, 45.0, 50.0];

    // Load available candles
    let candles = match redis.load_candles(symbol, MIN_CANDLES_FOR_TUNING as u64).await {
        Ok(c) if c.len() >= MIN_CANDLES_FOR_TUNING => c,
        _ => {
            tracing::info!(
                "  ⚙️  {} - Insufficient data for RSI tuning, using default 45.0",
                symbol
            );
            return 45.0;
        }
    };

    tracing::info!(
        "  ⚙️  {} - Running RSI parameter sweep ({} candles)...",
        symbol,
        candles.len()
    );

    use cryptobot::backtest::BacktestRunner;
    use cryptobot::risk::CircuitBreakers;
    use cryptobot::strategy::signals::SignalConfig;

    let mut best_return = f64::NEG_INFINITY;
    let mut best_threshold = 45.0;

    for &rsi_threshold in &RSI_THRESHOLDS {
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

        let strategy = MomentumStrategy::new(config).with_poll_interval(5);
        let runner = BacktestRunner::new(10000.0, CircuitBreakers::default());

        if let Ok(metrics) = runner.run(&strategy, candles.clone(), symbol, 5, 0.0) {
            if metrics.total_return_pct > best_return {
                best_return = metrics.total_return_pct;
                best_threshold = rsi_threshold;
            }
        }
    }

    tracing::info!(
        "  ✓ {} - Optimal RSI < {:.0} ({:+.2}% backtest return)",
        symbol,
        best_threshold,
        best_return
    );

    best_threshold
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
        ExecutionAction::Close {
            position_id,
            exit_reason,
        } => {
            execute_close(
                *position_id,
                current_price,
                exit_reason.clone(),
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
    exit_reason: cryptobot::execution::ExitReason,
    position_manager: &Arc<Mutex<PositionManager>>,
    postgres_persistence: Option<&mut PostgresPersistence>,
) {
    let closed_position = {
        let mut pm = position_manager.lock().unwrap();
        match pm.close_position(position_id, current_price, exit_reason) {
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
