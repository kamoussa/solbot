# Birdeye Implementation Plan

**Owner:** Team
**Status:** Draft v0.1
**Date:** 2025-10-17
**Goal:** Add multi-token discovery via Birdeye API, enabling automated token selection and trading

---

## Motivation

**Current limitations:**
- Fixed 2 tokens (SOL, JUP) - missing opportunities
- Fake candles (price snapshots) - poor indicator accuracy
- No volume spike detection - panic buy can't work properly
- No security checks - could trade scam tokens
- No buy/sell flow data - missing key signals

**Birdeye enables:**
- Discover trending tokens automatically (20-50 per cycle)
- Real OHLCV candles with per-interval volume
- Security filtering (mint auth, freeze, liquidity locks)
- Trade-level data for buy/sell imbalance
- Per-token strategy configuration

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        DISCOVERY LAYER                          │
│  Birdeye Trending API → Security Filter → Deterministic Rank   │
└────────────────────────────┬────────────────────────────────────┘
                             │ Top 5-10 tokens
                             v
┌─────────────────────────────────────────────────────────────────┐
│                      ENRICHMENT LAYER                           │
│  For each token: OHLCV + Trades + Liquidity → Feature Extract  │
└────────────────────────────┬────────────────────────────────────┘
                             │ TokenFeatures[]
                             v
┌─────────────────────────────────────────────────────────────────┐
│                      STRATEGY LAYER                             │
│  Match token → strategy (BlueChip=Momentum, MidCap=Aggressive)  │
└────────────────────────────┬────────────────────────────────────┘
                             │ Signal[]
                             v
┌─────────────────────────────────────────────────────────────────┐
│                      EXECUTION LAYER                            │
│  Position Manager (multi-position) → Risk checks → Execute      │
└─────────────────────────────────────────────────────────────────┘
```

**Key insight:** Deterministic ranking replaces LLM initially. LLM becomes enhancement layer later.

---

## Phase Breakdown

### Phase 0: API Client Foundation (Week 1)
**Goal:** Get Birdeye trending data flowing, hybrid with DexScreener for candles

**REALITY CHECK:** Free tier only includes:
- ✅ `/defi/price` - Current price + liquidity
- ✅ `/defi/token_trending` - **Perfect for discovery!** (1000 tokens with price, volume, liquidity, FDV, 24h change)
- ❌ `/defi/ohlcv` - REQUIRES PREMIUM (cannot get real candles)
- ❌ `/defi/token_security` - REQUIRES PREMIUM
- ❌ `/defi/txs/pair` - REQUIRES PREMIUM

**Revised Approach: Hybrid Architecture**
- **Discovery:** Birdeye `/defi/token_trending` (excellent metadata)
- **Candles:** Keep DexScreener (already works, free, good enough)
- **Security:** Manual curation (allowlist of known-good tokens: SOL, JUP, BONK, etc.)
- **Later:** Upgrade to Birdeye premium if we need real OHLCV + security

**Tasks:**
1. Implement `BirdeyeClient` with trending + price endpoints only
2. Test trending endpoint - validate 1000 tokens returned
3. Keep DexScreener for candle data (don't replace it)
4. Add API key to .env

**Deliverables:**
- `src/api/birdeye.rs` - Trending + price only (remove unused OHLCV/security code)
- Unit tests for trending endpoint
- Validate trending data quality
- Rate limit telemetry

**Endpoints implemented:**
```rust
pub struct BirdeyeClient {
    // Free tier endpoints only
    pub async fn get_price(&self, address: &str) -> Result<(f64, Option<f64>)>;
    pub async fn get_trending(&self, sort_by: &str, limit: usize) -> Result<Vec<TrendingToken>>;
}
```

**Success criteria:**
- Can fetch 50-1000 trending tokens per cycle
- Trending data includes liquidity, volume, price change %
- Rate limiting stays under 1 RPS average (trending is 1 call per cycle)
- Can rank tokens using trending metadata alone

---

### Phase 1: Keep DexScreener, Focus on Discovery (Week 2)
**Goal:** Don't replace candles (free tier can't), focus on discovery ranking instead

**DECISION:** Keep DexScreener for candles
- DexScreener is free and works fine
- Birdeye free tier doesn't have OHLCV anyway
- "Fake" candles (price snapshots) are good enough for our indicators
- Focus effort on discovery, not data quality upgrade

**Tasks:**
1. Implement trending endpoint parser
2. Build deterministic ranking algorithm (no LLM)
3. Add manual security allowlist (SOL, JUP, BONK, RAY, ORCA)
4. Test ranking on real trending data

**Ranking without security checks:**
```rust
// Simplified scoring (no buy/sell flow, no security API)
fn calculate_score(token: &TrendingToken) -> f64 {
    let momentum_score = score_momentum(&token);      // 0-40 pts (price changes)
    let liquidity_score = score_liquidity(&token);    // 0-30 pts (absolute liquidity)
    let volume_score = score_volume(&token);          // 0-30 pts (volume/liquidity ratio)

    momentum_score + liquidity_score + volume_score  // 0-100 total
}

