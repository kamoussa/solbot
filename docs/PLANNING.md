# CryptoBot - Planning Document

## Project Overview

A Solana-based cryptocurrency trading bot that uses LLM analysis combined with price momentum and social sentiment data to make buy/sell/hold decisions.

## ✅ DECIDED: Swing Trading Strategy (1-7 Day Holds)

**Chosen Approach**: Swing trading with trend following
- **Hold period**: 1-7 days (primary), with some positions extending to 1-4 weeks
- **Trade frequency**: 2-5 trades per week
- **Target returns**: 15-25% per winning trade, -6-8% stop losses
- **Expected annual return**: +60-120% (if profitable)
- **Monthly fee cost**: ~0.5-2% (sustainable)

**Why this approach**:
- LLM latency (1-5s) is acceptable for this timeframe
- Lower costs vs day trading (0.5% vs 15%+ monthly)
- Better signal-to-noise ratio in price movements
- Can capture larger moves (50-200% vs 2-5%)
- Less competition from HFT firms
- Sustainable stress levels
- Plays to LLM strengths (strategic analysis, sentiment interpretation)

**Architecture**:
- **LLM**: Strategic layer (every 30-60 min) - curates watchlist, analyzes sentiment, sets risk parameters
- **Fast Algorithms**: Tactical layer (real-time) - detects technical signals, executes trades, manages exits

## Critical Analysis & Concerns

### ⚠️ High-Risk Considerations

1. **LLM Limitations for Trading**
   - LLMs are non-deterministic and can hallucinate
   - High latency (1-5+ seconds per API call) vs millisecond market movements
   - API costs can be significant with frequent analysis
   - Not designed for numerical/financial analysis
   - **Recommendation**: Use LLM for qualitative analysis only (sentiment, news interpretation), not direct trading signals

2. **Timing & Latency Issues**
   - By the time LLM analyzes data and returns decision, prices may have changed significantly
   - Crypto markets move fast, especially on Solana (400ms blocks)
   - **Mitigation**: Use LLM for strategic decisions (which tokens to watch, general market sentiment), use faster algorithms for execution

3. **Social Sentiment Manipulation**
   - Pump and dump schemes actively manipulate social media
   - Coordinated shilling on Telegram/Twitter is common
   - Bots can inflate engagement metrics
   - **Mitigation**: Multi-source validation, anomaly detection, historical pattern analysis

4. **Regulatory & Legal**
   - Automated trading may have regulatory implications depending on jurisdiction
   - Need proper risk disclosures
   - Consider consulting legal counsel before live deployment

5. **Financial Risk**
   - Trading bots can lose money rapidly
   - Need robust risk management (position sizing, stop losses, max drawdown)
   - **Critical**: Start with paper trading and extensive backtesting

### Why Solana?

**Pros:**
- Fast transaction finality (~400ms)
- Low transaction fees
- Active DeFi ecosystem
- Good DEX aggregators (Jupiter)
- Strong developer tooling

**Cons:**
- Network stability issues (historical outages)
- Higher technical complexity than EVM chains
- Less liquidity than Ethereum for many tokens
- Rug pulls and scams are common

**Alternative consideration**: Could also support Ethereum/Base for more established tokens?

## Architecture Plan

### Core Components

#### 1. Data Collection Layer
- **Price Feeds**: Real-time and historical price data
- **Social Sentiment**: Twitter, Telegram, Reddit monitoring
- **On-chain Data**: Volume, liquidity, holder distribution
- **News/Events**: Crypto news aggregation

#### 2. Analysis Engine
- **Momentum Indicators**: RSI, MACD, moving averages, volume analysis
- **Sentiment Scoring**: Aggregate and score social data
- **LLM Analysis Module**: Qualitative analysis of aggregated data
- **Risk Assessment**: Volatility, liquidity checks, smart contract risk

#### 3. Decision Engine
- **Signal Aggregation**: Combine momentum + sentiment + LLM insights
- **Risk Management**: Position sizing, portfolio limits, stop losses
- **Strategy Rules**: Configurable trading strategies

#### 4. Execution Layer
- **Order Management**: Buy/sell execution via DEX
- **Wallet Management**: Secure key handling
- **Transaction Monitoring**: Confirmation and error handling

#### 5. Monitoring & Logging
- **Performance Tracking**: P&L, win rate, Sharpe ratio
- **System Metrics**: Latency, API usage, errors
- **Audit Trail**: All decisions and their rationale

## Data Sources

### Price Feeds (Solana)
1. **Birdeye API** - Comprehensive DEX data, good for Solana
2. **DexScreener API** - Multi-chain DEX aggregator
3. **Jupiter API** - Direct DEX aggregator with price quotes
4. **Pyth Network** - Oracle network with real-time prices
5. **Helius** - Solana RPC with enhanced APIs

