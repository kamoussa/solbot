# Birdeye-Powered Auto-Trading: Candidate Discovery → LLM Re-rank → Deterministic Execution

**Owner:** you  
**Status:** draft v0.1  
**Date:** 2025-10-17 (ET)

---

## 0) Goal & Non-Goals
**Goal:** Automate “which tokens are worth trading now?” using Birdeye data + an LLM *re-ranker*, while keeping entries/exits/sizing/risk **deterministic**.

**Non-Goals:**  
- Predicting future prices directly with the LLM.  
- Executing without hard safety gates (security/liquidity).  

---

## 1) High-Level Architecture

```
[Birdeye APIs] ---> [Ingest/Cache] ---> [Deterministic Gates] ---> [LLM Re-rank]
                                                    |                   |
                                                    |                   v
                                                    +------> [Rules Engine: entry/exit/size] ---> [Broker/Executor]
                                                                      |
                                                                  [Logger/Backtest/Analytics]
```

- **Ingest/Cache:** Pull OHLCV, trades, price stats, liquidity, security; cache 15–120s.
- **Discovery:** Trending tokens (HTTP) + token lists w/ filters.
- **Gates:** security_ok, min liquidity, min 24h volume, optional token age.
- **LLM Re-rank:** Assign `TradeScore[0-100]` + label using structured features only.
- **Rules Engine:** Confirmed setups (breakout or mean-reversion), ATR stops, slippage caps—no LLM decisions here.

**⚠️ Standard/Free Tier Limitations:**
- Rate limit: ~1 RPS (request per second)
- Credit usage: 30,000 CUs (credit units) per month
- No WebSocket access (premium only)
- No exit-liquidity endpoint (premium only)

---

## 2) Data Model (features we compute)

```ts
type TokenFeatures = {
  token: { address: string; symbol?: string; chain: "solana"|...; age_hours?: number };
  market: { price_usd: number; liquidity_usd?: number; market_cap?: number; fdv?: number };
  volume: { v_5m: number; v_1h: number; v_24h: number };
  momentum: {
    pct_5m: number; pct_1h: number; pct_24h: number;
    ma_state: { m5_cross_m15: boolean; m15_above_m60: boolean };
    atr_5m_pct: number; range_breakout_5m: boolean; range_breakout_1h: boolean;
  };
  flow: {
    trades_5m: number; buys_5m: number; sells_5m: number; imbalance_5m: number;
    // Note: unique buyers/whale tracking may require premium tier
  };
  security: { ok: boolean; flags?: string[] };
};
```

---

## 3) Candidate Discovery (Standard Tier Only)

### 3.1 Sources
- **Trending tokens** (HTTP): `/defi/token_trending` - Sort by volume24hUSD or price change
- **Token list v3 w/ filters:** `/defi/v3/token_list` - Filter by min FDV/liquidity to avoid junk

**Note:** New listings via WebSocket (`SUBSCRIBE_TOKEN_NEW_LISTING`) requires premium tier. Instead, poll trending endpoint every 60-120 seconds to discover new movers.

### 3.2 Discovery Algorithm (Free Tier)
1. Poll `/defi/token_trending` every 60-120 seconds (limit N=20-50).
2. Filter by min liquidity ($20k+) and min 24h volume ($50k+).
3. De-dupe and cap to top K by `(24h volume, liquidity, price %Δ)`.
4. Hand off to **Enrichment**.

**Rate Limit Strategy:** Stagger requests across 60s window to stay under 1 RPS average.

---

## 4) Enrichment: Birdeye API calls (Standard Tier)

**Per candidate (sequential, respecting 1 RPS limit):**
- **Price (current):** `/defi/price?include_liquidity=true` - Get price + liquidity in one call
- **Price/Volume snapshot:** `/defi/price_volume/single` - 24h volume + price
- **OHLCV (1m/5m/1h):** `/defi/ohlcv?interval=5m&limit=100` - Candles for momentum analysis
- **Trades:** `/defi/txs/pair?limit=50` - Recent trades for buy/sell flow
- **Security:** `/defi/token_security` - Security flags (mint auth, freeze, etc.)
- **(Optional)** `/defi/token_overview` - Market cap / FDV