// Manual security filter
const ALLOWLIST: &[&str] = &[
    "So11111111111111111111111111111111111111112",  // SOL
    "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN", // JUP
    "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263", // BONK
    // ... add more known-good tokens
];
```

**Success criteria:**
- Can rank 50-1000 trending tokens per cycle
- SOL/JUP always in top 10
- Scam tokens filtered by allowlist
- Deterministic and explainable scores

---

### Phase 2: Discovery + Deterministic Ranking (Week 3-4)
**Goal:** Auto-discover tokens, rank without LLM using composite score

**Tasks:**
1. Implement trending endpoint polling
2. Build deterministic ranking algorithm
3. Security gate filtering
4. Top-K selection logic

#### Deterministic Ranking Algorithm (Free Tier Constraints)

**Approach: Weighted composite score (0-100)**

```rust
pub struct TokenScore {
    address: String,
    symbol: String,
    total_score: f64,      // 0-100
    category: TokenCategory,
    breakdown: ScoreBreakdown,
}

pub enum TokenCategory {
    BlueChip,      // SOL, JUP, BONK - use MomentumStrategy
    MidCap,        // $100M-$1B FDV - use Aggressive params
    Emerging,      // $10M-$100M - use Conservative params
    Microcap,      // <$10M - skip for now (too risky)
}

pub struct ScoreBreakdown {
    momentum_score: f64,    // 0-40 points (increased weight)
    liquidity_score: f64,   // 0-30 points (increased weight)
    volume_score: f64,      // 0-30 points (increased weight)
    // safety_score: Replaced by allowlist filter
    // flow_score: Not available on free tier
}
```

**Scoring formula (free tier only has: price, liquidity, volume, 24h change):**

```
MOMENTUM (0-40 pts):
  - 24h price change (only field available):
    - >20%: +40 pts (strong momentum)
    - >10%: +30 pts (good momentum)
    - >5%: +20 pts (moderate)
    - >2%: +10 pts (slight)
    - <2%: +5 pts (flat)
  - Note: No 5m/1h data on free tier

LIQUIDITY (0-30 pts):
  - Liquidity > $10M: +30 pts (blue chip)
  - Liquidity > $1M: +25 pts (mid cap)
  - Liquidity > $500k: +15 pts (emerging)
  - Liquidity > $100k: +5 pts (microcap)
  - Liquidity < $100k: 0 pts (filtered out)

VOLUME (0-30 pts):
  - 24h vol / liquidity ratio:
    - >10x: +30 pts (extremely active)
    - >5x: +20 pts (very active)
    - >2x: +10 pts (active)
    - <2x: +5 pts (slow)

SECURITY (allowlist only):
  - If address NOT in allowlist → score = 0 (instant disqualify)
  - If address IN allowlist → continue scoring
  - No API-based security checks (not available)

