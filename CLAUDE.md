# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

CryptoBot is a Solana-based cryptocurrency trading bot using swing trading strategies (1-7 day holds). It combines LLM analysis for strategic decisions with fast algorithmic execution for tactical trading.

## Commands

### Build and Test
```bash
# Build the project
cargo build

# Run the bot (connects to real APIs, polls every 5 min)
cargo run

# Run all tests
cargo test

# Run tests including ignored (API integration) tests
cargo test -- --ignored

# Run a specific test
cargo test test_rsi_calculation

# Run with logging output
RUST_LOG=debug cargo run

# Run specific module tests
cargo test candle_buffer --lib
cargo test price_feed --lib
cargo test strategy --lib
```

### Development
```bash
# Check code without building
cargo check

# Format code
cargo fmt

# Run linter
cargo clippy

# Watch and rebuild on changes (requires cargo-watch)
cargo watch -x test
```

## Architecture

### Module Structure
```
src/
├── api/                  # External API clients
│   ├── dexscreener.rs   # DexScreener price data
│   └── jupiter.rs       # Jupiter DEX aggregator
├── models/              # Core data types
├── indicators/          # Technical indicators (RSI, MA, etc.)
├── strategy/            # Trading strategies
├── execution/           # Order execution
├── risk/                # Risk management & circuit breakers
├── db/                  # Database layer
└── llm/                 # LLM integration
```

### Key Components

**Data Sources**:
- DexScreener: Historical & current price data (free)
- Jupiter: Real-time quotes & swap execution
- Reddit API: Social sentiment (future)

**Trading Logic**:
- LLM (Strategic): Watchlist curation, sentiment analysis, risk parameters (runs every 30-60 min)
- Fast Algorithms (Tactical): Technical signals, trade execution, position management (real-time)

**Exit Strategy**:
- Stop loss: -8% fixed from entry
- Take profit: Trailing stop (activates at +12%, trails by 5%)
- Time stop: Force exit after 14 days

**Circuit Breakers**:
- Max daily loss: -5%
- Max drawdown: -20% from peak
- Max consecutive losses: 5
- Max position size: 5%
- Max daily trades: 10

## Development Workflow

This repository follows a structured four-phase development process for all features and changes:

### 1. Planning Phase
- Analyze requirements and constraints
- Break down the task into concrete steps
- Identify dependencies and integration points
- Document the approach and architectural decisions
- Consider edge cases and error handling upfront

### 2. Critique Phase (Plan Review)
- Review the plan for completeness and correctness
- Identify potential issues, risks, or missed requirements
- Evaluate alternative approaches
- Validate assumptions
- Refine the plan based on critique findings

### 3. Implementation Phase (TDD)
- Follow Test-Driven Development methodology:
  - Write failing tests first
  - Implement minimal code to pass tests
  - Refactor while keeping tests green
- Write tests before implementation code
- Ensure comprehensive test coverage
- Run tests frequently during development

### 4. Critique Phase (Implementation Review)
- Review the implementation for correctness
- Verify test coverage is adequate
- Check for potential bugs or edge cases
- Evaluate code quality and maintainability
- Ensure the implementation matches the original requirements
- Validate error handling and edge cases

## Process Notes

- Always complete all four phases before considering a task done
- Document decisions and rationale during planning
- Be thorough in critique phases - challenge assumptions
- In TDD, resist the urge to implement before writing tests
- Each phase builds on the previous - don't skip ahead

## Testing Conventions

- Unit tests in same file as implementation
- Integration tests marked with `#[ignore]` to avoid hitting APIs
- Use `mockito` for mocking external services
- Property-based testing with `proptest` for critical calculations

## Code Cleanup and Simplification

Maintain minimal complexity and clean code by following these guidelines:

### Regular Cleanup Checks
```bash
# Check for compiler warnings
cargo build 2>&1 | grep warning

# Run clippy for code quality
cargo clippy --all-targets

# Verify all tests pass
cargo test
```

### Handling Unused Code

**Remove immediately**:
- Unused imports
- Unused functions that are clearly obsolete
- Dead code paths that will never be used

**Annotate if necessary**:
- API response fields: Use `#[allow(dead_code)]` for fields required by serde but not used in code
- Test helpers: Prefix with `_` if intentionally unused (e.g., `_id2`)
- Future features: Add TODO comments explaining why code is kept

### Documentation Management

**Keep in main `docs/` folder**:
- `ARCHITECTURE.md` - Core system design
- `DEPLOYMENT.md` - Deployment guide
- `MULTI_USER_ARCHITECTURE.md` - Future architecture plans

**Archive to `docs/archive/`**:
- Completed planning documents
- Historical critiques and session summaries
- Superseded documentation

### Testing Requirements

**Every new function must have tests**:
- Happy path tests
- Edge case tests (empty input, single item, boundaries)
- Error condition tests
- Integration points

**Example**: When adding `validate_candle_uniformity()`, add tests for:
- Uniform data (should pass)
- Data with gaps (should fail)
- Single candle (edge case)
- Backwards timestamps (should fail)
- Tolerance boundaries

### Cleanup Checklist

Before considering any feature complete:
- [ ] No compiler warnings about unused code
- [ ] All new functions have comprehensive tests
- [ ] No commented-out code
- [ ] Documentation is up-to-date or archived
- [ ] `cargo clippy` shows no new warnings
- [ ] All tests pass: `cargo test`
