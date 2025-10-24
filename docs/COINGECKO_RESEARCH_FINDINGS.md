# CoinGecko Backfill - Research Findings (Phase 0.5)

**Date:** 2025-10-23
**Purpose:** Answer blocking questions before implementation

---

## 1. Volume Data Meaning ✅ RESOLVED

**Question:** What does `total_volumes` in market_chart represent?

**Test Results:**
```
First 10 volume points (SOL, 24h period):
- $7,843,417,507
- $7,737,200,140
- $7,586,182,842
- $7,549,350,521
- $7,450,418,896
  ...

Average: $7,505,943,096
Min: $7,342,551,853
Max: $7,843,417,507
Variance: $500,865,655 (6.7% of average)
```

**Conclusion:** ✅ **24h ROLLING VOLUME** (low variance confirms this)

**Impact on Plan:**
- ❌ Cannot use for per-candle volume calculation
- ✅ Options:
  1. **Use 0.0 for volume** in backfilled candles (simplest)
  2. Calculate delta between consecutive volume points (unreliable)
  3. Use average rolling volume / 288 as approximation

**Recommendation:** Use 0.0 for volume. Trading strategy doesn't currently use volume anyway.

---

## 2. Candle Struct Fields ✅ RESOLVED

**Current Definition:**
```rust
// src/models/mod.rs
pub struct Candle {
    pub token: String,
    pub timestamp: DateTime<Utc>,
    pub open: f64,      // ✅ EXISTS
    pub high: f64,      // ✅ EXISTS
    pub low: f64,       // ✅ EXISTS
    pub close: f64,     // ✅ EXISTS
    pub volume: f64,    // ✅ EXISTS (not volume_24h)
}
```

**DataSource enum:**
```rust
pub enum DataSource {
    DexScreener,
    Jupiter,
    Fallback,
}
```

**Findings:**
- ✅ All required OHLC fields exist
- ✅ Volume field exists (just `volume`, not `volume_24h`)
- ❌ No `source` field on Candle (it's on PriceData)
- ✅ Has `token` field for symbol

**Impact on Plan:**
- ✅ **No changes needed to Candle struct**
- ✅ Can directly create Candles with OHLC data
- ❌ Cannot track data source on candle level (not critical)

---

## 3. Redis Storage API ✅ RESOLVED

**Current API:**
```rust
// src/persistence/mod.rs
impl RedisPersistence {
    pub async fn save_candles(&mut self, token: &str, candles: &[Candle]) -> Result<()>
    pub async fn load_candles(&mut self, token: &str, hours_back: u64) -> Result<Vec<Candle>>
    pub async fn count_snapshots(&mut self, token: &str) -> Result<usize>
    pub async fn cleanup_old(&mut self, token: &str, keep_hours: u64) -> Result<usize>
}
```

**Storage Format:**
```rust
struct StoredSnapshot {
    price: f64,      // Only stores close price!
    volume: f64,
    timestamp: DateTime<Utc>,
}
```

**CRITICAL DISCOVERY:**
- ✅ `save_candles()` method EXISTS and takes `&[Candle]`
- ⚠️ **But only stores `close` price and `volume`**
- ⚠️ When loading, reconstructs: `open = high = low = close = price`
- ⚠️ **Current system is already "faking" OHLC data!**

**Impact on Plan:**
- ✅ Can use `save_candles()` directly (already exists)
- ✅ Storage method already accepts OHLC, just doesn't persist it all
- ⚠️ **Our backfilled OHLC will be BETTER than live data** (has real high/low)
- 💡 **Future enhancement:** Store full OHLC in Redis (requires schema change)

---

## 4. Timestamp Alignment Strategy ⚠️ CRITICAL DECISION NEEDED

**The Problem:**
```
Scenario: Backfill SOL on 2025-10-23 at 21:45

Live data in Redis:
- 21:37:42 (actual DexScreener fetch)
- 21:42:43 (actual DexScreener fetch)
- 21:47:44 (actual DexScreener fetch)

Backfilled data (if aligned to buckets):
- 21:00:00 (bucket boundary)
- 21:05:00 (bucket boundary)
- ...
- 21:35:00 (bucket boundary)
- 21:40:00 (bucket boundary) ← OVERLAP with 21:37:42 and 21:42:43!

Result: DUPLICATE candles for same time period!
```

**Options:**

### **Option A: Align All Data** (COMPLEX)
- Modify DexScreener storage to align to buckets
- Major refactor of existing code
- Breaks backward compatibility
- **Effort:** HIGH (6-8 hours extra)

### **Option B: Don't Align Backfilled Data** (SIMPLE)
- Keep irregular timestamps from CoinGecko
- Rely on ±60s tolerance in uniformity check
- Accept "rough" backfilled data
- **Effort:** NONE (already in plan)

### **Option C: Only Backfill Old Data** (SAFEST) ⭐ RECOMMENDED
- Check latest live timestamp
- Only backfill data >24hrs old
- Never overlap with live data
- Clear separation: backfill = historical, live = recent
- **Effort:** LOW (add timestamp check)

**Recommendation:** **Option C**
```rust
pub async fn backfill_token(...) -> Result<BackfillStats> {
    // Get latest live timestamp
    let latest_live = persistence.get_latest_timestamp(symbol).await?;

    if let Some(latest) = latest_live {
        // Only backfill data older than 24 hours before latest live
        let backfill_cutoff = latest - Duration::hours(24);

        // Filter out any backfilled candles newer than cutoff
        candles.retain(|c| c.timestamp < backfill_cutoff);
    }

    // Store remaining candles
}
```

**Impact:**
- ✅ **Eliminates all overlap issues**
- ✅ Simple to implement
- ✅ Clear semantic separation
- ❌ Can't backfill very recent data (but that's OK - live data covers it)