TOTAL: momentum + liquidity + volume → 0-100
```

**Categorization:**
```rust
fn categorize(fdv: f64, liquidity: f64) -> TokenCategory {
    match (fdv, liquidity) {
        (f, _) if f > 1_000_000_000.0 => TokenCategory::BlueChip,
        (f, l) if f > 100_000_000.0 && l > 500_000.0 => TokenCategory::MidCap,
        (f, l) if f > 10_000_000.0 && l > 100_000.0 => TokenCategory::Emerging,
        _ => TokenCategory::Microcap,  // Skip
    }
}
```

**Ranking logic:**
```rust
pub fn rank_tokens(candidates: Vec<TokenFeatures>) -> Vec<TokenScore> {
    let mut scored: Vec<TokenScore> = candidates
        .into_iter()
        .filter(|t| t.security.ok)  // Hard filter
        .filter(|t| t.market.liquidity_usd > 100_000.0)  // Hard filter
        .map(|t| calculate_score(t))
        .collect();

    // Sort by total_score descending
    scored.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap());

    // Take top K per category
    let top_bluechip = take_top(&scored, TokenCategory::BlueChip, 2);
    let top_midcap = take_top(&scored, TokenCategory::MidCap, 3);
    let top_emerging = take_top(&scored, TokenCategory::Emerging, 2);

    // Max 7 tokens total
    [top_bluechip, top_midcap, top_emerging].concat()
}
```

**Advantages over LLM:**
- Fully deterministic and explainable
- No API costs or latency
- Easy to tune weights via backtesting
- Can log exact score breakdown for debugging

**When to add LLM:**
- After we validate deterministic ranking works
- LLM can add "sentiment layer" on top of score
- Example: Score says "buy", but LLM sees news about rug → override to "skip"

**Deliverables:**
- `src/discovery/mod.rs` - Discovery engine
- `src/discovery/ranking.rs` - Deterministic scoring
- Unit tests for scoring edge cases
- Integration test: discover + rank trending tokens

**Success criteria:**
- Discovers 20-50 tokens per cycle
- Ranks top 5-10 with explainable scores
- Blue chip tokens (SOL/JUP) always in top tier
- No scam tokens pass security filter

---

### Phase 3: Per-Token Strategy Matching (Week 5)
**Goal:** Different tokens get different strategies and parameters

**Design:**
```rust
pub struct StrategyConfig {
    pub strategy_type: StrategyType,
    pub params: SignalConfig,
}

pub enum StrategyType {
    Momentum,          // Conservative dip-buying for blue chips
    AggressiveMomentum, // Tighter stops, more frequent trades
    // Future: Breakout, MeanReversion, etc.
}

pub fn match_strategy(category: TokenCategory) -> StrategyConfig {
    match category {
        TokenCategory::BlueChip => StrategyConfig {
            strategy_type: StrategyType::Momentum,
            params: SignalConfig {
                rsi_period: 14,
                rsi_oversold: 40.0,
                short_ma_period: 10,
                long_ma_period: 20,
                volume_threshold: 1.5,
                enable_panic_buy: true,  // Safe for blue chips
                panic_rsi_threshold: 30.0,
                panic_price_drop_pct: 8.0,
                ..Default::default()
            },
        },
        TokenCategory::MidCap => StrategyConfig {
            strategy_type: StrategyType::AggressiveMomentum,
            params: SignalConfig {
                rsi_period: 10,
                rsi_oversold: 35.0,  // Earlier entry
                short_ma_period: 5,
                long_ma_period: 15,
                volume_threshold: 2.0,  // Need stronger volume
                enable_panic_buy: false,  // Too risky for midcaps
                ..Default::default()
            },
        },
        TokenCategory::Emerging => StrategyConfig {
            strategy_type: StrategyType::Momentum,
            params: SignalConfig {
                rsi_period: 14,
                rsi_oversold: 30.0,  // Very oversold only
                short_ma_period: 10,
                long_ma_period: 30,  // Longer trend confirmation
                volume_threshold: 3.0,  // Need huge volume
                enable_panic_buy: false,
                ..Default::default()
            },
        },
        TokenCategory::Microcap => {
            // Don't trade these for now
            panic!("Microcap tokens filtered at discovery")
        }
    }
}
```

**Changes to main loop:**
```rust
// Before: Single strategy for all tokens
let strategy = MomentumStrategy::default();

