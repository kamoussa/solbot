# CryptoBot

A Solana-based cryptocurrency trading bot using swing trading strategies (1-7 day holds). Combines technical analysis for tactical decisions with Redis persistence for historical data.

## Features

- **Price Tracking**: Real-time price data from DexScreener API
- **Technical Analysis**: RSI, Moving Averages, Volume analysis
- **Momentum Strategy**: Multi-indicator signal generation
- **Redis Persistence**: Historical data storage to eliminate cold-start periods
- **Graceful Degradation**: Works with or without Redis
- **Circuit Breakers**: Risk management and loss prevention

## Quick Start

**Current Status**: MVP - Single User

The bot is currently designed for a single user. Multi-user support is planned for the future (see `docs/MULTI_USER_ARCHITECTURE.md`).

### Prerequisites

- Rust 1.75+ (install via [rustup](https://rustup.rs/))
- Optional: Docker & Docker Compose (for Redis)
- Solana wallet private key (for trading)

### Installation

```bash
git clone <your-repo-url>
cd cryptobot
cargo build
```

### Running the Bot

**Without Redis** (no persistence):
```bash
cargo run
```

**With Redis** (recommended):
```bash
# Start Redis
docker-compose up -d

# Run bot
cargo run

# Stop Redis when done
docker-compose down
```

The bot will:
- Poll prices every 5 minutes (configurable in `src/main.rs:40`)
- Need 288 snapshots for signal generation (24h lookback with 5min polling)
- Save snapshots to Redis automatically
- Load historical data on restart (no warmup period!)

## Testing

### Unit Tests (No External Dependencies)

Run all unit tests that don't require external services:

```bash
cargo test
```

### Integration Tests (Requires Redis)

**Start Redis first:**
```bash
docker-compose up -d
```

**Run persistence unit tests:**
```bash
cargo test persistence --lib -- --ignored --nocapture
```

**Run E2E tests:**
```bash
# Persistence workflow test
cargo test test_e2e_persistence_workflow --test e2e_test -- --ignored --nocapture

# Full bot simulation
cargo test test_e2e_full_bot_simulation --test e2e_test -- --ignored --nocapture

# All E2E tests
cargo test --test e2e_test -- --ignored --nocapture
```

### Run All Tests (Without Redis)

```bash
# Run all tests except those marked #[ignore]
cargo test
```

### Run All Tests (With Redis)

```bash
# Ensure Redis is running
docker-compose up -d

# Run ALL tests including ignored ones
cargo test -- --ignored --nocapture
```

### Specific Module Tests

```bash
# Test indicators
cargo test indicators --lib

# Test strategy
cargo test strategy --lib

# Test risk management
cargo test risk --lib

# Test candle buffer
cargo test candle_buffer --lib
```

## Configuration

### Environment Variables

```bash
# Redis URL (optional, defaults to localhost)
export REDIS_URL="redis://127.0.0.1:6379"
```

### Polling Interval

Edit `src/main.rs`:
```rust
let poll_interval_minutes = 5; // Testing: 5 min, Production: 30 min
```

### Strategy Parameters

Edit `src/strategy/signals.rs`:
```rust
pub struct SignalConfig {
    pub rsi_period: usize,          // Default: 14
    pub rsi_oversold: f64,          // Default: 30.0
    pub rsi_overbought: f64,        // Default: 70.0
    pub short_ma_period: usize,     // Default: 10
    pub long_ma_period: usize,      // Default: 20
    pub volume_threshold: f64,      // Default: 1.5x average
    pub lookback_hours: u64,        // Default: 24 hours
}
```

## Architecture

```
src/
├── api/                  # External API clients
│   ├── dexscreener.rs   # Price data from DexScreener
│   └── jupiter.rs       # Jupiter DEX integration
├── models/              # Core data types (Token, Candle, Signal)
├── indicators/          # Technical indicators (RSI, MA, etc.)
├── strategy/            # Trading strategies
│   ├── momentum.rs      # Momentum-based strategy
│   └── signals.rs       # Signal generation logic
├── execution/           # Price feed and candle management
│   ├── price_feed.rs    # Price fetching and tracking
│   └── candle_buffer.rs # In-memory rolling window buffer
├── persistence/         # Redis persistence layer
├── risk/                # Circuit breakers and risk management
├── db/                  # Database layer (future)
└── llm/                 # LLM integration (future)
```

## Development Workflow

This project follows a structured development process (see `CLAUDE.md`):

1. **Planning**: Analyze requirements and design approach
2. **Critique**: Review plan for issues and edge cases
3. **Implementation**: Write tests first (TDD), then implement
4. **Critique**: Review implementation for correctness and quality

### Running with Debug Logs

```bash
RUST_LOG=debug cargo run
```

### Formatting and Linting

```bash
# Format code
cargo fmt

# Run linter
cargo clippy

# Check without building
cargo check
```

## Redis Data Structure

The bot uses Redis sorted sets for efficient time-series storage:

```
Key: snapshots:{token}
Score: Unix timestamp
Value: {"price": 197.12, "volume": 497553440.82, "timestamp": "2025-10-14T15:40:02Z"}
```

### Inspecting Redis Data

```bash
# Connect to Redis
docker exec -it cryptobot-redis redis-cli

# List all snapshot keys
KEYS snapshots:*

# Count snapshots for a token
ZCARD snapshots:SOL

# View snapshots
ZRANGE snapshots:SOL 0 -1 WITHSCORES

# Get recent snapshots (last 10)
ZRANGE snapshots:SOL -10 -1
```

## Testing Strategy

### Unit Tests
- Fast, isolated tests for individual components
- No external dependencies (mocked)
- Run automatically in CI/CD

### Integration Tests (marked `#[ignore]`)
- Test Redis persistence with real Redis instance
- Test API integrations with live endpoints
- Require external services to be running

### E2E Tests (marked `#[ignore]`)
- Full bot simulation with all components
- Test complete workflows (fetch → save → restart → load)
- Verify data integrity across bot restarts

## Troubleshooting

### "Failed to connect to Redis"

**Problem**: Bot can't connect to Redis

**Solutions**:
```bash
# Check if Redis is running
docker-compose ps

# Start Redis
docker-compose up -d

# Check Redis logs
docker-compose logs redis

# Test Redis connection manually
docker exec cryptobot-redis redis-cli ping
# Should return: PONG
```

### "Not enough data for signal"

**Problem**: Bot needs more historical snapshots

**Why**: Strategy requires 288 snapshots (24h × 12 per hour with 5min polling)

**Solutions**:
- Wait for more data to accumulate
- Lower lookback period in `SignalConfig`
- Pre-populate Redis with historical data

### Test Failures

**Problem**: Tests fail with "connection refused"

**Solution**: Ensure Redis is running for ignored tests:
```bash
docker-compose up -d
cargo test -- --ignored
```

## Next Steps

See [Next Steps](#next-steps-for-development) section below.

## Project Status

**Current Capabilities:**
- ✅ Real-time price tracking (SOL, JUP)
- ✅ Technical indicator calculation
- ✅ Signal generation (Buy/Sell/Hold)
- ✅ Redis persistence with auto-recovery
- ✅ Circuit breakers and risk management
- ✅ Comprehensive test coverage

**Architecture:**
- 🎯 **MVP**: Single user (private key in env var)
- 📋 **Future**: Multi-user with encrypted wallets (see `docs/MULTI_USER_ARCHITECTURE.md`)

**In Development:**
- 🚧 Executor layer (portfolio-aware decisions)
- 🚧 Position management
- 🚧 Wallet integration (Solana transactions)

## Contributing

1. Follow the 4-phase development workflow (see `CLAUDE.md`)
2. Write tests first (TDD)
3. Run tests before committing: `cargo test`
4. Format code: `cargo fmt`
5. Check for issues: `cargo clippy`

## License

[Your License Here]

---

## Next Steps for Development

Based on the current architecture and `docs/PLANNING.md`, here are the recommended next steps:

### 1. Executor Layer (High Priority)
**Why**: Decouple signal generation from execution decisions

**Tasks**:
- Create `src/execution/executor.rs`
- Implement portfolio-aware decision making:
  - Check current positions before executing signals
  - Respect position size limits (max 5% per position)
  - Avoid duplicate positions in same token
  - Handle "already sold" scenarios
- Add position sizing logic:
  - Calculate appropriate quantities based on portfolio value
  - Respect circuit breaker limits
- Tests:
  - Test signal → execution translation
  - Test portfolio state checking
  - Test position sizing calculations

**Example**:
```rust
// Signal says "Buy SOL" but we already have SOL
// Executor decides: Skip (already positioned)

// Signal says "Sell JUP" but we don't own JUP
// Executor decides: Skip (nothing to sell)
```

### 2. Position Management (High Priority)
**Why**: Track open positions, calculate P&L, manage exits

**Tasks**:
- Create `src/execution/position_manager.rs`
- Track open positions in memory + database
- Implement exit logic:
  - Stop loss: -8% from entry
  - Take profit: Trailing stop (activates at +12%, trails by 5%)
  - Time stop: Force exit after 14 days
- Calculate real-time P&L
- Tests:
  - Test position creation and tracking
  - Test exit triggers (stop loss, take profit, time stop)
  - Test P&L calculations

### 3. Database Integration (Medium Priority)
**Why**: Persist trades, positions, and performance history

**Tasks**:
- Implement PostgreSQL schema (see `src/db/mod.rs`)
- Tables needed:
  - `trades` (id, token, entry_time, entry_price, exit_time, exit_price, pnl)
  - `positions` (id, token, status, entry_price, quantity, stop_loss, take_profit)
  - `signals` (id, token, signal_type, timestamp, indicators)
- Add trade logging on entry/exit
- Add performance analytics queries
- Tests:
  - Test trade persistence
  - Test position queries
  - Test analytics calculations

### 4. LLM Integration - Strategic Layer (Medium Priority)
**Why**: Add intelligent watchlist curation and sentiment analysis

**Tasks**:
- Create `src/llm/openai.rs` or similar
- Implement watchlist curation:
  - Analyze top tokens by volume
  - Filter out meme coins, scams
  - Suggest promising tokens based on fundamentals
- Run periodically (every 30-60 minutes)
- Update tracked tokens list dynamically
- Tests:
  - Test LLM API integration
  - Test response parsing
  - Mock LLM for CI/CD tests

### 5. Enhanced Risk Management (Medium Priority)
**Why**: Prevent catastrophic losses

**Tasks**:
- Add drawdown tracking
- Implement max consecutive losses check
- Add daily trade limit
- Add correlation analysis (don't over-expose to correlated assets)
- Tests:
  - Test each circuit breaker trigger
  - Test recovery after breaker trips
  - Test with historical loss scenarios

### 6. Backtesting Framework (Low Priority)
**Why**: Validate strategy performance before live trading

**Tasks**:
- Create `src/backtest/mod.rs`
- Load historical price data
- Simulate strategy execution
- Calculate metrics:
  - Win rate
  - Average P&L per trade
  - Max drawdown
  - Sharpe ratio
- Generate performance reports
- Tests:
  - Test backtest engine accuracy
  - Verify metrics calculations

### 7. Paper Trading Mode (Low Priority)
**Why**: Test with live data without risking capital

**Tasks**:
- Add `--paper-trading` flag
- Track virtual positions and P&L
- Log trades to database as "paper"
- Generate performance reports
- Switch to real trading after validation

### 8. Monitoring & Alerts (Low Priority)
**Why**: Stay informed of bot status and important events

**Tasks**:
- Add Prometheus metrics export
- Create Grafana dashboard
- Implement alerts:
  - Circuit breaker triggered
  - API errors
  - Large P&L swings
  - Position exits
- Integrate with Discord/Telegram/Email

### Recommended Order:

**Phase 0 - MVP (Current Focus - 2-3 weeks)**:
1. Executor Layer (single-user)
2. Position Management (single-user)
3. Wallet integration (private key from env var)
4. Test with real money (small amounts)

**Phase 1 - Multi-User (4-6 weeks after MVP)**:
5. Database Integration
6. Wallet encryption for multiple users
7. CLI tools for user management
8. See `docs/MULTI_USER_ARCHITECTURE.md` for details

**Phase 2 - Intelligence (1-2 weeks)**:
9. LLM Integration
10. Enhanced Risk Management

**Phase 3 - Validation (1-2 weeks)**:
11. Backtesting Framework
12. Paper Trading Mode

**Phase 4 - Production (1 week)**:
13. Monitoring & Alerts
14. Deploy to production environment

### Current Blockers:
- None! Redis persistence is complete and tested
- Ready to start Executor Layer immediately

### Current Focus:
- **MVP**: Single user with private key in environment variable
- **Goal**: Get one user (me) trading successfully
- **Next**: Build executor layer for portfolio-aware decisions

### Resources Needed (Now):
- Solana wallet with private key
- Small amount of SOL for testing

### Resources Needed (Later):
- Database: PostgreSQL (already in docker-compose.yml)
- LLM: OpenAI API key or local Ollama setup (Phase 2)
- Metrics: Prometheus + Grafana (optional, Phase 4)
