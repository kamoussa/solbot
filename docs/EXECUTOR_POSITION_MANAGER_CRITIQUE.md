# Executor & Position Manager Plan - Critique

**Date**: 2025-10-14
**Status**: 🔍 Critique Phase
**Reviewer**: Self-review following CLAUDE.md workflow

---

## Critical Issues Found

### 🚨 Issue 1: Position Manager State Inconsistency

**Problem**: Position Manager owns both `positions` and `trading_state`, but they can get out of sync.

```rust
pub struct PositionManager {
    positions: Vec<Position>,
    trading_state: TradingState,  // ⚠️ Can diverge from positions!
}
```

**Example Bug**:
```rust
// Position closes with +$50 profit
position_manager.close_position(id, 150.0, ExitReason::TakeProfit)?;

// But if we forget to update trading_state.daily_pnl...
// Circuit breakers will have wrong information!
```

**Solution**: Make P&L updates atomic
```rust
pub fn close_position(
    &mut self,
    position_id: Uuid,
    exit_price: f64,
    reason: ExitReason,
) -> Result<f64> {  // Return realized P&L
    // Calculate P&L
    let pnl = self.calculate_pnl(position_id, exit_price)?;

    // Update position
    position.status = PositionStatus::Closed;
    position.realized_pnl = Some(pnl);

    // ✅ ATOMICALLY update trading state
    self.trading_state.daily_pnl += pnl;
    self.trading_state.total_trades += 1;

    if pnl < 0.0 {
        self.trading_state.consecutive_losses += 1;
    } else {
        self.trading_state.consecutive_losses = 0;
    }

    Ok(pnl)
}
```

---

### 🚨 Issue 2: Portfolio Value Calculation is Wrong

**Problem**: Plan doesn't account for cash vs positions

```rust
// Current plan
pub fn portfolio_value(&self, prices: &HashMap<String, f64>) -> Result<f64> {
    // ⚠️ What about cash balance?
    let position_value = self.positions.iter()
        .map(|p| p.quantity * prices.get(&p.token))
        .sum();

    // Missing: + cash_balance
}
```

**Reality**:
- Start: $10,000 cash
- Buy 5 SOL @ $100 = $500 in positions, $9,500 cash
- Portfolio = $500 + $9,500 = $10,000

**Solution**: Track cash explicitly
```rust
pub struct PositionManager {
    positions: Vec<Position>,
    cash_balance: f64,  // ✅ Add this
    initial_balance: f64,
    trading_state: TradingState,
}

pub fn portfolio_value(&self, prices: &HashMap<String, f64>) -> Result<f64> {
    let position_value: f64 = self.positions.iter()
        .filter(|p| p.status == PositionStatus::Open)
        .map(|p| {
            let price = prices.get(&p.token).unwrap_or(&0.0);
            p.quantity * price
        })
        .sum();

    Ok(self.cash_balance + position_value)
}

pub fn available_cash(&self) -> f64 {
    self.cash_balance
}
```

---

### 🚨 Issue 3: Position Sizing Uses Total Portfolio, Not Available Cash

**Problem**: Executor sizes positions based on total portfolio value

```rust
// Current plan
fn calculate_position_size(&self, price: f64, max_pct: f64) -> Result<f64> {
    let portfolio_value = self.position_manager.portfolio_value()?;
    let max_position_value = portfolio_value * max_pct;
    Ok(max_position_value / price)
}
```

**Bug**:
- Portfolio = $10,000
- Already have $5,000 in positions (3 tokens @ 5% each = 15%)
- Try to buy 4th token: 5% of $10,000 = $500
- **But we only have $5,000 cash! Can't buy 5 more tokens** ❌

**Solution**: Use available cash, not total portfolio
```rust
fn calculate_position_size(
    &self,
    price: f64,
    max_position_pct: f64,
) -> Result<f64> {
    let pm = self.position_manager.lock().unwrap();

    // Use AVAILABLE CASH, not total portfolio
    let available = pm.available_cash();
    let desired_position_value = pm.initial_balance * max_position_pct;

    // Take minimum of: what we want vs what we can afford
    let position_value = desired_position_value.min(available);

    Ok(position_value / price)
}
```

---

### 🚨 Issue 4: Trailing Stop Logic is Incomplete

**Problem**: Plan doesn't show how to track "highest price seen"

