# Birdeye Discovery: Persistence Layer Design

**Goal:** Define what data we store, where, and why for the discovery system.

---

## Storage Systems

**Redis (time-series, ephemeral):**
- Candles (OHLCV)
- API response cache
- Real-time token scores

**Postgres (relational, durable):**
- Tracked tokens metadata
- Strategy assignments
- Discovery history
- Performance metrics
- Token lifecycle events

**Memory (session state):**
- Active strategies per token
- Rate limiter state
- Current discovery cycle

---

## Schema Design

### Postgres Tables

#### 1. `tracked_tokens` - Which tokens we're actively trading

```sql
CREATE TABLE tracked_tokens (
    id SERIAL PRIMARY KEY,
    symbol VARCHAR(20) NOT NULL,
    address VARCHAR(44) NOT NULL UNIQUE,  -- Solana address
    name VARCHAR(100),

    -- Metadata
    category VARCHAR(20) NOT NULL,  -- 'bluechip', 'midcap', 'emerging'
    fdv_usd DECIMAL(20, 2),
    liquidity_usd DECIMAL(20, 2),

    -- Discovery info
    discovered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    discovery_score DECIMAL(5, 2),  -- 0-100 score when discovered
    discovery_rank INTEGER,         -- Rank when discovered (1-50)

    -- Status
    status VARCHAR(20) NOT NULL DEFAULT 'active',  -- 'active', 'paused', 'removed'
    status_reason TEXT,  -- Why paused/removed

    -- Strategy assignment
    strategy_type VARCHAR(50) NOT NULL,  -- 'momentum', 'aggressive_momentum'
    strategy_config JSONB,  -- Full SignalConfig as JSON

    -- Lifecycle
    activated_at TIMESTAMPTZ,  -- When first trade allowed (after 288 candles)
    deactivated_at TIMESTAMPTZ,
    last_scored_at TIMESTAMPTZ,

    -- Performance tracking
    trades_count INTEGER DEFAULT 0,
    win_count INTEGER DEFAULT 0,
    total_pnl_usd DECIMAL(20, 2) DEFAULT 0,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_tracked_tokens_status ON tracked_tokens(status);
CREATE INDEX idx_tracked_tokens_category ON tracked_tokens(category);
CREATE INDEX idx_tracked_tokens_address ON tracked_tokens(address);
```

**Why Postgres?**
- Need to query "which tokens are active?"
- Need to JOIN with positions table
- Need to track lifecycle over weeks/months
- Need ACID for status changes

**Example rows:**
```sql
-- SOL - permanent blue chip
INSERT INTO tracked_tokens (symbol, address, category, status, strategy_type, discovery_score)
VALUES ('SOL', 'So11111...', 'bluechip', 'active', 'momentum', 100);

-- JUP - permanent blue chip
INSERT INTO tracked_tokens (symbol, address, category, status, strategy_type, discovery_score)
VALUES ('JUP', 'JUPyi...', 'bluechip', 'active', 'momentum', 95);

-- BONK - discovered via trending, currently active
INSERT INTO tracked_tokens (symbol, address, category, status, strategy_type, discovery_score, discovery_rank)
VALUES ('BONK', 'DezX...', 'midcap', 'active', 'aggressive_momentum', 87, 3);

-- WIF - discovered but removed due to low score
INSERT INTO tracked_tokens (symbol, address, category, status, strategy_type, discovery_score, status_reason, deactivated_at)
VALUES ('WIF', 'EKpQ...', 'midcap', 'removed', 'aggressive_momentum', 65, 'Score dropped below 60 for 3 cycles', NOW());
```

---

#### 2. `discovery_snapshots` - Historical discovery cycles

