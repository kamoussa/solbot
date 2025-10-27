# CryptoBot

Automated Solana trading bot using momentum strategies with 1-7 day swing trades.

## Status

**✅ Deployed on Railway**
- **Token Discovery**: Automated watchlist from Birdeye trending tokens (every 30 min)
- **Token Rotation**: Automatically removes stale tokens (24h → stale, 7d → removed)
- **Price Collection**: DexScreener real-time + CoinGecko historical backfill (every 5 min)
- **Trading**: Momentum + panic buy signals with circuit breakers
- **Dual Persistence**: Postgres (positions + discovery) + Redis (price candles)

## Quick Start

### Local Development

```bash
# Start services
docker-compose up -d postgres redis

# Run bot
cargo run

# Backfill historical data for a token (optional)
cargo run backfill SOL So11111111111111111111111111111111111111112 --days 7

# Run synthetic backtests
cargo run --bin backtest_real

# Stop services
docker-compose down
```

### Environment Variables

Copy `.env.example` to `.env` and configure:

```bash
# Required APIs
BIRDEYE_API_KEY=your_birdeye_api_key       # Token discovery only
COINGECKO_API_KEY=your_coingecko_api_key   # Historical backfill (optional)
# Note: DexScreener is free (no API key needed)

# Persistence (defaults work for local development)
REDIS_URL=redis://127.0.0.1:6379
DATABASE_URL=postgres://cryptobot:cryptobot_dev_password@localhost:5432/cryptobot

# Portfolio
INITIAL_PORTFOLIO_VALUE=10000.0

# Optional: Trading (not required for paper trading)
WALLET_PRIVATE_KEY=your_base58_private_key
```

## Testing

```bash
# Fast unit tests (no services required)
cargo test

# Integration tests (requires Postgres + Redis)
docker-compose up -d
DATABASE_URL="postgres://cryptobot:cryptobot_dev_password@localhost:5432/cryptobot" \
  cargo test --lib db::postgres -- --ignored --test-threads=1
REDIS_URL="redis://127.0.0.1:6379" \
  cargo test --lib persistence -- --ignored --test-threads=1

# Run backtests
cargo run --bin backtest
```

## Architecture

### Multi-Loop System

The bot runs **3 independent loops** for optimal performance:

1. **Price Fetch Loop** (every 5 min, clock-aligned)
   - Fetches prices from DexScreener for all tracked tokens
   - Stores 5-min candles in Redis (keeps 48h of data)
   - Runs cleanup every hour to remove old data

2. **Trading Loop** (every 5 min, 30s after price fetch)
   - Reads candles from Redis
   - Generates buy/sell signals using MomentumStrategy
   - Executes paper trades via PositionManager
   - Persists positions to Postgres

3. **Discovery Loop** (every 30 min)
   - Fetches trending tokens from Birdeye (top 20 by rank)
   - Applies safety filters (liquidity, volume, FDV)
   - Runs token rotation (marks stale/removed tokens)
   - Backfills historical data for new tokens via CoinGecko
   - Updates watchlist (max 10 tokens)

### Module Structure

```
src/
├── api/
│   ├── dexscreener.rs    Real-time price data
│   ├── birdeye.rs        Token discovery (trending list)
│   └── coingecko.rs      Historical data backfill
├── models/               Token, Candle, Signal, Position
├── indicators/           RSI, Moving Averages, Bollinger Bands
├── strategy/
│   ├── momentum.rs       RSI + MA crossover strategy
│   └── signals.rs        Panic buy + volume spike detection
├── execution/            PriceFeedManager, PositionManager, Executor
├── risk/                 Circuit breakers
├── persistence/          Redis for time-series candles
├── db/                   Postgres for positions + discovery
├── discovery/            Token safety filters
├── backfill/             CoinGecko historical data loader
└── backtest/             Synthetic + real data backtesting
```

## Strategy

### MomentumStrategy
**Lookback**: 24h | **Polling**: 5 min | **Target Hold**: 1-7 days

**Entry Signals**:
1. **Momentum Entry**: RSI < 40 AND 3+ bullish indicators
   - Short MA > Long MA
   - Price > 20-period MA
   - RSI rising (current > previous)
   - Volume spike (2x average, if available)

2. **Panic Buy**: -10% flash crash in <20 min + volume spike
   - Catches oversold bounces
   - Requires volume confirmation
   - Only triggers if RSI < 50

**Exit Conditions**:
- **Stop Loss**: -8% from entry (hard exit)
- **Take Profit**: Trailing stop (activates at +12%, trails by 5%)
- **Time Stop**: Force exit after 14 days
- **Technical Sell**: Sell signal with >5% profit

**Graceful Degradation**:
- If volume data missing (CoinGecko backfill), trades without volume confirmation
- Logs warning when operating without volume features

**Circuit Breakers**:
- Max daily loss: 5%
- Max drawdown: 20%
- Max consecutive losses: 5
- Max position size: 5% of portfolio
- Max daily trades: 10

## Railway Deployment

View logs:
```bash
railway logs --service solbot
```

The bot automatically:
- Runs migrations on startup
- Loads historical candles from Redis
- Loads historical positions from Postgres
- Survives redeployments (data persists)

## Development

### Using Just (recommended)
```bash
# Install just
brew install just

# See all available commands
just

# Pre-commit checks (format, clippy, tests)
just pre-commit

# Run all tests including integration
just pre-push

# Quick check (format + clippy only)
just check
```

### Manual commands
```bash
# Format & lint
cargo fmt && cargo clippy

# Watch mode
cargo watch -x test

# Debug logging
RUST_LOG=debug cargo run
```

## Token Discovery & Rotation

### Safety Filters
Tokens must pass all filters to be added to watchlist:
- **Liquidity**: ≥ $100k USD
- **Volume 24h**: ≥ $50k USD
- **FDV**: ≥ $1M USD (for "credible value")
- **Rank**: Top 50 in Birdeye trending

### Automatic Rotation
- **Active** → **Stale** (24h): Token not seen in trending, stops price fetching
- **Stale** → **Removed** (7d): Archived from watchlist
- **Protected Tokens**: SOL, JUP (never rotated)
- **Position Protection**: Tokens with open positions never rotated

### Must-Track Tokens
Always included regardless of trending status:
- SOL (native Solana)
- JUP (Jupiter DEX)

## What's Next

### Short Term
1. **Strategy Tuning** - Adjust RSI/MA thresholds based on backtest results
2. **Live Testing** - Monitor first trades in production
3. **Volume Feature Validation** - Verify panic buy works with real volume data

### Medium Term
1. **Multi-User Support** - User-specific watchlists and positions
2. **Wallet Integration** - Execute real trades via Jupiter
3. **Advanced Strategies** - Mean reversion, breakout detection

### Long Term
1. **LLM Integration** - AI-powered sentiment analysis and trade reviews
2. **Social Signals** - Twitter/Reddit sentiment tracking
3. **Portfolio Rebalancing** - Dynamic allocation based on market conditions

See `docs/` for detailed planning and architecture docs.