```rust
// Current plan - incomplete
if current_price >= activation_price {
    let highest = max(position.entry_price * 1.12, current_price);  // ⚠️ Wrong!
    let trailing_stop = highest * 0.95;
}
```

**Bug**: `highest` should be tracked over time, not calculated on each tick

**Solution**: Add `highest_price` to Position
```rust
pub struct Position {
    // ...existing fields...
    pub highest_price_seen: f64,  // ✅ Track this
    pub trailing_stop_active: bool,  // ✅ Track activation
}

// In should_exit()
// Update highest price seen
if current_price > position.highest_price_seen {
    position.highest_price_seen = current_price;
}

// Check if trailing stop should activate
let activation_price = position.entry_price * 1.12;  // +12%
if !position.trailing_stop_active && position.highest_price_seen >= activation_price {
    position.trailing_stop_active = true;
}

// If active, check if we should exit
if position.trailing_stop_active {
    let trailing_stop = position.highest_price_seen * 0.95;  // 5% from peak
    if current_price <= trailing_stop {
        return Some(ExitReason::TakeProfit);
    }
}
```

---

### ⚠️ Issue 5: Concurrent Access Pattern is Wrong

**Problem**: Using `Arc<Mutex<PositionManager>>` in executor

```rust
pub struct Executor {
    position_manager: Arc<Mutex<PositionManager>>,  // ⚠️ Awkward
}
```

**Issues**:
1. Every operation needs `.lock().unwrap()` (ugly code)
2. Mutex can deadlock if we're not careful
3. Over-complicates single-threaded main loop

**Better Solution**: Pass references, don't share ownership
```rust
pub struct Executor {
    // No ownership of position manager!
}

impl Executor {
    pub fn process_signal(
        &self,
        signal: &Signal,
        token: &str,
        current_price: f64,
        position_manager: &PositionManager,  // ✅ Borrow
        circuit_breakers: &CircuitBreakers,
    ) -> Result<ExecutionDecision> {
        // Use position_manager directly, no locks needed
    }
}
```

Main loop stays simple:
```rust
let mut position_manager = PositionManager::new(...);
let executor = Executor::new();

loop {
    // Check exits
    position_manager.check_exits(&prices)?;

    // Process signals - just borrow
    let decision = executor.process_signal(
        &signal,
        token,
        price,
        &position_manager,  // ✅ Simple borrow
        &circuit_breakers,
    )?;
}
```

---

### ⚠️ Issue 6: Missing Transaction Costs

**Problem**: P&L calculation doesn't account for fees

```rust
// Current plan
let pnl = (exit_price - entry_price) * quantity;  // ⚠️ Missing fees
```

**Reality**: Solana charges ~0.000005 SOL per transaction
- Buy: Pay fee
- Sell: Pay fee

**Solution**: Add fees to model
```rust
pub struct Position {
    // ...
    pub entry_fee: f64,  // Fee paid when opening
}

pub struct ClosedPosition {
    // ...
    pub exit_fee: f64,   // Fee paid when closing
}

// In calculate_pnl
let gross_pnl = (exit_price - position.entry_price) * position.quantity;
let net_pnl = gross_pnl - position.entry_fee - exit_fee;
```

**For MVP**: Can hardcode fee as 0.000005 SOL * SOL price

---

### ⚠️ Issue 7: No Handling of Failed Transactions

**Problem**: What if buy/sell transaction fails?

Scenarios:
1. Insufficient SOL for gas
2. Network congestion
3. Slippage too high
4. Token account doesn't exist

**Solution**: Add transaction states
```rust
pub enum PositionStatus {
    Pending,      // ✅ Transaction submitted, waiting for confirmation
    Open,         // Transaction confirmed
    Closing,      // ✅ Close tx submitted
    Closed,       // Close tx confirmed
    Failed,       // ✅ Transaction failed
}

// In executor
match transaction_result {
    Ok(_) => {
        position.status = PositionStatus::Pending;
        // Wait for confirmation...
    }
    Err(e) => {
        tracing::error!("Transaction failed: {}", e);
        // Retry? Alert? Revert position?
    }
}
```

**For MVP**: Can skip this, assume all transactions succeed

---

### ⚠️ Issue 8: Race Condition in Exit Checks

**Problem**: Check exits, then process signals - price could change in between

```rust
// Tick 1: Price = $100 (no exit trigger)
position_manager.check_exits(&prices)?;  // No exits

// ... generate signals ...

// But price dropped to $91 during signal generation!
// Stop loss should have triggered but we missed it
```