---

## 5. Three-Tier Lookup Fix ✅ RESOLVED

**Bug in Original Plan:**
```rust
// WRONG: This checks if coin_id from by_symbol exists in by_address VALUES
for coin_id in coin_ids {
    if cache.by_address.values().any(|id| id == coin_id) {  // ← BROKEN!
        return Ok(coin_id.clone());
    }
}
```

**Fix:**
```rust
struct CoinCache {
    by_address: HashMap<String, String>,       // mint -> coin_id
    by_symbol: HashMap<String, Vec<String>>,   // symbol -> [coin_ids]
    solana_coin_ids: HashSet<String>,          // NEW: coin_ids with solana platform
}

// Then:
for coin_id in coin_ids {
    if cache.solana_coin_ids.contains(coin_id) {
        return Ok(coin_id.clone());
    }
}
```

**Alternative (simpler):**
```rust
// Just check if coin_id exists as a VALUE in by_address
// (meaning it has a solana mint address)
for coin_id in coin_ids {
    if cache.by_address.values().any(|v| v == coin_id) {
        return Ok(coin_id.clone());
    }
}

// Or invert the map:
by_coin_id_to_address: HashMap<String, String>,  // coin_id -> mint

for coin_id in coin_ids {
    if cache.by_coin_id_to_address.contains_key(coin_id) {
        return Ok(coin_id.clone());
    }
}
```

---

## 6. Overlap Detection Fix ✅ RESOLVED

**Problem:** Exact timestamp matching won't work with alignment differences

**Fix:** Use time windows
```rust
pub async fn get_existing_timestamps(&mut self, token: &str) -> Result<Vec<DateTime<Utc>>> {
    let key = format!("snapshots:{}", token);
    let scores: Vec<f64> = self.conn.zrange(&key, 0, -1).await?;

    Ok(scores.iter()
        .map(|&score| DateTime::from_timestamp(score as i64, 0).unwrap())
        .collect())
}

// In backfill logic:
for candle in candles {
    let is_duplicate = existing_timestamps.iter().any(|&ts| {
        (candle.timestamp - ts).num_seconds().abs() < 60  // Within 60s
    });

    if !force_overwrite && is_duplicate {
        stats.skipped_existing += 1;
        continue;
    }

    // Store candle
}
```

---

## 7. CLI Design ✅ CLARIFIED

**Check:** Does bot have CLI parsing?

**Finding:** Need to verify in main.rs, but assume simple approach:
```rust
// Simple enum for command
enum Command {
    Run,              // Normal bot mode
    Backfill {
        symbol: String,
        days: u32,
        force: bool,
    },
}

// In main:
match command {
    Command::Run => {
        // Start all loops
    }
    Command::Backfill { symbol, days, force } => {
        // Run backfill and exit
    }
}
```

---

## Summary of Fixes Needed