**Not Available on Standard Tier:**
- ❌ Exit liquidity: `/defi/v3/token/exit-liquidity` (premium only)
- ❌ Wallet/holder concentration (premium only)

**Rate Limit Management:**
- Budget ~5 API calls per token (price, volume, ohlcv, trades, security)
- At 1 RPS → analyze 12 tokens/minute → 720 tokens/hour
- Cache aggressively (60-120s TTL) to reduce redundant calls

---

## 5) Deterministic Gates (Standard Tier)
- `security.ok === true` - Must pass security checks
- `liquidity_usd ≥ L0` - e.g., $20k minimum liquidity
- `v_24h ≥ V0` - e.g., $50k minimum 24h volume
- Optional: token age ≥ A0 hours (e.g., 24h minimum for stability)

**Note:** Without exit-liquidity data (premium tier), we rely on regular `liquidity_usd` as a proxy. Conservative filters are critical to avoid low-liquidity traps.

---

## 6) LLM Re-rank (JSON-in/JSON-out)
- Inputs: array of `TokenFeatures`.  
- Output (per token): `{ address, trade_score:0..100, label, reason_short }`.  
- The LLM **cannot** place orders or change stops; it only ranks/labels.

---

## 7) Rules Engine (entries/exits/sizing)
- **Momentum:** `range_breakout_5m && m15_above_m60 && imbalance_5m > θ`
- **Mean-reversion:** large negative ATR-multiple wick + buy domination over 3–5m
- **Size:** `min(R% * equity, I% * liquidity_usd)` - e.g., R=5%, I=2%
- **Stops:** `k_atr * atr_5m_pct` - e.g., 2.5x ATR
- **Slippage cap:** skip if estimated slippage > BPS limit
- **Circuit breakers:** disable if security flips or liquidity drops below threshold

**Standard Tier Adaptation:** Without exit-liquidity data, use conservative position sizing relative to reported `liquidity_usd`. Recommend I% = 1-2% maximum.

---

## 8) Rate Limits (Standard/Free Tier)
- **Rate:** ~1 request per second (RPS)
- **Credits:** 30,000 CUs per month
- **Strategy:**
  - Batch candidates into groups (analyze 1-2 tokens at a time)
  - Sleep 1-2 seconds between API calls
  - Cache results aggressively (60-120s TTL)
  - Use token-bucket or leaky-bucket pattern to smooth request rate

**Example:** At 5 API calls per token, can analyze ~12 tokens/minute → 720 tokens/hour → ~17,000 tokens/day (if running 24/7).

---

## 9) Example `curl` Commands (Standard Tier)

> Replace `YOUR_API_KEY` and set `-H 'x-chain: solana'` for all requests

### 9.1 Trending Tokens
```bash
curl -s 'https://public-api.birdeye.so/defi/token_trending?sort_by=volume24hUSD&limit=20' \
  -H 'x-chain: solana' \
  -H 'X-API-KEY: YOUR_API_KEY'
```

### 9.2 Token List (filter by FDV/liquidity)
```bash
curl -s 'https://public-api.birdeye.so/defi/v3/token_list?min_fdv=1000000&min_liquidity=20000&limit=100&offset=0' \
  -H 'x-chain: solana' \
  -H 'X-API-KEY: YOUR_API_KEY'
```

### 9.3 Current Price + Liquidity
```bash
curl -s 'https://public-api.birdeye.so/defi/price?address=So11111111111111111111111111111111111111112&include_liquidity=true' \
  -H 'x-chain: solana' \
  -H 'X-API-KEY: YOUR_API_KEY'
```

### 9.4 Price + 24h Volume
```bash
curl -s 'https://public-api.birdeye.so/defi/price_volume/single?address=So11111111111111111111111111111111111111112' \
  -H 'x-chain: solana' \
  -H 'X-API-KEY: YOUR_API_KEY'
```

