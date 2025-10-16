# CryptoBot

Automated Solana trading bot using momentum strategies with 1-7 day swing trades.

## Status

**✅ Deployed on Railway**
- Collecting SOL & JUP price data every 5 minutes
- Dual persistence: Postgres (positions) + Redis (candles)
- Needs 288 snapshots (~24h) before strategy activates

## Quick Start

### Local Development

```bash
# Start services
docker-compose up -d postgres redis

# Run bot
cargo run

# Run backtests
cargo run --bin backtest

# Stop services
docker-compose down
```

### Environment Variables

Copy `.env.example` to `.env` and configure:

```bash
# Required for trading (not required for testing)
WALLET_PRIVATE_KEY=your_base58_private_key

# Persistence (defaults work for local development)
REDIS_URL=redis://127.0.0.1:6379
DATABASE_URL=postgres://cryptobot:cryptobot_dev_password@localhost:5432/cryptobot

# Portfolio
INITIAL_PORTFOLIO_VALUE=10000.0
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

```
src/
├── api/              DexScreener & Jupiter API clients
├── models/           Token, Candle, Signal, Position
├── indicators/       RSI, Moving Averages
├── strategy/         MomentumStrategy (RSI + MA crossover)
├── execution/        PriceFeedManager, PositionManager, Executor
├── risk/             Circuit breakers (5% daily loss, 20% drawdown)
├── persistence/      Redis for candles (time-series)
├── db/               Postgres for positions (transactional)
└── backtest/         Synthetic data generator & performance metrics
```

## Strategy

**MomentumStrategy** - 24h lookback, 5min polling

**Entry**: Buy when RSI < 30 AND short MA crosses above long MA
**Exit**:
  - Stop loss: -8% from entry
  - Take profit: Trailing stop (activates at +12%, trails by 5%)
  - Time stop: 14 days max hold

**Circuit Breakers**:
  - Max daily loss: 5%
  - Max drawdown: 20%
  - Max consecutive losses: 5
  - Max position size: 5% of portfolio

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

```bash
# Format & lint
cargo fmt && cargo clippy

# Watch mode
cargo watch -x test

# Debug logging
RUST_LOG=debug cargo run
```

## What's Next

1. **Tune Strategy** - Current parameters are too conservative (0 trades in backtests)
2. **Wallet Integration** - Execute real trades via Jupiter
3. **LLM Layer** - Watchlist curation and sentiment analysis

See `docs/` for detailed planning and architecture docs.