// After: Per-token strategy
let mut token_strategies: HashMap<String, Box<dyn Strategy>> = HashMap::new();

for token_score in discovered_tokens {
    let config = match_strategy(token_score.category);
    let strategy: Box<dyn Strategy> = match config.strategy_type {
        StrategyType::Momentum => Box::new(MomentumStrategy::new(config.params)),
        StrategyType::AggressiveMomentum => Box::new(MomentumStrategy::new(config.params)),
    };
    token_strategies.insert(token_score.symbol.clone(), strategy);
}

// In trading loop
for (symbol, strategy) in &token_strategies {
    let signal = strategy.generate_signal(&candles)?;
    // ...
}
```

**Deliverables:**
- `src/strategy/config.rs` - Strategy matching logic
- Per-token strategy tests
- Backtest comparing same vs different params for SOL vs microcap

**Success criteria:**
- Blue chips use conservative params
- Mid/emerging use stricter filters
- Each token's strategy is logged and explainable

---

### Phase 4: Multi-Position Portfolio Manager (Week 6)
**Goal:** Support multiple open positions simultaneously

**Current limitation:**
```rust
// PositionManager assumes single position at a time
pub fn open_position(&mut self, token: String, price: f64, quantity: f64) -> Result<u64>
```

**New design:**
```rust
pub struct PositionManager {
    positions: Vec<Position>,
    max_positions: usize,           // e.g., 5
    max_position_pct: f64,          // e.g., 0.20 (20% per token)
    max_category_allocation: HashMap<TokenCategory, f64>,
}

impl PositionManager {
    pub fn can_open_position(&self, category: TokenCategory, size_usd: f64) -> bool {
        // Check: total positions < max
        if self.open_positions().len() >= self.max_positions {
            return false;
        }

        // Check: category allocation
        let category_exposure = self.category_exposure(category);
        let category_limit = self.max_category_allocation.get(&category).unwrap_or(&0.5);
        if category_exposure + size_usd > self.equity * category_limit {
            return false;
        }

        // Check: per-position limit
        if size_usd > self.equity * self.max_position_pct {
            return false;
        }

        true
    }
}
```

**Portfolio allocation:**
```
Max 5 positions:
- BlueChip: 2 positions, max 40% total (20% each)
- MidCap: 2 positions, max 30% total (15% each)
- Emerging: 1 position, max 10% total
- Cash reserve: 20% minimum
```

**Deliverables:**
- Enhanced `PositionManager` with multi-position support
- Category-based allocation limits
- Portfolio rebalancing logic
- Tests for allocation limits

**Success criteria:**
- Can hold 5 positions simultaneously
- No category over-concentration
- Cash reserve maintained for opportunities

---

### Phase 5: Rate Limit & Caching (Week 7)
**Goal:** Stay under 1 RPS with smart caching

**Rate limit budget:**
```
Discovery cycle (every 120s):
- Trending: 1 call
- Security checks (10 tokens): 10 calls
- OHLCV (top 7): 7 calls
- Trades (top 7): 7 calls
- Total: 25 calls per 120s = 0.21 RPS avg ✅

Buffer: Leave 0.8 RPS for burst/retry
```

**Caching strategy:**
```rust
pub struct BirdeyeCache {
    // Cache trending for 60s (changes slowly)
    trending: TtlCache<String, Vec<TrendingToken>>,

    // Cache OHLCV for 30s (updates frequently)
    ohlcv: TtlCache<String, Vec<Candle>>,

    // Cache security for 24h (rarely changes)
    security: TtlCache<String, SecurityCheck>,
}
```

**Request throttling:**
```rust
pub struct RateLimiter {
    last_request: Instant,
    min_interval: Duration,  // 1000ms for 1 RPS
}