**Solution**: Use same price snapshot for entire tick
```rust
loop {
    ticker.tick().await;

    // 1. Fetch prices ONCE
    let prices = fetch_all_prices().await?;

    // 2. Use same prices for exits
    position_manager.check_exits(&prices)?;

    // 3. Use same prices for signals
    for token in &tokens {
        let price = prices.get(&token.symbol).unwrap();
        let decision = executor.process_signal(&signal, token, *price, ...)?;
    }
}
```

---

## Edge Cases to Handle

### Edge Case 1: First Trade with Insufficient Funds
```rust
// Portfolio = $100, trying to buy SOL @ $100
// Max position = 5% = $5
// Can only buy 0.05 SOL

// ⚠️ Some exchanges have minimum order sizes!
// Need to check: quantity > minimum_trade_size
```

**Solution**: Add validation
```rust
const MIN_POSITION_VALUE: f64 = 10.0;  // $10 minimum

if position_value < MIN_POSITION_VALUE {
    return ExecutionDecision {
        action: ExecutionAction::Skip,
        reason: "Position too small".to_string(),
    };
}
```

---

### Edge Case 2: Token Price Goes to Zero
```rust
// Bought token at $100
// Token rugged, price = $0

// All exit conditions fail (stop loss never triggers because price < stop_loss forever)
// Position stuck open
```

**Solution**: Add max loss exit
```rust
if current_price <= 0.0 || (current_price / position.entry_price) < 0.5 {
    return Some(ExitReason::MaxLoss);  // -50% emergency exit
}
```

---

### Edge Case 3: Multiple Positions Hit Exits Simultaneously
```rust
// Have 3 positions: SOL, JUP, BONK
// Market crashes, all 3 trigger stop loss

// Need to close all 3 in same tick
// Do we have enough SOL for 3 transaction fees?
```

**Solution**: Check total fees before executing
```rust
let total_fees = positions_to_close.len() as f64 * TRANSACTION_FEE;
if sol_balance < total_fees {
    tracing::error!("Insufficient SOL for exit fees!");
    // Close positions in order of worst P&L first
}
```

---

### Edge Case 4: Circuit Breaker Triggers Mid-Day
```rust
// Morning: Portfolio = $10,000
// Trading happens...
// Afternoon: Portfolio = $9,400 (-6%, triggers -5% daily loss limit)

// What happens to open positions?
// - Leave them open? (could lose more)
// - Close them? (forced exit at bad time)
```

**Solution**: Define circuit breaker behavior
```rust
pub enum CircuitBreakerAction {
    PauseNewTrades,  // Keep positions, stop new ones
    CloseAll,        // Emergency exit everything
}
```

