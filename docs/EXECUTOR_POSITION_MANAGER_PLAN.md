# Executor Layer & Position Manager - Implementation Plan

**Date**: 2025-10-14
**Status**: 📋 Planning Phase
**Goal**: Build portfolio-aware execution and position management for single-user MVP

---

## Phase 1: Planning

### Overview

We need to build two interconnected components:

1. **Position Manager**: Tracks open positions, calculates P&L, manages exits
2. **Executor**: Receives signals, checks portfolio state, decides whether to execute

**Key Principle**: Signals are pure (no portfolio awareness) → Executor is smart (portfolio-aware)

### Architecture Diagram

```
Price Feed → Strategy → Signal → Executor → Position Manager → Wallet
                                    ↓              ↓
                                Portfolio State   Trade Log
```

**Flow**:
1. Strategy generates signal: "Buy SOL"
2. Executor checks: Do we already have SOL? Portfolio state? Risk limits?
3. Executor decides: Execute, skip, or adjust size
4. Position Manager: Creates position, tracks it, monitors exits
5. Wallet: Signs and sends transaction

---

## Component 1: Position Manager

### Responsibilities

1. **Track Positions**: Maintain in-memory list of open positions
2. **Calculate P&L**: Real-time profit/loss based on current price
3. **Monitor Exits**: Check stop loss, take profit, time stop
4. **Update State**: Keep TradingState current (for circuit breakers)

### Data Model

```rust
// src/execution/position_manager.rs

pub struct Position {
    pub id: Uuid,
    pub token: String,
    pub entry_price: f64,
    pub quantity: f64,
    pub entry_time: DateTime<Utc>,
    pub stop_loss: f64,          // -8% from entry
    pub take_profit: Option<f64>, // Trailing stop
    pub status: PositionStatus,
    pub realized_pnl: Option<f64>,
    pub exit_price: Option<f64>,
    pub exit_time: Option<DateTime<Utc>>,
    pub exit_reason: Option<ExitReason>,
}

pub enum PositionStatus {
    Open,
    Closed,
}

pub enum ExitReason {
    StopLoss,
    TakeProfit,
    TimeStop,
    Manual,
}

pub struct PositionManager {
    positions: Vec<Position>,
    circuit_breakers: CircuitBreakers,
    trading_state: TradingState,
}
```

### Key Methods

```rust
impl PositionManager {
    /// Create new position
    pub fn open_position(
        &mut self,
        token: String,
        entry_price: f64,
        quantity: f64,
    ) -> Result<Uuid>;

    /// Check if we have open position for token
    pub fn has_open_position(&self, token: &str) -> bool;

    /// Get open position for token
    pub fn get_open_position(&self, token: &str) -> Option<&Position>;

    /// Calculate current P&L for position
    pub fn calculate_pnl(&self, position_id: Uuid, current_price: f64) -> Result<f64>;

    /// Check if position should exit (returns exit reason if yes)
    pub fn should_exit(
        &self,
        position_id: Uuid,
        current_price: f64,
    ) -> Result<Option<ExitReason>>;

    /// Close position
    pub fn close_position(
        &mut self,
        position_id: Uuid,
        exit_price: f64,
        reason: ExitReason,
    ) -> Result<()>;

    /// Check all open positions for exits
    pub fn check_exits(&mut self, prices: &HashMap<String, f64>) -> Result<Vec<Uuid>>;

    /// Get portfolio value
    pub fn portfolio_value(&self, prices: &HashMap<String, f64>) -> Result<f64>;

    /// Get current trading state
    pub fn trading_state(&self) -> &TradingState;
}
```

### Exit Logic

**Stop Loss (-8%)**:
```rust
if current_price <= position.stop_loss {
    // Exit immediately
    return Some(ExitReason::StopLoss);
}
```