impl RateLimiter {
    pub async fn wait(&mut self) {
        let elapsed = self.last_request.elapsed();
        if elapsed < self.min_interval {
            tokio::time::sleep(self.min_interval - elapsed).await;
        }
        self.last_request = Instant::now();
    }
}
```

**Deliverables:**
- TTL cache for API responses
- Rate limiter with backoff
- Telemetry for actual RPS usage
- Tests for cache hit rates

**Success criteria:**
- Actual RPS < 0.5 average
- Cache hit rate > 50%
- No rate limit errors from Birdeye

---

### Phase 6: LLM Enhancement Layer (Week 8+, Optional)
**Goal:** Add LLM re-ranking on top of deterministic scores

**Design:**
```rust
pub async fn llm_rerank(scored_tokens: Vec<TokenScore>) -> Vec<TokenScore> {
    // Take top 15 from deterministic ranking
    let candidates = scored_tokens.into_iter().take(15).collect();

    // Send to LLM with structured prompt
    let llm_scores = llm_client.rank_tokens(&candidates).await?;

    // Blend: 70% deterministic + 30% LLM
    for (token, llm_score) in candidates.iter_mut().zip(llm_scores) {
        token.total_score = token.total_score * 0.7 + llm_score * 0.3;
    }

    // Re-sort and return top 7
    candidates.sort_by_score();
    candidates.into_iter().take(7).collect()
}
```

**LLM adds:**
- Sentiment analysis (news, social mentions)
- Pattern recognition (chart patterns)
- Anomaly detection (unusual behavior)
- Risk assessment (narrative/fundamental)

**Not in scope for Phase 6:**
- LLM execution decisions (stays deterministic)
- LLM position sizing (stays rule-based)

---

## Data Model

**New types needed:**

```rust
// Discovery
pub struct TrendingToken {
    pub address: String,
    pub symbol: String,
    pub price_usd: f64,
    pub price_change_5m: f64,
    pub price_change_1h: f64,
    pub price_change_24h: f64,
    pub volume_24h: f64,
    pub liquidity_usd: f64,
    pub fdv: Option<f64>,
}

pub struct SecurityCheck {
    pub ok: bool,
    pub has_mint_authority: bool,
    pub has_freeze_authority: bool,
    pub is_liquidity_locked: bool,
    pub risk_flags: Vec<String>,
}

pub struct TradeFlow {
    pub buys: usize,
    pub sells: usize,
    pub buy_volume_usd: f64,
    pub sell_volume_usd: f64,
    pub imbalance: f64,  // (buys - sells) / (buys + sells)
}

// Ranking
pub struct TokenFeatures {
    pub token: TrendingToken,
    pub security: SecurityCheck,
    pub flow: TradeFlow,
    pub candles: Vec<Candle>,  // Last 100 for analysis
}