### Social Sentiment
1. **Twitter/X API**
   - ⚠️ Expensive ($100-$5000/month for v2 API)
   - Alternative: Scraping (ToS violation risk) or third-party services
2. **Telegram**
   - Bot API (free, but requires bot in channels)
   - Limited historical data access
3. **Reddit API** - Free tier available
4. **LunarCrush** - Crypto social analytics (paid service)
5. **Santiment** - On-chain + social metrics (paid)

### On-Chain Data (Solana)
1. **Helius** - Enhanced Solana APIs
2. **QuickNode** - RPC provider with analytics
3. **Solscan API** - Block explorer data
4. **Solana Beach API** - Validator and network data

## Technology Stack

### Backend (Rust)
- **solana-sdk / solana-client** - Solana interaction
- **anchor-lang** - If interacting with Anchor programs
- **tokio** - Async runtime
- **sqlx** - Database (PostgreSQL for historical data)
- **redis** - Caching and real-time data
- **reqwest** - HTTP client for APIs
- **serde** - Serialization
- **ta** - Technical analysis library

### LLM Integration
- **OpenAI API** (GPT-4) - Most capable, expensive
- **Anthropic Claude** (via API) - Good reasoning, lower cost
- **Local models** (Llama 3.1) - Free but require GPU infrastructure
- **Recommendation**: Start with Claude 3.5 Sonnet for cost/performance balance

### DEX Integration
- **Jupiter Aggregator** - Best liquidity and routing on Solana
- **Raydium** - Direct integration as fallback

## Development Phases

### Phase 1: Foundation & Data Collection
1. Set up Rust project structure
2. Implement price feed integration (choose 1-2 sources)
3. Create data storage layer (PostgreSQL + Redis)
4. Build basic momentum indicators
5. Set up logging and monitoring

### Phase 2: Sentiment Analysis
1. Integrate Twitter/Telegram APIs
2. Build sentiment scoring algorithms
3. Create LLM integration for qualitative analysis
4. Test sentiment accuracy against known events

### Phase 3: Strategy & Backtesting
1. Implement trading strategies
2. Build backtesting framework with historical data
3. Paper trading mode (simulated execution)
4. Performance metrics and reporting

### Phase 4: Execution & Risk Management
1. Wallet management and security
2. Jupiter integration for swaps
3. Risk management rules (position limits, stop losses)
4. Transaction monitoring and error handling

### Phase 5: Live Trading (with caution)
1. Start with very small positions
2. Gradual scaling based on performance
3. Continuous monitoring and adjustment
4. Kill switch implementation

## Key Questions to Resolve

1. **Budget**: What's the budget for API costs (LLM, data feeds, RPC)?
2. **Capital**: How much capital to deploy? Start small!
3. **Risk Tolerance**: Max drawdown acceptable? Position size limits?
4. **Time Horizon**: Day trading, swing trading, or longer-term holds?
5. **Token Universe**: All Solana tokens or curated list? (recommend curated)
6. **LLM Role**: Advisory only or direct trading signals?
7. **Paper Trading Duration**: How long to test before live trading? (recommend 1-3 months minimum)

## Recommended Approach

### Short Term (MVP)
1. Focus on 5-10 well-known Solana tokens (SOL, BONK, JUP, PYTH, etc.)
2. Use free/low-cost data sources initially (Jupiter API, DexScreener)
3. Implement basic momentum strategies (moving average crossovers)
4. LLM does daily/weekly strategic analysis, not per-trade decisions
5. Paper trading only - no real money yet

### Medium Term
1. Add sentiment analysis from 1-2 sources
2. Expand token universe carefully
3. Refine strategies based on backtest results
4. Build comprehensive risk management
5. Consider micro-live trading ($100-500)

### Long Term
1. Multi-strategy approach
2. Advanced on-chain analysis
3. Machine learning for pattern recognition
4. Cross-chain expansion if Solana proves successful

## Success Metrics

- **Phase 1-3**: Positive returns in backtesting (Sharpe ratio > 1.0)
- **Phase 4**: Successful paper trading for 1+ month
- **Phase 5**: Profitable live trading with controlled risk

## Red Flags to Watch For

- Consistent losses over 2+ weeks
- High correlation with random chance
- Over-optimization (works in backtest, fails live)
- Excessive drawdowns (>20% of capital)
- High slippage or execution issues
- LLM costs exceeding trading profits

## Alternative Approach: LLM as Analyst

Instead of LLM making trading decisions, consider:
- LLM analyzes daily/weekly market summaries
- Identifies emerging trends or risks
- Curates token watchlist
- Fast quantitative algorithms make actual trades
- This reduces latency and cost while leveraging LLM strengths

## Next Steps

1. Decide on initial scope and constraints
2. Set up development environment
3. Choose specific data providers
4. Create initial project structure
5. Begin Phase 1 implementation with TDD
