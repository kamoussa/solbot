# CoinGecko Backfill Integration - Planning Document (v3 - Final)

**Status:** ✅ Ready for Implementation (Phase 0.5 Research Complete)
**Created:** 2025-10-23
**Updated:** 2025-10-23 (after Phase 0.5 research)
**Goal:** Enable fast historical data backfilling for new tokens to eliminate 24-hour wait time for trading

**Research Findings:** See `COINGECKO_RESEARCH_FINDINGS.md` for detailed investigation results

---

## Phase 0.5 Research Summary

**All blocking questions answered:**
1. ✅ Volume data: 24h rolling (use 0.0 for backfilled candles)
2. ✅ Candle struct: Compatible (has open/high/low/close/volume)
3. ✅ Redis API: `save_candles()` exists and works
4. ✅ Timestamp strategy: Only backfill data >24hrs old (prevents overlap)
5. ✅ Lookup logic: Fixed with inverted map
6. ✅ Overlap detection: Use time windows (±60s)

**Key Discovery:** Current Redis storage only saves `close` price. When loading, it sets `open=high=low=close=price`. **Our backfilled data will actually be BETTER** (has real OHLC values).

---

## Problem Statement

**Current Issue:**
- When bot discovers a new token, it needs 24+ hours of 5-minute candle data before trading
- This blocks development velocity and testing
- Frustrating iteration cycle when testing strategy changes

**User Need:**
> "its annoying and bad for dev productivity to wait 1 full day for trading to go through for a token"

---

## Critique Summary

**Phase 1 Critique - Major Issues Fixed:**
1. ✅ Token mapping now uses `/coins/list` API with cache (scalable)
2. ✅ Uniformity check relaxed to ±60s tolerance for backfilled data
3. ✅ Forward-fill removed - skip empty buckets instead
4. ✅ Overlap detection added to prevent overwriting live data
5. ✅ Data validation added for OHLC sanity checks
6. ✅ Bucketing algorithm rewritten with proper alignment
7. ✅ Edge case tests added
8. ✅ Rate limiter properly designed

**Phase 2 Critique - Critical Issues Found & Resolved:**
1. ⚠️ Timestamp alignment would cause overlaps → **Fixed: Only backfill >24hrs old data**
2. ⚠️ Volume data misunderstood → **Fixed: Use 0.0 (24h rolling, not per-candle)**
3. ⚠️ Three-tier lookup broken → **Fixed: Added inverted map**
4. ⚠️ Overlap detection wouldn't work → **Fixed: Use time windows (±60s)**
5. ✅ Candle struct compatibility confirmed (no changes needed)
6. ✅ Redis API confirmed (save_candles exists)

---

## Requirements

### Functional Requirements

1. **Backfill historical candle data** for any token from CoinGecko API
2. **Convert price points to OHLC candles** compatible with existing strategy code
3. **Store in Redis** using existing snapshot schema (`snapshots:{symbol}` sorted sets)
4. **Manual CLI trigger** for backfilling specific tokens
5. **Auto-backfill** new tokens discovered in discovery loop
6. **Resilient to irregular intervals** in source data (1.8-10.6 min range)
7. **Token ID resolution** using `/coins/list` API (5,425 Solana tokens)
8. **Overlap prevention** - only backfill data >24hrs older than latest live data ⭐ NEW

### Non-Functional Requirements

1. **Stay within rate limits:** 30 RPM, 10k calls/month
2. **Relaxed uniformity tolerance:** ±60s for backfilled data (vs ±30s for live)
3. **Test coverage:** Comprehensive unit and integration tests (TDD)
4. **Error handling:** Graceful degradation if API fails
5. **Backward compatible:** Don't break existing DexScreener price fetching
6. **Data quality:** Validate OHLC before storing

---

## Technical Analysis

### API Endpoints

Based on testing:

**`/market_chart` endpoint (RECOMMENDED)**
- Returns ~287 price points per day (avg 5.02 min intervals)
- Includes volume data in separate array
- 85% of data within 3-7 minute range
- Available on free tier