```sql
CREATE TABLE discovery_snapshots (
    id SERIAL PRIMARY KEY,
    cycle_timestamp TIMESTAMPTZ NOT NULL,

    -- Discovery config
    discovery_frequency_secs INTEGER,  -- 120s
    trending_limit INTEGER,            -- Top 50 from trending
    min_liquidity_usd DECIMAL(20, 2),
    min_volume_24h_usd DECIMAL(20, 2),

    -- Results
    candidates_count INTEGER,  -- How many passed security filter
    tokens_added INTEGER,      -- How many new tokens added
    tokens_removed INTEGER,    -- How many tokens removed

    -- Metadata
    duration_ms INTEGER,       -- How long discovery took
    api_calls_made INTEGER,    -- Rate limit tracking

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_discovery_snapshots_timestamp ON discovery_snapshots(cycle_timestamp);
```

**Why track this?**
- Debug: "Why didn't we discover BONK during that cycle?"
- Performance: "Is discovery getting slower?"
- Tuning: "What min_liquidity threshold works best?"

---

#### 3. `token_scores` - Historical scoring for each token

```sql
CREATE TABLE token_scores (
    id SERIAL PRIMARY KEY,
    token_address VARCHAR(44) NOT NULL,
    snapshot_id INTEGER REFERENCES discovery_snapshots(id),

    -- Score breakdown
    total_score DECIMAL(5, 2) NOT NULL,
    momentum_score DECIMAL(5, 2),
    liquidity_score DECIMAL(5, 2),
    volume_score DECIMAL(5, 2),
    safety_score DECIMAL(5, 2),
    flow_score DECIMAL(5, 2),

    -- Rank
    rank INTEGER,  -- 1 = best

    -- Metadata for score
    price_change_5m DECIMAL(10, 4),
    price_change_1h DECIMAL(10, 4),
    price_change_24h DECIMAL(10, 4),
    liquidity_usd DECIMAL(20, 2),
    volume_24h_usd DECIMAL(20, 2),
    buy_sell_imbalance DECIMAL(5, 4),  -- -1 to 1

    -- Decision
    action VARCHAR(20),  -- 'add', 'keep', 'remove', 'skip'
    action_reason TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_token_scores_address ON token_scores(token_address);
CREATE INDEX idx_token_scores_snapshot ON token_scores(snapshot_id);
CREATE INDEX idx_token_scores_rank ON token_scores(rank);
```

**Why track this?**
- Backtest: "What was SOL's score during Oct 16 crash?"
- Debug: "Why did BONK rank #3 instead of #1?"
- Tuning: "Does momentum_score actually predict performance?"

**Example query:**
```sql
-- Get score history for SOL over last week
SELECT
    ds.cycle_timestamp,
    ts.total_score,
    ts.rank,
    ts.price_change_24h,
    ts.action
FROM token_scores ts
JOIN discovery_snapshots ds ON ts.snapshot_id = ds.id
WHERE ts.token_address = 'So11111...'
  AND ds.cycle_timestamp > NOW() - INTERVAL '7 days'
ORDER BY ds.cycle_timestamp DESC;
```

---

#### 4. `strategy_performance` - How well does each strategy work per token?

```sql
CREATE TABLE strategy_performance (
    id SERIAL PRIMARY KEY,
    token_address VARCHAR(44) NOT NULL,
    token_category VARCHAR(20) NOT NULL,
    strategy_type VARCHAR(50) NOT NULL,

    -- Performance window
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL,

    -- Metrics
    trades_count INTEGER DEFAULT 0,
    win_count INTEGER DEFAULT 0,
    loss_count INTEGER DEFAULT 0,
    win_rate DECIMAL(5, 4),  -- 0-1

    avg_gain_pct DECIMAL(10, 4),
    avg_loss_pct DECIMAL(10, 4),
    max_gain_pct DECIMAL(10, 4),
    max_loss_pct DECIMAL(10, 4),

    total_pnl_usd DECIMAL(20, 2),
    sharpe_ratio DECIMAL(10, 4),
    max_drawdown_pct DECIMAL(10, 4),

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_strategy_performance_token ON strategy_performance(token_address);
CREATE INDEX idx_strategy_performance_category ON strategy_performance(token_category);
```

**Why track this?**
- Optimization: "Does MomentumStrategy actually work better for blue chips?"
- Tuning: "Should we use RSI 35 or 40 for midcaps?"
- Reporting: "What's our best performing token category?"