**Take Profit (Trailing Stop)**:
```rust
// Activate at +12% gain
let activation_price = position.entry_price * 1.12;

if current_price >= activation_price {
    // Trail by 5% from highest price
    let highest = max(position.entry_price * 1.12, current_price);
    let trailing_stop = highest * 0.95;

    position.take_profit = Some(trailing_stop);
}

if let Some(tp) = position.take_profit {
    if current_price <= tp {
        return Some(ExitReason::TakeProfit);
    }
}
```

**Time Stop (14 days)**:
```rust
let days_open = (Utc::now() - position.entry_time).num_days();
if days_open >= 14 {
    return Some(ExitReason::TimeStop);
}
```

---

## Component 2: Executor

### Responsibilities

1. **Receive Signals**: Get Buy/Sell/Hold from strategy
2. **Check Portfolio**: Query position manager for current state
3. **Validate Execution**: Check circuit breakers, position limits
4. **Size Positions**: Calculate appropriate quantity based on portfolio value
5. **Execute Trades**: Send to wallet layer (future)

### Data Model

```rust
// src/execution/executor.rs

pub struct Executor {
    position_manager: Arc<Mutex<PositionManager>>,
    wallet: Option<Keypair>, // Will be None for now, Some later
}

pub struct ExecutionDecision {
    pub action: ExecutionAction,
    pub reason: String,
}

pub enum ExecutionAction {
    Execute { quantity: f64 },
    Skip,
    Close { position_id: Uuid },
}
```

### Key Methods

```rust
impl Executor {
    /// Process a signal and decide what to do
    pub fn process_signal(
        &mut self,
        signal: &Signal,
        token: &str,
        current_price: f64,
    ) -> Result<ExecutionDecision>;

    /// Calculate position size based on portfolio value and risk limits
    fn calculate_position_size(
        &self,
        current_price: f64,
        max_position_pct: f64,
    ) -> Result<f64>;

    /// Execute buy order
    async fn execute_buy(
        &mut self,
        token: &str,
        quantity: f64,
        price: f64,
    ) -> Result<()>;

    /// Execute sell order
    async fn execute_sell(
        &mut self,
        position_id: Uuid,
        quantity: f64,
        price: f64,
    ) -> Result<()>;
}
```

### Execution Logic

**Signal: Buy**
```rust
// Check 1: Do we already have this token?
if position_manager.has_open_position(token) {
    return ExecutionDecision {
        action: ExecutionAction::Skip,
        reason: "Already have open position".to_string(),
    };
}

// Check 2: Circuit breakers
if let Err(e) = circuit_breakers.check(&trading_state) {
    return ExecutionDecision {
        action: ExecutionAction::Skip,
        reason: format!("Circuit breaker: {}", e),
    };
}

// Check 3: Calculate size (5% of portfolio)
let quantity = calculate_position_size(current_price, 0.05)?;

// Execute
ExecutionDecision {
    action: ExecutionAction::Execute { quantity },
    reason: "Buy signal with available capital".to_string(),
}
```

**Signal: Sell**
```rust
// Check: Do we have this token?
if let Some(position) = position_manager.get_open_position(token) {
    return ExecutionDecision {
        action: ExecutionAction::Close {
            position_id: position.id
        },
        reason: "Sell signal on open position".to_string(),
    };
} else {
    return ExecutionDecision {
        action: ExecutionAction::Skip,
        reason: "No position to sell".to_string(),
    };
}
```

**Signal: Hold**
```rust
// Still check for exits
let exits = position_manager.check_exits(&prices)?;
if !exits.is_empty() {
    // Close positions that hit exits
    for position_id in exits {
        // ...
    }
}

ExecutionDecision {
    action: ExecutionAction::Skip,
    reason: "Hold signal".to_string(),
}
```

---

## Integration with Main Loop

### Current Main Loop (Simplified)
```rust
loop {
    // Fetch prices
    let results = price_manager.fetch_all().await;

    // Generate signals
    for token in &tokens {
        let candles = price_manager.buffer().get_candles(&token.symbol)?;
        let signal = strategy.generate_signal(&candles)?;

        tracing::info!("{}: Signal = {:?}", token.symbol, signal);
    }
}
```