### Critical (Blocking)
1. ✅ **Volume:** Use 0.0 (strategy doesn't need it)
2. ✅ **Candle struct:** No changes needed
3. ✅ **Redis API:** Use existing `save_candles()`
4. ⭐ **Timestamp strategy:** Only backfill >24hrs old data
5. ✅ **Lookup logic:** Add solana_coin_ids set or invert map
6. ✅ **Overlap detection:** Use time windows (±60s)

### Non-Critical
7. 💡 **Rate limiter:** Use `governor` crate
8. 💡 **CLI:** Simple enum-based command parsing
9. 💡 **Progress:** Add progress logging

---

## Updated Plan Changes

### 1. Volume Handling
```rust
// In CandleConverter
fn synthesize_candle(&self, bucket_timestamp: i64, prices: Vec<f64>) -> Candle {
    Candle {
        token: "".to_string(),  // Set by caller
        timestamp: DateTime::from_timestamp(bucket_timestamp, 0).unwrap(),
        open: prices[0],
        high: *prices.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap(),
        low: *prices.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap(),
        close: *prices.last().unwrap(),
        volume: 0.0,  // ← Changed from plan: Use 0.0 instead of approximation
    }
}
```

### 2. Timestamp Strategy (NEW)
```rust
pub async fn backfill_token(
    symbol: &str,
    mint_address: &str,
    days: u32,
    force_overwrite: bool,
    coingecko: &CoinGeckoClient,
    persistence: &mut RedisPersistence,
) -> Result<BackfillStats> {
    // NEW: Get latest live timestamp
    let existing = persistence.load_candles(symbol, 24 * 7).await?;  // Last week
    let latest_live = existing.last().map(|c| c.timestamp);

    // Fetch from CoinGecko
    let coin_id = coingecko.find_coin_id(symbol, mint_address).await?;
    let market_data = coingecko.get_market_chart(&coin_id, days).await?;

    // Convert to candles
    let mut candles = converter.convert_to_candles(market_data)?;

    // NEW: Only keep candles >24hrs before latest live
    if let Some(latest) = latest_live {
        let cutoff = latest - chrono::Duration::hours(24);
        candles.retain(|c| c.timestamp < cutoff);
        tracing::info!(
            "Filtered to {} candles older than 24h before live data",
            candles.len()
        );
    }

    // Validate and store
    // ...
}
```

### 3. Lookup Logic
```rust
struct CoinCache {
    by_address: HashMap<String, String>,           // mint -> coin_id
    by_coin_id: HashMap<String, String>,           // coin_id -> mint (NEW: inverted)
    by_symbol: HashMap<String, Vec<String>>,       // symbol -> [coin_ids]
}

pub async fn find_coin_id(&self, symbol: &str, mint_address: &str) -> Result<String> {
    let cache = self.coin_cache.read().await;

    // 1. Try exact mint address match
    if let Some(coin_id) = cache.by_address.get(mint_address) {
        return Ok(coin_id.clone());
    }

    // 2. Native SOL special case
    if symbol == "SOL" && mint_address == "So11111111111111111111111111111111111111112" {
        return Ok("solana".to_string());
    }

    // 3. Try symbol match, prefer ones with Solana platform
    if let Some(coin_ids) = cache.by_symbol.get(&symbol.to_uppercase()) {
        for coin_id in coin_ids {
            if cache.by_coin_id.contains_key(coin_id) {  // Has Solana platform
                return Ok(coin_id.clone());
            }
        }
        // Fallback to first match
        return Ok(coin_ids[0].clone());
    }

    Err(format!("Token {} ({}) not found", symbol, mint_address).into())
}
```

### 4. Add Helper Method to RedisPersistence
```rust
impl RedisPersistence {
    /// Get all timestamps for a token (for overlap detection)
    pub async fn get_timestamps(&mut self, token: &str) -> Result<Vec<DateTime<Utc>>> {
        let key = format!("snapshots:{}", token);
        let scores: Vec<f64> = self.conn.zrange_withscores(&key, 0, -1)
            .await?
            .into_iter()
            .map(|(_member, score)| score)
            .collect();

        Ok(scores.iter()
            .filter_map(|&score| DateTime::from_timestamp(score as i64, 0))
            .collect())
    }
}
```

---

## Ready for Implementation? ✅ YES

All blocking questions answered:
1. ✅ Volume handling: Use 0.0
2. ✅ Candle struct: Compatible
3. ✅ Redis API: Use existing methods
4. ✅ Timestamp strategy: Only backfill old data
5. ✅ Lookup logic: Fixed
6. ✅ Overlap detection: Time windows

**Next Step:** Update main plan document and proceed with TDD implementation.
