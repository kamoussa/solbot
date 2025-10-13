# Project Decisions Log

## Strategy Decisions

### ✅ Trading Style: Swing Trading
- Hold period: 1-7 days (primary), up to 14 days max
- Trade frequency: 2-5 trades/week
- **Rationale**: Lower costs, better for LLM latency, more sustainable

### ✅ Exit Strategy
**Stop Loss**: Fixed -8% from entry price
**Take Profit**: Trailing stop
- Activate at +12% profit
- Trail by 5% from peak price
- Let winners run while protecting gains

**Time Stop**: Force exit after 14 days
- Prevents indefinite bag-holding
- Frees capital for new opportunities

**Rationale**: Simple, robust, 2:1+ risk/reward ratio

### ✅ LLM Approach
- Accept non-determinism
- Use confidence thresholds (e.g., >70% confidence required)
- Structured outputs (JSON format)
- **Rationale**: Pragmatic for MVP, can refine later

### ✅ Circuit Breakers
Implement:
- Max daily loss limit
- Max consecutive losses
- Max drawdown from peak
- Position size limits

**Rationale**: Prevent catastrophic losses

## Data Decisions

### ✅ Price Feeds
**Primary**: DexScreener API
- Historical data for backtesting
- Free tier sufficient
- Good Solana coverage

**Secondary**: Jupiter API
- Real-time quotes
- Execution routing
- Swap aggregation

**Rationale**: Free, reliable, covers all needs

### ✅ Backtesting Data
- Use DexScreener historical data (3-6 months)
- Supplement with paper trading (1 month minimum)
- Don't need perfect historical data for MVP

**Rationale**: Good enough to validate strategies, paper trading provides real-world validation

## Scope Decisions

### ✅ MVP Scope
**In scope**:
- Core trading logic
- Basic momentum strategies
- LLM watchlist curation
- Circuit breakers
- Paper trading mode

**Out of scope for now** (defer to later):
- Advanced wallet security (personal project)
- Disaster recovery (accept risk)
- Multi-strategy optimization
- Advanced monitoring dashboards
- Legal/regulatory compliance (personal use)

**Rationale**: Move fast, prove concept, iterate

## Technical Decisions

### ✅ Position Sizing
Start with: **Fixed 2-3% per position**
- Simple to implement
- Safe for initial testing
- Can add volatility adjustment later

### ✅ Token Universe
Start with: **5-10 established Solana tokens**
- SOL, JUP, PYTH, BONK, RAY, ORCA, JTO, WIF
- Avoid low-liquidity/scam tokens initially
- Expand carefully based on LLM recommendations

### ✅ Phase 1 Scope (Keep Ambitious)
User is comfortable with ambitious Phase 1:
1. Rust project structure
2. Price feed integration (DexScreener + Jupiter)
3. Data storage (PostgreSQL or TimescaleDB)
4. Basic momentum indicators
5. Logging and monitoring

**Rationale**: User preference, willing to invest time

## Risk Decisions

### ✅ Acceptable Risks for MVP
- LLM non-determinism
- Basic wallet security (personal project)
- No disaster recovery initially
- Limited historical data
- Phase 1 ambitious scope

**Rationale**: Personal project, learning exercise, willing to accept risks

### ✅ Must-Have Protections
- Circuit breakers (prevent catastrophic loss)
- Stop losses on every trade
- Position size limits
- Paper trading before live

**Rationale**: Balance speed with basic safety

## Budget & Capital

### 🔄 To Be Decided
- Monthly API budget
- Trading capital amount
- Timeline to paper/live trading
- Risk tolerance (max acceptable loss)

## Next Steps

Based on decisions above:
1. Update PLANNING.md with exit strategy details
2. Define circuit breaker thresholds
3. Move to implementation phase (TDD)
4. Start with Phase 0: Data collection setup