**Example query:**
```sql
-- Compare momentum vs aggressive_momentum for midcaps
SELECT
    strategy_type,
    COUNT(*) as tokens,
    AVG(win_rate) as avg_win_rate,
    AVG(total_pnl_usd) as avg_pnl,
    AVG(sharpe_ratio) as avg_sharpe
FROM strategy_performance
WHERE token_category = 'midcap'
  AND window_end > NOW() - INTERVAL '30 days'
GROUP BY strategy_type;
```

---

### Redis Keys

#### 1. Candles (existing, but expanded)

```
candles:{symbol}:{interval} → List of Candle JSON
  Example: candles:SOL:5m → [{timestamp, open, high, low, close, volume}, ...]
  TTL: 48 hours (keep 2 days of history)
```

**Changes from current:**
- Add interval suffix (5m, 1h, 1d) for multi-timeframe
- Store real OHLCV instead of fake candles

---

#### 2. Token scores cache

```
score:{address} → TokenScore JSON
  Example: score:So111...111 → {total_score: 95, category: "bluechip", ...}
  TTL: 120 seconds (refresh every discovery cycle)
```

**Why Redis?**
- Fast lookup: "Is this token still top-ranked?"
- Auto-expire: Old scores don't pollute decisions
- No need to persist - rebuilt every cycle

---

#### 3. API response cache

```
birdeye:trending:{timestamp} → TrendingResponse JSON
  TTL: 60 seconds

birdeye:security:{address} → SecurityCheck JSON
  TTL: 24 hours (security rarely changes)

birdeye:ohlcv:{address}:{interval}:{limit} → Vec<Candle> JSON
  TTL: 30 seconds (price data changes fast)

birdeye:trades:{address} → TradeFlow JSON
  TTL: 30 seconds
```

**Why Redis?**
- Rate limit savings: Cache hit = no API call
- Fast access: Don't wait for Birdeye on every request
- TTL matches data freshness needs

---

#### 4. Discovery state

```
discovery:last_cycle → Timestamp
discovery:active_tokens → Set of token addresses
discovery:candidate_count → Integer (for monitoring)
```

**Why Redis?**
- Ephemeral state doesn't need Postgres
- Fast atomic updates
- Auto-cleanup on restart (rebuild from DB)

---

## Data Flow

### Discovery Cycle Flow

```
1. Check discovery:last_cycle
   └─> If < 120s ago, skip (too soon)

2. Fetch trending (cache: birdeye:trending)
   └─> Parse top 50 candidates

3. For each candidate:
   a. Check birdeye:security:{address} cache
   b. If miss, fetch from API and cache (24h TTL)
   c. Filter: security.ok == false → drop candidate

4. For remaining candidates (maybe 30):
   a. Fetch OHLCV (cache: birdeye:ohlcv)
   b. Fetch trades (cache: birdeye:trades)
   c. Calculate TokenScore
   d. Cache score (TTL: 120s)

5. Rank all scores → top 7

6. Save to Postgres:
   - INSERT discovery_snapshot
   - INSERT token_scores (all 30 candidates)
   - For top 7:
     - INSERT/UPDATE tracked_tokens
     - SET status='active'

7. For tokens not in top 7:
   - UPDATE tracked_tokens SET status='paused' WHERE rank > 10

8. Update Redis:
   - SET discovery:last_cycle NOW()
   - SET discovery:active_tokens (top 7 addresses)
```

---

### Strategy Assignment Flow

```
1. Load tracked_tokens WHERE status='active'

2. For each token:
   a. Read strategy_config from DB (JSONB)
   b. Instantiate Strategy object
   c. Store in memory: HashMap<String, Box<dyn Strategy>>

3. On position close:
   - UPDATE tracked_tokens SET trades_count++, win_count++, total_pnl_usd
   - INSERT strategy_performance (daily rollup)

4. On strategy config change:
   - UPDATE tracked_tokens SET strategy_config, updated_at
   - Reload strategy from DB
```

---

### Token Lifecycle Flow