**Recommended for swing trading**: `PauseNewTrades` (don't panic sell)

---

## Missing from Plan

### Missing 1: Position History
**Issue**: Can't backtest or analyze performance without history

**Solution**: Add after MVP works
```rust
pub struct PositionHistory {
    closed_positions: Vec<Position>,
}

// Save to Redis or DB when position closes
persistence.save_position(&position).await?;
```

---

### Missing 2: Logging and Observability
**Issue**: How do we know what executor decided and why?

**Solution**: Structured logging
```rust
tracing::info!(
    event = "signal_processed",
    token = %token,
    signal = ?signal,
    action = ?decision.action,
    reason = %decision.reason,
    portfolio_value = %portfolio_value,
    open_positions = %position_count,
);
```

---

### Missing 3: Dry-Run Mode
**Issue**: Can't test without risking real money

**Solution**: Add dry-run flag
```rust
pub struct Executor {
    dry_run: bool,  // If true, log but don't execute
}

if self.dry_run {
    tracing::info!("DRY RUN: Would execute {:?}", decision);
    return Ok(());
}
```

---

### Missing 4: Position Limits per Token
**Issue**: Plan only has % limit, not absolute limit

**What if**: Token price drops 50% after buy?
- Position now worth $250 (was $500)
- Buy signal comes again
- Executor says: "No position in SOL" (because old one is still "open")

**Wait, no**: Plan prevents this with `has_open_position()` check

**Actually Missing**: What if we WANT multiple positions in same token at different prices?

**Decision**: For MVP, one position per token is fine. Can enhance later.

---

## Answers to Plan's Open Questions

### Q1: Should position manager track cash balance separately?
**Answer**: ✅ YES! Critical for correct portfolio calculations (see Issue #2)

### Q2: How to handle partial fills?
**Answer**: For MVP, assume full fills. Real exchanges need order tracking.

### Q3: Should we support multiple positions in same token?
**Answer**: For MVP, NO. One position per token keeps it simple.
Later: Can add if needed (e.g., DCA strategy)

### Q4: Trailing stop: Update on every tick or only on new highs?
**Answer**: Only on new highs (less compute, same result)

### Q5: What happens if signal comes before enough data?
**Answer**: Skip (already handled in main loop with `candles.len() >= samples_needed`)

---

## Revised Implementation Order

**Original order was correct**, but add these:

1. **Position Manager**
   - ✅ Add `cash_balance` field
   - ✅ Add `highest_price_seen` and `trailing_stop_active` to Position
   - ✅ Make `close_position` update `trading_state` atomically
   - ✅ Implement proper `portfolio_value` with cash
   - Write tests for everything

2. **Executor**
   - ✅ Remove `Arc<Mutex<>>`, use simple references
   - ✅ Fix position sizing to use available cash
   - ✅ Add minimum position size validation
   - Write tests

3. **Main Loop Integration**
   - ✅ Ensure same price snapshot used throughout tick
   - ✅ Add structured logging
   - ✅ Add dry-run mode (env var)

4. **Wallet Integration** (Later, after dry-run testing)
   - Add transaction sending
   - Handle failures gracefully
   - Add fee calculation

---

## Test Coverage Gaps

**Missing tests from original plan**:

```rust
// Position Manager
#[test]
fn test_cash_balance_updates_on_buy() { }

#[test]
fn test_cash_balance_updates_on_sell() { }

#[test]
fn test_portfolio_value_with_cash_and_positions() { }

#[test]
fn test_trading_state_updates_on_close() { }

#[test]
fn test_trailing_stop_activation() { }

#[test]
fn test_trailing_stop_follows_price_up() { }

// Executor
#[test]
fn test_skip_when_insufficient_cash() { }

#[test]
fn test_position_size_uses_available_cash() { }

#[test]
fn test_minimum_position_size() { }

// Integration
#[test]
fn test_full_cycle_with_profit() { }

#[test]
fn test_full_cycle_with_loss() { }

#[test]
fn test_stop_loss_closes_position() { }

#[test]
fn test_take_profit_closes_position() { }
```

---

## Security Considerations

### Private Key Handling
```rust
// ⚠️ NEVER log the private key
let keypair = Keypair::from_base58_string(&env::var("WALLET_PRIVATE_KEY")?);

// ✅ Log only the public key
tracing::info!("Trading wallet: {}", keypair.pubkey());
```

### Environment Variable Validation
```rust
// Check required env vars on startup
fn validate_env() -> Result<()> {
    env::var("WALLET_PRIVATE_KEY").context("WALLET_PRIVATE_KEY not set")?;
    env::var("INITIAL_PORTFOLIO_VALUE").context("INITIAL_PORTFOLIO_VALUE not set")?;
    // ...
    Ok(())
}
```

---

## Performance Considerations

**Not a concern for MVP**, but for reference:

- Position lookups: O(n) with Vec, could use HashMap<String, Position>
- Exit checks: O(n * m) where n = positions, m = price updates
- For 10 positions, 2 tokens: negligible

**Optimization needed when**: >100 positions (multi-user future)

---

## Summary of Required Changes to Plan

### Critical (Must Fix):
1. ✅ Add `cash_balance` to PositionManager
2. ✅ Track `highest_price_seen` for trailing stop
3. ✅ Fix position sizing to use available cash
4. ✅ Make state updates atomic
5. ✅ Remove Arc<Mutex<>>, use references

### Important (Should Fix):
6. ✅ Add minimum position size check
7. ✅ Add structured logging
8. ✅ Add dry-run mode
9. ✅ Same price snapshot per tick
10. ✅ Better error handling

### Nice-to-Have (Can Add Later):
11. ⏳ Transaction fee tracking
12. ⏳ Position history
13. ⏳ Transaction failure handling
14. ⏳ Emergency exit on max loss

---

## Revised Plan Is Ready ✅

**Verdict**: Original plan was 80% correct, but had critical issues with:
- Portfolio calculation (missing cash)
- Position sizing (using wrong amount)
- Concurrency pattern (overengineered)
- Trailing stop (incomplete logic)

**After fixes**: Plan is solid and ready for TDD implementation!

**Next**: Implement with tests