### 9.5 OHLCV Candles (5m interval)
```bash
curl -s 'https://public-api.birdeye.so/defi/ohlcv?address=So11111111111111111111111111111111111111112&interval=5m&limit=100' \
  -H 'x-chain: solana' \
  -H 'X-API-KEY: YOUR_API_KEY'
```

### 9.6 Recent Trades
```bash
curl -s 'https://public-api.birdeye.so/defi/txs/pair?address=PAIR_ADDRESS&limit=50&sort_type=desc' \
  -H 'x-chain: solana' \
  -H 'X-API-KEY: YOUR_API_KEY'
```

### 9.7 Security Check
```bash
curl -s 'https://public-api.birdeye.so/defi/token_security?address=TOKEN_ADDRESS' \
  -H 'x-chain: solana' \
  -H 'X-API-KEY: YOUR_API_KEY'
```

### ❌ Premium Tier Only (Not Available)
```bash
# WebSocket new listings - requires premium
# wss://public-api.birdeye.so/socket/solana?x-api-key=YOUR_API_KEY

# Exit liquidity - requires premium
# curl -s 'https://public-api.birdeye.so/defi/v3/token/exit-liquidity?address=TOKEN_ADDRESS' ...
```

---

## 10) LLM Prompt Contract

**System:**  
“You are a trading signal ranker. Use only the structured fields. Output valid JSON schema below. Keep `reason_short` ≤ 200 chars.”

**User:**
"Given these `TokenFeatures[]`, assign each a `trade_score(0–100)` and `label` ∈ {BlueChipMomentum, MidCapBreakout, MicrocapMomentum, MeanReversion, Illiquid, Risky, NoTrade}. Penalize thin liquidity & security risks; reward multi-TF momentum + buyer participation. Output:
```json
{"version":"v1","results":[{"address":"...","trade_score":87,"label":"MidCapBreakout","reason_short":"..."}]}
```"

---

## 11) Execution Rules (Standard Tier)
- **Enter Momentum:** `score≥80` AND `range_breakout_5m` AND `m15_above_m60` AND `imbalance_5m>θ`
- **Enter MR:** `score≥65` AND `pct_5m < -c*atr_5m_pct` AND buys>sells
- **Size:** `min(R%*equity, I%*liquidity_usd)` - Recommend R=5%, I=1-2%
- **Stop:** `2.5× ATR(5m)` or -8% fixed (whichever is tighter)
- **Take profit:** trailing ATR or 1h momentum rollover
- **Abort:** if security flips or liquidity drops below minimum threshold

**Conservative Sizing:** Without exit-liquidity data, position sizing must be more conservative. Maximum I=2% of reported liquidity.

---

## 12) Backtesting & Telemetry
- Shadow mode: store features + LLM outputs + rule decisions.  
- Calibration: pick score threshold `S0`, imbalance `θ`, ATR multipliers.  
- Batch candidates; sleep/jitter to honor API limits.

---

## 13) Open Questions / Next Steps (Standard Tier)
- Which discovery mix performs best (Trending vs TokenList)?
- What K (candidate count) saturates 1 RPS budget? (Initial estimate: 10-20 tokens per cycle)
- How often to run discovery cycle? (60s? 120s?)
- What cache TTL balances freshness vs rate limits? (60-120s?)
- Future: upgrade to premium for WebSockets, exit-liquidity, holder concentration

---

## Why This Works (Standard Tier)
- **Discovery:** Trending + Filtered TokenList (HTTP polling, no WebSocket required)
- **Safety:** Security checks + liquidity filters + conservative position sizing
- **Momentum:** OHLCV + Trades for technical analysis
- **LLM:** Only ranks/labels, deterministic execution rules
- **Rate Limits:** 1 RPS budget sufficient for 10-20 token analysis per cycle

**Key Insight:** Standard tier provides all essential data for token discovery and technical analysis. Premium features (WebSocket, exit-liquidity, holder data) are nice-to-have but not critical for initial implementation.