### New Main Loop (With Executor)
```rust
// Initialize
let circuit_breakers = CircuitBreakers::from_env();
let mut position_manager = PositionManager::new(
    initial_portfolio_value,
    circuit_breakers.clone()
);
let mut executor = Executor::new(position_manager.clone());

loop {
    ticker.tick().await;

    // Fetch prices
    let results = price_manager.fetch_all().await;
    let mut prices: HashMap<String, f64> = HashMap::new();

    for (i, result) in results.iter().enumerate() {
        if let Ok(snapshot) = result {
            prices.insert(tokens[i].symbol.clone(), snapshot.price);
        }
    }

    // Check exits FIRST (before generating new signals)
    position_manager.check_exits(&prices)?;

    // Generate and execute signals
    for token in &tokens {
        let candles = price_manager.buffer().get_candles(&token.symbol)?;

        if candles.len() >= samples_needed {
            let signal = strategy.generate_signal(&candles)?;
            let current_price = prices.get(&token.symbol).unwrap();

            // Process signal with executor
            let decision = executor.process_signal(
                &signal,
                &token.symbol,
                *current_price,
            )?;

            tracing::info!(
                "{}: Signal={:?}, Action={:?}, Reason={}",
                token.symbol,
                signal,
                decision.action,
                decision.reason
            );

            // Execute if needed (will be async later with real wallet)
            match decision.action {
                ExecutionAction::Execute { quantity } => {
                    // For now: just log, later: send transaction
                    tracing::info!(
                        "Would buy {} {} @ ${:.4} (qty: {:.4})",
                        quantity,
                        token.symbol,
                        current_price,
                        quantity
                    );
                }
                ExecutionAction::Close { position_id } => {
                    // For now: just log, later: send transaction
                    tracing::info!(
                        "Would sell position {} @ ${:.4}",
                        position_id,
                        current_price
                    );
                }
                ExecutionAction::Skip => {
                    // Do nothing
                }
            }
        }
    }

    // Log portfolio state
    let portfolio_value = position_manager.portfolio_value(&prices)?;
    tracing::info!(
        "Portfolio: ${:.2}, Open Positions: {}",
        portfolio_value,
        position_manager.positions.len()
    );
}
```

---

## Testing Strategy

### Position Manager Tests

**Unit Tests**:
```rust
#[test]
fn test_open_position() {
    let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
    let id = pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();

    assert!(pm.has_open_position("SOL"));
    assert_eq!(pm.positions.len(), 1);
}

#[test]
fn test_stop_loss_triggered() {
    let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
    let id = pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();

    // Price drops 9% - should trigger stop loss (-8%)
    let reason = pm.should_exit(id, 91.0).unwrap();
    assert_eq!(reason, Some(ExitReason::StopLoss));
}

#[test]
fn test_take_profit_trailing() {
    let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
    let id = pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();

    // Price goes up 15% - activates trailing stop
    pm.update_trailing_stop(id, 115.0).unwrap();

    // Price drops 6% from peak - should trigger (trails by 5%)
    let reason = pm.should_exit(id, 108.0).unwrap();
    assert_eq!(reason, Some(ExitReason::TakeProfit));
}

#[test]
fn test_time_stop() {
    let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());

    // Create position 15 days ago
    let mut position = Position {
        entry_time: Utc::now() - chrono::Duration::days(15),
        // ...
    };

    let reason = pm.should_exit(position.id, 105.0).unwrap();
    assert_eq!(reason, Some(ExitReason::TimeStop));
}

#[test]
fn test_pnl_calculation() {
    let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
    let id = pm.open_position("SOL".to_string(), 100.0, 2.0).unwrap();

    // Price went from 100 to 110, bought 2 tokens
    let pnl = pm.calculate_pnl(id, 110.0).unwrap();
    assert_eq!(pnl, 20.0); // 2 * (110 - 100)
}

#[test]
fn test_prevent_duplicate_positions() {
    let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
    pm.open_position("SOL".to_string(), 100.0, 1.0).unwrap();

    let result = pm.open_position("SOL".to_string(), 105.0, 1.0);
    assert!(result.is_err());
}
```