pub struct TokenScore {
    pub address: String,
    pub symbol: String,
    pub total_score: f64,
    pub category: TokenCategory,
    pub breakdown: ScoreBreakdown,
    pub reason: String,  // Why this score
}
```

---

## Testing Strategy

**Unit tests:**
- Scoring algorithm edge cases
- Security filtering
- Rate limiting logic
- Cache TTL behavior

**Integration tests:**
- Discover + rank actual trending tokens
- Backfill historical OHLCV
- Multi-position portfolio scenarios

**Backtests:**
- Oct 16 SOL crash with real Birdeye data
- Compare deterministic vs LLM ranking (later)
- Validate per-token strategies on different market caps

**Shadow mode:**
- Run discovery in parallel with existing SOL/JUP bot
- Log what trades discovery would have made
- Compare P&L after 1 week

---

## Risks & Mitigations

**Risk 1: Rate limits too tight**
- Mitigation: Aggressive caching, limit to 3 tokens initially (SOL + JUP + 1 discovery)
- Actual budget: ~15 calls per 120s = 0.125 RPS (well under 1 RPS limit)
- Fallback: Reduce discovery frequency to 300s

**Risk 2: Too many false positives (scam tokens pass filter)**
- Mitigation: CRITICAL security rules (auto-reject mint/freeze authority)
- Shadow mode first (log decisions, don't auto-trade)
- Manual approval required for first 2 weeks
- Fallback: Manually curated allowlist initially

**Risk 3: Token churn (constantly swapping tokens)**
- Mitigation: Minimum 24h hold period after adding token
- Keep token while open position exists (even if score drops)
- Stable core (SOL/JUP never removed)
- Fallback: Increase min score threshold to 80 (only excellent tokens)

**Risk 4: Discovery adds rug tokens**
- Mitigation: Circuit breaker (max 1 new token per day)
- Require manual approval in Week 3
- Shadow mode validation in Week 2
- Fallback: Disable auto-discovery, manual token selection only

**Risk 5: Cold start problem (new tokens have no history)**
- Mitigation: Backfill 288 candles from Birdeye historical OHLCV
- Don't trade until backfill validated (no gaps)
- Fallback: Wait 24h collecting real-time data

---

## Success Metrics

**Phase 0-1:** Data quality
- Real candles vs fake candles indicator accuracy
- Volume spike detection rate

**Phase 2:** Discovery quality
- % of discovered tokens that are legitimate (not scams)
- Top 7 tokens include SOL/JUP consistently

**Phase 3:** Strategy matching
- Blue chip P&L > 0 over 1 week
- Mid/emerging caps have tighter risk management (lower max drawdown)

**Phase 4:** Portfolio performance
- Multi-token portfolio Sharpe > single-token
- Drawdown < 15% over 1 month

**Phase 5:** Operational
- Rate limit usage < 50% of budget
- Cache hit rate > 50%
- No API errors

---

## Revised Timeline (MVP-First Approach)

**MVP Path (3 weeks) - Incremental validation:**

```
Week 1: Foundation + Real Candles
  ✅ BirdeyeClient with OHLCV + Security endpoints
  ✅ Postgres schema (tracked_tokens, discovery_snapshots, token_scores)
  ✅ Replace DexScreener with Birdeye for SOL/JUP
  ✅ Validate: Panic buy works better with real volume data

Week 2: Shadow Discovery
  ✅ Discovery engine (trending + security filter + deterministic ranking)
  ✅ Save to DB, log results, but DON'T auto-trade
  ✅ Validate: "Would we have picked profitable tokens over last week?"

Week 3: Single Discovery Slot
  ✅ Add top 1 discovery token (alongside SOL/JUP)
  ✅ Use same MomentumStrategy for all 3 tokens
  ✅ Validate: "Does it make money in production?"
```

**Post-MVP (4+ weeks) - Scale up:**

```
Week 4-5: Multi-position (up to 5 tokens)
Week 6: Per-token strategy matching
Week 7: Rate limit optimization + caching
Week 8+: LLM enhancement (optional)
```

**Key differences from original plan:**
- Phase 2A/2B split (shadow mode before live trading)
- Single discovery slot initially (not 7)
- Defer per-token strategies (use MomentumStrategy for all)
- Defer advanced caching (simple TTL is fine initially)

---

## Open Questions

1. **Discovery frequency:** 60s, 120s, or 300s?
   - Faster = more responsive, but higher rate limit usage
   - Recommendation: Start 120s, tune based on telemetry

2. **Top K selection:** 5, 7, or 10 tokens?
   - More = diversification, less = focus
   - Recommendation: 7 (2 blue + 3 mid + 2 emerging)

3. **Minimum token age:** 24h, 72h, or 1 week?
   - Older = safer, but miss early pumps
   - Recommendation: 24h minimum, 72h for non-blue chips

4. **Position sizing:** Fixed 20% or dynamic based on score?
   - Fixed = simpler, dynamic = optimized
   - Recommendation: Fixed initially, dynamic in Phase 4

5. **Backfill strategy:** Fetch 24h history or wait?
   - Backfill = trade immediately, wait = safer
   - Recommendation: Backfill for discovered tokens, validate quality

---

## Next Steps

1. Critique this plan - what's missing? What's wrong?
2. Implement Phase 0 (API client)
3. Test real Birdeye data quality vs DexScreener
4. Validate deterministic ranking on current trending tokens
5. Shadow mode comparison: discovery vs hardcoded SOL/JUP