**Adding a token:**
```sql
-- Step 1: Discovered and scored
INSERT INTO token_scores (address, total_score, rank, ...) VALUES (...);

-- Step 2: Top 7, add to tracked_tokens
INSERT INTO tracked_tokens (
    symbol, address, category,
    status, strategy_type, strategy_config,
    discovery_score, discovery_rank, discovered_at
) VALUES (
    'BONK', 'DezX...', 'midcap',
    'active', 'aggressive_momentum', '{"rsi_oversold": 35, ...}',
    87, 3, NOW()
);

-- Step 3: Backfill candles (or wait 24h)
-- Redis: candles:BONK:5m ← fetch from Birdeye historical

-- Step 4: Mark as activated when enough data
UPDATE tracked_tokens
SET activated_at = NOW()
WHERE symbol = 'BONK' AND candle_count >= 288;
```

**Removing a token:**
```sql
-- Step 1: Score drops below threshold for 3 consecutive cycles
-- (checked in application logic)

-- Step 2: Mark as paused (don't delete, keep history)
UPDATE tracked_tokens
SET
    status = 'paused',
    status_reason = 'Score below 60 for 3 cycles',
    deactivated_at = NOW()
WHERE symbol = 'BONK';

-- Step 3: If no open positions, can fully remove
UPDATE tracked_tokens
SET status = 'removed'
WHERE symbol = 'BONK' AND id NOT IN (
    SELECT DISTINCT token FROM positions WHERE closed_at IS NULL
);

-- Step 4: Keep in DB for historical analysis
-- (Never DELETE, just mark removed)
```

---

## Queries Needed

### Application Queries

```rust
// Get all active tokens
pub async fn get_active_tokens(&self) -> Result<Vec<TrackedToken>> {
    sqlx::query_as!(
        TrackedToken,
        "SELECT * FROM tracked_tokens WHERE status = 'active'"
    )
    .fetch_all(&self.pool)
    .await
}

// Get strategy config for token
pub async fn get_strategy_config(&self, symbol: &str) -> Result<SignalConfig> {
    let row = sqlx::query!(
        "SELECT strategy_config FROM tracked_tokens WHERE symbol = $1",
        symbol
    )
    .fetch_one(&self.pool)
    .await?;

    serde_json::from_value(row.strategy_config)
}

// Save discovery snapshot
pub async fn save_discovery_snapshot(
    &self,
    candidates_count: i32,
    tokens_added: i32,
    duration_ms: i32
) -> Result<i32> {
    let row = sqlx::query!(
        "INSERT INTO discovery_snapshots (
            cycle_timestamp, candidates_count, tokens_added, duration_ms
        ) VALUES (NOW(), $1, $2, $3) RETURNING id",
        candidates_count, tokens_added, duration_ms
    )
    .fetch_one(&self.pool)
    .await?;

    Ok(row.id)
}

// Save token scores
pub async fn save_token_scores(
    &self,
    snapshot_id: i32,
    scores: Vec<TokenScore>
) -> Result<()> {
    for score in scores {
        sqlx::query!(
            "INSERT INTO token_scores (
                snapshot_id, token_address, total_score, rank, action
            ) VALUES ($1, $2, $3, $4, $5)",
            snapshot_id, score.address, score.total_score, score.rank, score.action
        )
        .execute(&self.pool)
        .await?;
    }
    Ok(())
}

// Update token performance
pub async fn update_token_performance(
    &self,
    symbol: &str,
    pnl: f64,
    is_win: bool
) -> Result<()> {
    sqlx::query!(
        "UPDATE tracked_tokens
         SET trades_count = trades_count + 1,
             win_count = win_count + $1,
             total_pnl_usd = total_pnl_usd + $2
         WHERE symbol = $3",
        if is_win { 1 } else { 0 },
        pnl,
        symbol
    )
    .execute(&self.pool)
    .await?;

    Ok(())
}
```

---

### Analytics Queries