### Executor Tests

**Unit Tests**:
```rust
#[test]
fn test_skip_buy_when_already_positioned() {
    let pm = Arc::new(Mutex::new(PositionManager::new(10000.0, CircuitBreakers::default())));
    pm.lock().unwrap().open_position("SOL".to_string(), 100.0, 1.0).unwrap();

    let mut executor = Executor::new(pm);
    let decision = executor.process_signal(&Signal::Buy, "SOL", 105.0).unwrap();

    assert!(matches!(decision.action, ExecutionAction::Skip));
    assert!(decision.reason.contains("Already have"));
}

#[test]
fn test_skip_sell_when_no_position() {
    let pm = Arc::new(Mutex::new(PositionManager::new(10000.0, CircuitBreakers::default())));
    let mut executor = Executor::new(pm);

    let decision = executor.process_signal(&Signal::Sell, "SOL", 100.0).unwrap();

    assert!(matches!(decision.action, ExecutionAction::Skip));
    assert!(decision.reason.contains("No position"));
}

#[test]
fn test_execute_buy_when_valid() {
    let pm = Arc::new(Mutex::new(PositionManager::new(10000.0, CircuitBreakers::default())));
    let mut executor = Executor::new(pm);

    let decision = executor.process_signal(&Signal::Buy, "SOL", 100.0).unwrap();

    assert!(matches!(decision.action, ExecutionAction::Execute { .. }));
}

#[test]
fn test_position_sizing() {
    let pm = Arc::new(Mutex::new(PositionManager::new(10000.0, CircuitBreakers::default())));
    let executor = Executor::new(pm);

    // Portfolio = 10000, max position = 5% = 500
    // Price = 100, so quantity = 500 / 100 = 5
    let quantity = executor.calculate_position_size(100.0, 0.05).unwrap();
    assert_eq!(quantity, 5.0);
}

#[test]
fn test_circuit_breaker_blocks_execution() {
    let mut breakers = CircuitBreakers::default();
    breakers.max_daily_loss_pct = 5.0;

    let pm = Arc::new(Mutex::new(PositionManager::new(10000.0, breakers)));

    // Trigger circuit breaker
    pm.lock().unwrap().trading_state.daily_pnl = -600.0; // -6%

    let mut executor = Executor::new(pm);
    let decision = executor.process_signal(&Signal::Buy, "SOL", 100.0).unwrap();

    assert!(matches!(decision.action, ExecutionAction::Skip));
    assert!(decision.reason.contains("Circuit breaker"));
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_full_trade_cycle() {
    // Initialize
    let mut pm = PositionManager::new(10000.0, CircuitBreakers::default());
    let mut executor = Executor::new(Arc::new(Mutex::new(pm)));

    // Buy signal - should execute
    let decision = executor.process_signal(&Signal::Buy, "SOL", 100.0).unwrap();
    assert!(matches!(decision.action, ExecutionAction::Execute { .. }));

    // Create position
    pm.open_position("SOL".to_string(), 100.0, 5.0).unwrap();

    // Buy signal again - should skip (already have position)
    let decision = executor.process_signal(&Signal::Buy, "SOL", 105.0).unwrap();
    assert!(matches!(decision.action, ExecutionAction::Skip));

    // Sell signal - should close position
    let decision = executor.process_signal(&Signal::Sell, "SOL", 110.0).unwrap();
    assert!(matches!(decision.action, ExecutionAction::Close { .. }));

    // Sell signal again - should skip (no position)
    let decision = executor.process_signal(&Signal::Sell, "SOL", 110.0).unwrap();
    assert!(matches!(decision.action, ExecutionAction::Skip));
}
```

---