**`/coins/list` endpoint (NEW - for token mapping)**
- Returns 19,308 total coins
- 5,425 have Solana platform addresses
- Format: `{id, symbol, name, platforms: {solana: "mint_address"}}`
- Cache this list on startup (1 API call)

### Data Conversion Strategy (REVISED)

**Challenge:** Convert irregular price points to uniform 5-min OHLC candles

**Revised Approach:**
1. Fetch market_chart data (days=7 for 1 week history)
2. Response: `{"prices": [[timestamp_ms, price], ...], "total_volumes": [[timestamp_ms, volume], ...]}`
3. **Sort timestamps** (handle out-of-order data)
4. **Bucket into 5-minute windows** (aligned to :00, :05, :10 UTC)
5. For each bucket:
   - **Open:** First price chronologically
   - **High:** Max price
   - **Low:** Min price
   - **Close:** Last price chronologically
   - **Volume:** 0.0 (CoinGecko returns 24h rolling volume, not per-candle)
   - **Timestamp:** Bucket boundary time (e.g., 21:05:00)
6. Handle edge cases:
   - **Empty bucket:** **SKIP** (don't forward-fill!) ⚠️ Changed from original plan
   - **Single point:** Use same value for O/H/L/C
   - **Invalid prices:** Skip and log warning
7. **Validate each candle** before storing
8. **Check for overlaps** with existing data

### Token ID Mapping (REVISED)

**Challenge:** CoinGecko uses coin IDs, we use mint addresses

**Solution - Three-tier lookup:**

```rust
pub struct CoinGeckoClient {
    client: Client,
    api_key: String,
    // Cache loaded from /coins/list on startup
    coin_cache: Arc<RwLock<CoinCache>>,
}

struct CoinCache {
    // mint_address -> coin_id
    by_address: HashMap<String, String>,
    // symbol -> Vec<coin_id> (multiple tokens can have same symbol)
    by_symbol: HashMap<String, Vec<String>>,
}

impl CoinGeckoClient {
    pub async fn new(api_key: String) -> Result<Self> {
        let mut client = Self { ... };
        client.refresh_coin_cache().await?;  // Load on startup
        Ok(client)
    }

    async fn refresh_coin_cache(&self) -> Result<()> {
        // GET /coins/list?include_platform=true
        // Parse and populate both hashmaps
        // Filter for Solana tokens only (platforms.solana exists or native SOL)
    }

    pub async fn find_coin_id(&self, symbol: &str, mint_address: &str) -> Result<String> {
        let cache = self.coin_cache.read().await;

        // 1. Try exact mint address match (most reliable)
        if let Some(coin_id) = cache.by_address.get(mint_address) {
            return Ok(coin_id.clone());
        }

        // 2. For native SOL (wrapped address), use hardcoded
        if symbol == "SOL" && mint_address == "So11111111111111111111111111111111111111112" {
            return Ok("solana".to_string());
        }

        // 3. Try symbol match (may have multiple, pick first with Solana platform)
        if let Some(coin_ids) = cache.by_symbol.get(&symbol.to_uppercase()) {
            // Return first match with solana platform
            for coin_id in coin_ids {
                if cache.by_address.values().any(|id| id == coin_id) {
                    return Ok(coin_id.clone());
                }
            }
            // Fall back to first match
            return Ok(coin_ids[0].clone());
        }

        // 4. Not found
        Err(format!("Token {} ({}) not found in CoinGecko", symbol, mint_address).into())
    }
}
```

**API call cost:**
- `/coins/list`: 1 call on startup
- Cache refresh: 1 call per 24 hours (optional)
- **Total: ~2 calls/day** ✅ Well within limits

---

## Architecture Design

### Module Structure

```
src/api/coingecko.rs          # API client with caching
src/backfill/
  ├── mod.rs                   # Public interface
  ├── converter.rs             # Price points → OHLC conversion
  └── validator.rs             # OHLC data validation (NEW)
```

### Key Types

```rust
// src/api/coingecko.rs
pub struct CoinGeckoClient {
    client: Client,
    api_key: String,
    coin_cache: Arc<RwLock<CoinCache>>,
    rate_limiter: Arc<RateLimiter>,  // Shared rate limiter
}

#[derive(Debug, Deserialize)]
pub struct MarketChartData {
    prices: Vec<[f64; 2]>,           // [timestamp_ms, price]
    total_volumes: Vec<[f64; 2]>,    // [timestamp_ms, volume_24h]
}

#[derive(Debug, Deserialize)]
struct CoinListEntry {
    id: String,
    symbol: String,
    name: String,
    platforms: HashMap<String, String>,  // chain -> address
}

impl CoinGeckoClient {
    pub async fn new(api_key: String) -> Result<Self>;
    async fn refresh_coin_cache(&self) -> Result<()>;
    pub async fn find_coin_id(&self, symbol: &str, mint_address: &str) -> Result<String>;
    pub async fn get_market_chart(&self, coin_id: &str, days: u32) -> Result<MarketChartData>;
}

// src/backfill/converter.rs
pub struct CandleConverter {
    interval_secs: i64,  // 300 for 5-min
}

impl CandleConverter {
    pub fn convert_to_candles(&self, data: MarketChartData) -> Result<Vec<Candle>>;
    fn sort_and_dedupe(&self, mut prices: Vec<[f64; 2]>) -> Vec<[f64; 2]>;
    fn bucket_into_windows(&self, prices: Vec<[f64; 2]>) -> BTreeMap<i64, Vec<f64>>;
    fn synthesize_candle(&self, bucket_timestamp: i64, prices: Vec<f64>, volume: f64) -> Candle;
}

// src/backfill/validator.rs (NEW)
pub struct CandleValidator;

impl CandleValidator {
    pub fn validate(&self, candle: &Candle) -> Result<()>;
    fn validate_prices(&self, candle: &Candle) -> Result<()>;
    fn validate_timestamp(&self, candle: &Candle) -> Result<()>;
    fn validate_ohlc_relationship(&self, candle: &Candle) -> Result<()>;
}

// src/backfill/mod.rs
pub async fn backfill_token(
    symbol: &str,
    mint_address: &str,
    days: u32,
    force_overwrite: bool,  // NEW: Allow overwriting existing data
    coingecko: &CoinGeckoClient,
    persistence: &RedisPersistence,
) -> Result<BackfillStats>;

pub struct BackfillStats {
    pub fetched_points: usize,
    pub converted_candles: usize,
    pub skipped_existing: usize,
    pub stored_new: usize,
    pub validation_failures: usize,
}
```

### Integration Points

**1. CLI Command (new)**
```rust
cargo run -- backfill SOL So1111... 7d
cargo run -- backfill JUP JUPyiw... 30d
cargo run -- backfill SOL So1111... 7d --force  // Overwrite existing
cargo run -- backfill-all 7d  // Backfill all tracked tokens
```

**2. Discovery Loop (modify existing)**
```rust
// src/main.rs - in discovery loop
for new_token in newly_discovered_tokens {
    // Auto-backfill when adding new token
    match backfill_token(&new_token.symbol, &new_token.address, 7, false, &coingecko, &redis).await {
        Ok(stats) => {
            tracing::info!(
                "✓ Backfilled {} with {} candles (skipped {} existing)",
                new_token.symbol, stats.stored_new, stats.skipped_existing
            );
        }
        Err(e) => {
            tracing::warn!("Failed to backfill {}: {}. Will use live data only.", new_token.symbol, e);
            // Continue anyway - not a fatal error
        }
    }
}
```

**3. Uniformity Check (modify existing)**
```rust
// src/main.rs - validate_candle_uniformity()
const EXPECTED_INTERVAL: i64 = 300;
const LIVE_DATA_TOLERANCE: i64 = 30;      // ±30s for live data
const BACKFILL_DATA_TOLERANCE: i64 = 60;  // ±60s for backfilled data (NEW)

// For now, use wider tolerance for all data (simplifies Phase 1)
let tolerance = BACKFILL_DATA_TOLERANCE;  // 60 seconds

if gap > EXPECTED_INTERVAL + tolerance {
    return Err(...);
}
```

---

## Test Strategy (TDD Approach)

### Unit Tests

**1. CoinGeckoClient Tests**
```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_load_coin_cache() {
        let client = CoinGeckoClient::new("test_key".to_string()).await.unwrap();
        let cache = client.coin_cache.read().await;
        assert!(cache.by_address.len() > 5000);  // Should have ~5425 Solana tokens
    }

    #[tokio::test]
    async fn test_find_coin_id_by_address() {
        let client = CoinGeckoClient::new("test_key".to_string()).await.unwrap();
        let coin_id = client.find_coin_id("JUP", "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN").await.unwrap();
        assert_eq!(coin_id, "jupiter-exchange-solana");
    }

    #[tokio::test]
    async fn test_find_coin_id_native_sol() {
        let client = CoinGeckoClient::new("test_key".to_string()).await.unwrap();
        let coin_id = client.find_coin_id("SOL", "So11111111111111111111111111111111111111112").await.unwrap();
        assert_eq!(coin_id, "solana");
    }

    #[tokio::test]
    async fn test_find_coin_id_not_found() {
        let client = CoinGeckoClient::new("test_key".to_string()).await.unwrap();
        let result = client.find_coin_id("FAKECOIN", "FakeAddress123").await;
        assert!(result.is_err());
    }
}
```

**2. CandleConverter Tests**
```rust
#[test]
fn test_convert_uniform_data() {
    // Given: Perfect 5-min intervals
    let data = MarketChartData {
        prices: vec![
            [1000000, 100.0],
            [1000300000, 101.0],
            [1000600000, 102.0],
        ],
        total_volumes: vec![...],
    };

    let converter = CandleConverter::new(300);
    let candles = converter.convert_to_candles(data).unwrap();

    assert_eq!(candles.len(), 3);
    assert_eq!(candles[0].close, 100.0);
}

#[test]
fn test_convert_irregular_data() {
    // Given: Irregular intervals (2-7 min)
    let data = MarketChartData {
        prices: vec![
            [1000000, 100.0],        // t=0
            [1000180000, 100.5],     // t=3min
            [1000360000, 101.0],     // t=6min
            [1000540000, 101.5],     // t=9min
        ],
        total_volumes: vec![...],
    };

    let converter = CandleConverter::new(300);
    let candles = converter.convert_to_candles(data).unwrap();

    // Should produce 2 candles (0-5min, 5-10min)
    assert_eq!(candles.len(), 2);
    // First bucket contains 100.0 and 100.5
    assert_eq!(candles[0].open, 100.0);
    assert_eq!(candles[0].high, 100.5);
    assert_eq!(candles[0].close, 100.5);
}

#[test]
fn test_convert_sparse_data_skips_gaps() {
    // Given: Data with gaps
    let data = MarketChartData {
        prices: vec![
            [1000000, 100.0],        // t=0
            [1000300000, 101.0],     // t=5min
            // GAP - no data at t=10min
            [1000900000, 102.0],     // t=15min
        ],
        total_volumes: vec![...],
    };

    let converter = CandleConverter::new(300);
    let candles = converter.convert_to_candles(data).unwrap();

    // Should produce 2 candles, SKIP middle bucket
    assert_eq!(candles.len(), 2);
    assert_eq!(candles[0].close, 101.0);
    assert_eq!(candles[1].close, 102.0);
}

#[test]
fn test_sort_out_of_order_timestamps() {
    let data = MarketChartData {
        prices: vec![
            [1000300000, 101.0],  // Out of order
            [1000000, 100.0],
            [1000600000, 102.0],
        ],
        total_volumes: vec![...],
    };

    let converter = CandleConverter::new(300);
    let candles = converter.convert_to_candles(data).unwrap();

    // Should sort before processing
    assert_eq!(candles[0].close, 100.0);
    assert_eq!(candles[1].close, 101.0);
}

#[test]
fn test_dedupe_duplicate_timestamps() {
    let data = MarketChartData {
        prices: vec![
            [1000000, 100.0],
            [1000000, 100.5],  // Duplicate timestamp
            [1000300000, 101.0],
        ],
        total_volumes: vec![...],
    };

    let converter = CandleConverter::new(300);
    let candles = converter.convert_to_candles(data).unwrap();

    // Should use last value for duplicates
    assert_eq!(candles[0].open, 100.5);
}

#[test]
fn test_empty_response() {
    let data = MarketChartData {
        prices: vec![],
        total_volumes: vec![],
    };

    let converter = CandleConverter::new(300);
    let candles = converter.convert_to_candles(data).unwrap();

    assert_eq!(candles.len(), 0);
}
```

**3. CandleValidator Tests**
```rust
#[test]
fn test_validate_valid_candle() {
    let candle = Candle {
        open: 100.0,
        high: 102.0,
        low: 99.0,
        close: 101.0,
        volume_24h: 1000000.0,
        timestamp: Utc::now() - Duration::hours(1),
        source: DataSource::CoinGecko,
    };

    let validator = CandleValidator;
    assert!(validator.validate(&candle).is_ok());
}

#[test]
fn test_validate_negative_price() {
    let candle = Candle {
        open: -100.0,  // Invalid
        high: 102.0,
        low: 99.0,
        close: 101.0,
        ...
    };

    let validator = CandleValidator;
    assert!(validator.validate(&candle).is_err());
}

#[test]
fn test_validate_high_less_than_low() {
    let candle = Candle {
        open: 100.0,
        high: 98.0,   // Less than low!
        low: 99.0,
        close: 101.0,
        ...
    };

    let validator = CandleValidator;
    assert!(validator.validate(&candle).is_err());
}

#[test]
fn test_validate_future_timestamp() {
    let candle = Candle {
        timestamp: Utc::now() + Duration::hours(10),  // In future
        ...
    };

    let validator = CandleValidator;
    assert!(validator.validate(&candle).is_err());
}
```

**4. Backfill Integration Tests**
```rust
#[tokio::test]
#[ignore]  // Requires API key
async fn test_backfill_token_live() {
    let coingecko = CoinGeckoClient::new(env::var("COINGECKO_API_KEY").unwrap()).await.unwrap();
    let redis = RedisPersistence::new(env::var("REDIS_URL").unwrap()).await.unwrap();

    let stats = backfill_token(
        "SOL",
        "So11111111111111111111111111111111111111112",
        1,
        false,
        &coingecko,
        &redis
    ).await.unwrap();

    assert!(stats.stored_new > 200);  // ~288 expected for 1 day
    assert_eq!(stats.validation_failures, 0);

    // Verify uniformity (with tolerance)
    let candles = redis.get_candles("SOL", 300).await.unwrap();
    for i in 1..candles.len() {
        let interval = (candles[i].timestamp - candles[i-1].timestamp).num_seconds();
        assert!(interval >= 240 && interval <= 360, "Candles should be ~300s apart (±60s)");
    }
}

#[tokio::test]
#[ignore]
async fn test_backfill_prevents_overlap() {
    let coingecko = CoinGeckoClient::new(env::var("COINGECKO_API_KEY").unwrap()).await.unwrap();
    let redis = RedisPersistence::new(env::var("REDIS_URL").unwrap()).await.unwrap();

    // First backfill
    let stats1 = backfill_token("SOL", "So1111...", 1, false, &coingecko, &redis).await.unwrap();

    // Second backfill (should skip existing)
    let stats2 = backfill_token("SOL", "So1111...", 1, false, &coingecko, &redis).await.unwrap();

    assert_eq!(stats2.stored_new, 0);
    assert!(stats2.skipped_existing > 0);
}

#[tokio::test]
#[ignore]
async fn test_backfill_force_overwrite() {
    let coingecko = CoinGeckoClient::new(env::var("COINGECKO_API_KEY").unwrap()).await.unwrap();
    let redis = RedisPersistence::new(env::var("REDIS_URL").unwrap()).await.unwrap();

    // First backfill
    let stats1 = backfill_token("SOL", "So1111...", 1, false, &coingecko, &redis).await.unwrap();

    // Force overwrite
    let stats2 = backfill_token("SOL", "So1111...", 1, true, &coingecko, &redis).await.unwrap();

    assert!(stats2.stored_new > 0);  // Should overwrite
}
```

---

## Implementation Steps (TDD Flow)

### Phase 0: Critical Fixes & Research (1 hour) - NEW

1. **Test `/coins/list` endpoint structure** ✅ DONE
2. **Design coin cache data structures**
3. **Decide uniformity tolerance strategy** → Use ±60s for all backfilled data
4. **Update CandleConverter spec** → Skip gaps instead of forward-fill
5. **Design overlap detection logic**

### Phase 1: CoinGecko Client with Caching (1.5-2 hours)

1. **Write tests first:**
   - `test_load_coin_cache()`
   - `test_find_coin_id_by_address()`
   - `test_find_coin_id_native_sol()`
   - `test_find_coin_id_not_found()`
   - `test_get_market_chart_live()` (ignored)

2. **Implement to pass tests:**
   - Create `src/api/coingecko.rs`
   - Implement `CoinGeckoClient` with cache
   - Add `refresh_coin_cache()` method
   - Add `find_coin_id()` with 3-tier lookup
   - Add `get_market_chart()` method

3. **Refactor:**
   - Add rate limiter
   - Add retry logic
   - Add logging

### Phase 2: Candle Conversion & Validation (1.5-2 hours)

1. **Write tests first:**
   - All CandleConverter tests (uniform, irregular, sparse, sorting, deduping, empty)
   - All CandleValidator tests (valid, negative, high<low, future timestamp)

2. **Implement to pass tests:**
   - Create `src/backfill/converter.rs`
   - Implement sorting and deduping
   - Implement bucketing logic (BTreeMap for sorted windows)
   - Implement OHLC synthesis
   - Skip empty buckets (don't forward-fill)
   - Create `src/backfill/validator.rs`
   - Implement all validation checks

3. **Refactor:**
   - Optimize bucketing
   - Add detailed error messages

### Phase 3: Backfill Integration with Overlap Detection (1 hour)

1. **Write tests first:**
   - `test_backfill_token_live()` (integration)
   - `test_backfill_prevents_overlap()`
   - `test_backfill_force_overwrite()`

2. **Implement to pass tests:**
   - Create `src/backfill/mod.rs`
   - Implement `backfill_token()` with overlap detection
   - Check existing timestamps before storing
   - Return detailed stats

3. **Refactor:**
   - Add progress logging
   - Add performance metrics

### Phase 4: CLI Command (30-45 min)

1. **Write tests first:**
   - CLI argument parsing tests
   - Integration test for CLI flow

2. **Implement to pass tests:**
   - Add clap arguments
   - Wire up to backfill function
   - Add `--force` flag

3. **Refactor:**
   - Add help text
   - Add validation
   - Add progress bar

### Phase 5: Discovery Loop Integration & Uniformity Fix (30-45 min)

1. **Write tests first:**
   - Mock test for auto-backfill on new token

2. **Implement to pass tests:**
   - Modify discovery loop
   - Add auto-backfill call
   - Modify uniformity check tolerance

3. **Refactor:**
   - Ensure non-blocking
   - Add metrics

---

## Error Handling

### Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum BackfillError {
    #[error("API error: {0}")]
    ApiError(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Token not found: {0}")]
    TokenNotFound(String),

    #[error("Invalid data: {0}")]
    ValidationError(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Empty response from API")]
    EmptyResponse,
}
```

### Error Strategies

1. **API Errors:**
   - 429 → Exponential backoff, retry up to 3 times
   - 404 → Log warning, return `TokenNotFound`
   - 500 → Retry with backoff
   - Network timeout → Retry

2. **Data Errors:**
   - Empty response → Return empty vec, log warning
   - Invalid JSON → Parse error, retry once
   - Out-of-order timestamps → Sort automatically
   - Invalid OHLC → Skip candle, log warning, continue

3. **Redis Errors:**
   - Connection lost → Reconnect with backoff
   - Write failure → Retry operation
   - Read failure → Return error (don't mask)

---

## Configuration

### Environment Variables

```bash
# .env (already exists)
COINGECKO_API_KEY=CG-ksJQUxkhQREHmTnHwyUJLhM2
REDIS_URL=redis://127.0.0.1:6379
```

### Constants

```rust
// src/api/coingecko.rs
const COINGECKO_API_BASE: &str = "https://api.coingecko.com/api/v3";
const RATE_LIMIT_RPM: u32 = 30;
const RATE_LIMIT_MONTHLY: u32 = 10000;
const DEFAULT_BACKFILL_DAYS: u32 = 7;
const COIN_CACHE_REFRESH_HOURS: u64 = 24;

// src/backfill/mod.rs
const BUCKET_INTERVAL_SECS: i64 = 300;  // 5 minutes
const UNIFORMITY_TOLERANCE_SECS: i64 = 60;  // ±60s for backfilled data
```

---

## Success Criteria

### Must Have (Phase 1)
- ✅ Coin cache loads successfully with 5000+ Solana tokens
- ✅ Token ID resolution works for SOL, JUP, BONK, RAY
- ✅ Backfill SOL/JUP from CLI successfully
- ✅ Data stored in Redis with ~300s intervals (±60s tolerance)
- ✅ Trading loop accepts backfilled data (no gap errors)
- ✅ All tests pass
- ✅ Rate limits respected (no 429 errors)
- ✅ Overlap detection prevents overwriting live data
- ✅ Invalid candles rejected by validator

### Nice to Have (Future)
- Auto-backfill on bot startup for tokens with <24hrs data
- PostgreSQL cache for coin_id mappings
- Progress bars for long operations
- Backfill from multiple sources (fallback to OHLC if market_chart fails)

---

## Risks and Mitigations

| Risk | Impact | Mitigation | Status |
|------|--------|------------|--------|
| Uniformity check fails | CRITICAL | ✅ Relax tolerance to ±60s | FIXED |
| Token mapping doesn't scale | High | ✅ Use /coins/list cache | FIXED |
| Forward-fill introduces bad data | Medium | ✅ Skip empty buckets instead | FIXED |
| Overlap overwrites good data | Medium | ✅ Add overlap detection | FIXED |
| No data validation | Medium | ✅ Add CandleValidator | FIXED |
| Bucketing alignment issues | High | ✅ Align to UTC boundaries | FIXED |
| Rate limit exceeded | Low | Rate limiter with tracking | Designed |
| Coin cache stale | Low | Refresh every 24h (optional) | Designed |

---

## Timeline Estimate (Updated)

| Phase | Estimated Time |
|-------|----------------|
| Phase 0: Critical Fixes & Research | ✅ 1 hr (mostly done) |
| Phase 1: CoinGecko Client | 1.5-2 hrs |
| Phase 2: Conversion & Validation | 1.5-2 hrs |
| Phase 3: Backfill Integration | 1 hr |
| Phase 4: CLI Command | 30-45 min |
| Phase 5: Discovery Integration | 30-45 min |
| **Total** | **6-8 hours** |

---

## Next Steps

1. ✅ **Critique complete**
2. ✅ **Plan updated with fixes**
3. **Begin TDD implementation** (Phase 1: CoinGecko Client)
4. **Test each phase** before moving to next
5. **Final review** after implementation complete

---

**Status:** Ready for Implementation ✅