```sql
-- Top performing tokens (last 30 days)
SELECT
    tt.symbol,
    tt.category,
    tt.trades_count,
    tt.win_count,
    ROUND(tt.win_count::DECIMAL / NULLIF(tt.trades_count, 0), 2) as win_rate,
    tt.total_pnl_usd
FROM tracked_tokens tt
WHERE tt.trades_count > 5
  AND tt.discovered_at > NOW() - INTERVAL '30 days'
ORDER BY tt.total_pnl_usd DESC
LIMIT 10;

-- Discovery effectiveness (are high scores actually profitable?)
SELECT
    CASE
        WHEN ts.total_score >= 80 THEN 'Excellent (80+)'
        WHEN ts.total_score >= 60 THEN 'Good (60-80)'
        ELSE 'Poor (<60)'
    END as score_tier,
    COUNT(DISTINCT tt.symbol) as tokens,
    AVG(tt.total_pnl_usd) as avg_pnl,
    AVG(tt.win_count::DECIMAL / NULLIF(tt.trades_count, 0)) as avg_win_rate
FROM token_scores ts
JOIN tracked_tokens tt ON ts.token_address = tt.address
WHERE tt.trades_count > 0
GROUP BY score_tier;

-- Strategy performance by category
SELECT
    tt.category,
    tt.strategy_type,
    COUNT(*) as tokens,
    SUM(tt.trades_count) as total_trades,
    AVG(tt.win_count::DECIMAL / NULLIF(tt.trades_count, 0)) as avg_win_rate,
    SUM(tt.total_pnl_usd) as total_pnl
FROM tracked_tokens tt
WHERE tt.trades_count > 0
GROUP BY tt.category, tt.strategy_type
ORDER BY total_pnl DESC;
```

---

## Migration Plan

### Phase 0: Schema creation

```sql
-- Run migrations
CREATE TABLE tracked_tokens (...);
CREATE TABLE discovery_snapshots (...);
CREATE TABLE token_scores (...);
CREATE TABLE strategy_performance (...);

-- Seed with current tokens
INSERT INTO tracked_tokens (symbol, address, category, status, strategy_type)
VALUES
    ('SOL', 'So111...', 'bluechip', 'active', 'momentum'),
    ('JUP', 'JUPyi...', 'bluechip', 'active', 'momentum');
```

### Phase 1: Read-only integration

```rust
// Load active tokens on startup
let tokens = db.get_active_tokens().await?;

// Still use hardcoded strategies for now
// Just validate DB has correct data
```

### Phase 2: Write discovery results

```rust
// After discovery cycle
let snapshot_id = db.save_discovery_snapshot(...).await?;
db.save_token_scores(snapshot_id, scores).await?;

// Don't auto-add tokens yet, just log
```

### Phase 3: Dynamic token management

```rust
// Add/remove tokens based on scores
for score in top_7 {
    db.upsert_tracked_token(score).await?;
}

// Pause low-scoring tokens
db.pause_tokens_below_threshold(60.0).await?;
```

### Phase 4: Analytics & tuning

```sql
-- Query historical data
-- Tune scoring weights
-- Validate strategy assignments
```

---

## Open Questions

1. **How long to keep removed tokens in DB?**
   - Forever (disk is cheap, history is valuable)?
   - 90 days (compliance/audit window)?
   - Until no positions reference them?

2. **Should we version strategy configs?**
   - Track when we changed RSI from 40 to 35?
   - Or just overwrite and assume latest is correct?

3. **How to handle token renames/migrations?**
   - BONK changes address due to contract upgrade?
   - Store old_address → new_address mapping?

4. **Cache invalidation strategy?**
   - Manual FLUSHDB on schema changes?
   - Or let TTL handle everything?

5. **Backup/disaster recovery?**
   - Redis is ephemeral (OK to lose on restart)
   - Postgres needs regular backups
   - Railway handles this automatically?

---

## Summary

**Postgres stores:**
- tracked_tokens (which tokens, what strategies)
- discovery_snapshots (when we ran discovery)
- token_scores (historical rankings)
- strategy_performance (what works)

**Redis stores:**
- Candles (time-series OHLCV)
- API response cache (rate limit optimization)
- Token scores (current rankings)
- Discovery state (ephemeral)

**Key insight:**
- Postgres = source of truth for "what are we trading"
- Redis = optimization layer for "what's the current price/score"
- Never lose trade history, always queryable for analytics