## File Structure

```
src/execution/
├── mod.rs                   # Re-exports
├── price_feed.rs           # Existing - fetch prices
├── candle_buffer.rs        # Existing - store snapshots
├── position_manager.rs     # NEW - track positions
└── executor.rs             # NEW - execute signals
```

---

## Implementation Order

1. **Position Manager** (Core data structure)
   - Write tests for `open_position`, `close_position`
   - Implement `has_open_position`, `get_open_position`
   - Write tests for P&L calculation
   - Implement `calculate_pnl`
   - Write tests for exit conditions
   - Implement `should_exit` with all exit logic
   - Write tests for `check_exits`
   - Implement `check_exits`

2. **Executor** (Decision making)
   - Write tests for signal processing
   - Implement `process_signal` logic
   - Write tests for position sizing
   - Implement `calculate_position_size`
   - Write tests for circuit breaker checks
   - Integrate with position manager

3. **Main Loop Integration**
   - Update `main.rs` to use executor
   - Add portfolio logging
   - Test with simulated signals

4. **Wallet Integration** (Later - After testing)
   - Load private key from env
   - Implement actual transaction sending
   - Add Solana client

---

## Key Design Decisions

### 1. Why In-Memory Position Storage?
For MVP, we don't need database persistence of positions. Benefits:
- Simpler implementation
- Faster iteration
- Easy to add DB later without changing interface

**When to add DB**: When we need position history for backtesting or multi-user support

### 2. Why Arc<Mutex<PositionManager>>?
Need shared mutable access between executor and main loop:
- Executor reads positions to make decisions
- Main loop updates positions on exits
- Mutex ensures thread-safety

### 3. Why Separate Executor and PositionManager?
**Separation of concerns**:
- PositionManager: "What positions do we have?"
- Executor: "Should we act on this signal?"

Makes testing easier and code more modular.

### 4. Wallet Integration: Now or Later?
**Later**. For initial testing:
- Log "Would buy" / "Would sell"
- Manually verify logic is correct
- Add real transactions after validation

Avoids losing money on bugs!

---

## Environment Variables Needed

```bash
# .env
WALLET_PRIVATE_KEY=<base58>  # For actual trading (later)
INITIAL_PORTFOLIO_VALUE=10000.0
MAX_POSITION_SIZE_PCT=5.0
MAX_DAILY_LOSS_PCT=5.0
MAX_DRAWDOWN_PCT=20.0
MAX_CONSECUTIVE_LOSSES=5
```

---

## Success Criteria

After implementation, we should be able to:

1. ✅ Process Buy signal when no position → Execute
2. ✅ Process Buy signal when already positioned → Skip
3. ✅ Process Sell signal when positioned → Close
4. ✅ Process Sell signal when not positioned → Skip
5. ✅ Calculate correct position size (5% of portfolio)
6. ✅ Detect stop loss triggers (-8%)
7. ✅ Detect take profit triggers (trailing stop)
8. ✅ Detect time stop triggers (14 days)
9. ✅ Respect circuit breakers
10. ✅ Track portfolio value accurately

---

## Open Questions for Critique

1. **Should position manager track cash balance separately?**
   - Currently: portfolio_value = cash + position values
   - Alternative: Track cash explicitly

2. **How to handle partial fills?**
   - Currently: Assume full fills
   - Later: Track pending orders?

3. **Should we support multiple positions in same token?**
   - Currently: One position per token
   - Alternative: Allow multiple (with different entry prices)

4. **Trailing stop: Update on every tick or only on new highs?**
   - Currently: Only on new highs
   - Alternative: Update continuously (more compute)

5. **What happens if signal comes before enough data?**
   - Currently: Skip
   - Alternative: Queue for later?

---

## Next Step: Critique This Plan

Ready for critique phase! Questions to address:
- Are there edge cases we're missing?
- Is the data model correct?
- Are the exit conditions right?
- Is the test coverage sufficient?
- Should we change the implementation order?
