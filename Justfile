# Justfile for cryptobot
# Install just: brew install just
# Run: just <command>

# Default recipe - show available commands
default:
    @just --list

# Run all unit tests
test:
    cargo test --lib

# Run unit tests with output
test-verbose:
    cargo test --lib -- --nocapture

# Run backtest tests specifically
test-backtest:
    cargo test --lib backtest

# Run Postgres integration tests (requires Postgres running)
test-postgres:
    DATABASE_URL="postgres://cryptobot:cryptobot_dev_password@localhost:5432/cryptobot" \
    cargo test --lib db::postgres -- --ignored --test-threads=1

# Run Redis integration tests (requires Redis running)
test-redis:
    REDIS_URL="redis://127.0.0.1:6379" \
    cargo test --lib persistence -- --ignored --test-threads=1

# Run all integration tests (requires Postgres + Redis)
test-integration: test-postgres test-redis

# Run doc tests
test-doc:
    cargo test --doc

# Run all tests including integration
test-all: test test-integration test-doc

# Check code formatting
fmt-check:
    cargo fmt --all -- --check

# Format code
fmt:
    cargo fmt --all

# Run clippy lints (warnings only)
clippy:
    cargo clippy --all-targets --all-features

# Run clippy lints (strict - warnings as errors)
clippy-strict:
    cargo clippy --all-targets --all-features -- -D warnings

# Run clippy with auto-fixes
clippy-fix:
    cargo clippy --all-targets --all-features --fix --allow-dirty

# Build the project
build:
    cargo build

# Build release binary
build-release:
    cargo build --release

# Clean build artifacts
clean:
    cargo clean

# Run the bot locally
run:
    cargo run

# Run the bot with debug logging
run-debug:
    RUST_LOG=debug cargo run

# Run backtest CLI
backtest:
    cargo run --bin backtest

# Pre-commit checks (fast - run before committing)
pre-commit: fmt clippy test test-doc
    @echo "✅ All pre-commit checks passed!"

# Pre-push checks (includes integration tests - requires services running)
pre-push: fmt clippy test-all
    @echo "✅ All pre-push checks passed!"

# Check Railway deployment logs
logs:
    railway logs --service solbot

# Quick check (format + clippy only)
check: fmt clippy
    @echo "✅ Quick check passed!"
