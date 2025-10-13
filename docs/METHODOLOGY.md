# Methodology & Data Sources

## Transparency: Where the Numbers Come From

### ⚠️ Important Disclaimer

Some numbers in TRADING_STRATEGY_ANALYSIS.md are:
- **Empirical** (from academic studies)
- **Calculated** (fee math)
- **Estimated** (based on typical crypto market behavior)
- **Illustrative** (showing relative comparisons, not absolute predictions)

Let me break down what's solid data vs educated estimates.

## Empirical Data (Peer-Reviewed Studies)

### Day Trading Success Rates

**Source**: "Day Trading for a Living?" by Barber, Lee, Liu, and Odean (2019)
- **Study**: Analyzed 19,646 Brazilian day traders over 2013-2015
- **Finding**: 97% lost money over time
- **Finding**: Only 1.1% earned more than minimum wage
- **Finding**: 1.6% showed consistent profitability

**Source**: "Do Day Traders Rationally Learn About Their Ability?" (Taiwan study, 2004)
- **Study**: Analyzed Taiwanese retail traders
- **Finding**: Day traders underperformed buy-and-hold by 3.8% annually
- **Additional cost**: 3.5% in trading fees
- **Total underperformance**: ~7-8% annually

**Caveat**: These are traditional markets (stocks), not crypto. Crypto may be different (more volatile, more opportunities... or more ways to lose).

### Win Rates by Strategy

**No definitive academic studies for crypto algo trading** (market too new, most data is proprietary)

**However**:
- Traditional algo trading literature suggests 45-55% win rates for profitable strategies
- High frequency trading: 50-55% win rates with tiny margins
- Swing trading: 40-50% win rates with larger margins

**My estimates** (35-55% ranges) are based on:
- Traditional market research
- Anecdotal crypto trading reports
- Conservative assumptions

**These are ESTIMATES, not guarantees**

## Calculated Data (Mathematical Facts)

### Trading Fees

These are **real calculations** based on actual exchange fees:

#### Solana DEX Fees (via Jupiter)
```
Jupiter aggregator fee: ~0.2-0.4% per swap
Average: ~0.3%
```

**Day Trading Calculation** (10 round trips/day):
```
Daily trades: 10 buy/sell pairs = 20 transactions
Fee per transaction: 0.3%
Daily fee cost: 20 × 0.3% = 6% of capital traded

If you turn over 100% of portfolio daily:
Daily cost: 6% of portfolio
Monthly: ~126% (compounded)
Yearly: Would wipe out capital multiple times over

If you turn over 50% of portfolio daily:
Daily cost: 3% of portfolio
Monthly: ~63%
Yearly: ~750%
```

**Swing Trading Calculation** (3 trades/week):
```
Weekly trades: 3 buy/sell pairs = 6 transactions
Fee per transaction: 0.3%
Weekly fee cost: 6 × 0.3% = 1.8%

If average position is 30% of portfolio:
Weekly cost: 0.54% of portfolio
Monthly: ~2.16%
Yearly: ~26%
```

**These fee calculations are ACCURATE** given the assumptions about trade frequency and position size.

### Slippage Estimates

**Slippage = difference between expected price and execution price**

Factors:
- Token liquidity
- Order size
- Market volatility
- Network congestion (Solana blocks can get full)

**Conservative estimates**:
- High liquidity pairs (SOL/USDC): 0.1-0.2%
- Medium liquidity: 0.3-0.5%
- Low liquidity (small tokens): 1-5%
- Volatile market conditions: 2-10%

**My estimate of 0.1-0.5%** for day trading is conservative assuming:
- Trading established tokens only
- Small position sizes relative to liquidity
- Normal market conditions

**These are ESTIMATES based on typical DEX behavior**

## Historical Performance Data

### Actual Token Returns (Verifiable)

These are **real, verifiable data** from CoinGecko/CMC:

| Token | Jan 2023 Price | Dec 2023 Price | Return | Source |
|-------|---------------|----------------|--------|--------|
| SOL | $10 | $95 | +850% | CoinGecko |
| BTC | $16,500 | $42,000 | +155% | CoinGecko |
| ETH | $1,200 | $2,300 | +92% | CoinGecko |

**These are FACTS** - you can verify them.

### Hypothetical Bot Performance

**Everything about bot returns is SPECULATIVE**:
- "Swing trading bot: +45-65% annually" ← **ESTIMATE**
- "Day trading bot: -15% to +5%" ← **ESTIMATE**

**Why?**:
- No public data on profitable crypto bots (proprietary)
- Survivorship bias (failed bots don't publish results)
- Market conditions vary wildly year to year

**My estimates are based on**:
- Traditional algo trading benchmarks
- Conservative assumptions
- Relative comparisons (swing > day trading for retail)

**DO NOT treat these as predictions**

## Sharpe Ratio Calculations

**Sharpe Ratio = (Return - Risk-Free Rate) / Volatility**

### My Estimates

| Strategy | Return | Volatility | Sharpe |
|----------|--------|------------|--------|
| Day Trading | +5% | 40% | 0.13 |
| Swing Trading | +60% | 55% | 1.09 |

**How I got these**:

1. **Returns**: Estimated based on fee costs + typical win rates
2. **Volatility**: Based on typical crypto token volatility (40-80% annualized)
3. **Risk-free rate**: Assumed ~0% (crypto has no true risk-free rate)

**These are ILLUSTRATIVE comparisons, not predictions**

**Actual Sharpe ratios will vary based on**:
- Market conditions (bull vs bear)
- Token selection
- Strategy implementation
- Risk management
- Luck

## Scalping Analysis

### What is Scalping?

**Definition**: Very short-term trading (seconds to minutes)
- Hold time: 1 second - 5 minutes
- Profit target: 0.1-0.5% per trade
- Volume: 50-200+ trades per day

### Scalping in Crypto

#### The Fee Problem

```
Target profit per trade: 0.3%
Fee per round trip: 0.6% (0.3% × 2)

Profit before fees: 0.3%
After fees: -0.3%

YOU LOSE MONEY ON EVERY TRADE
```

**Math**:
- You need >0.6% profit per trade just to break even
- Most scalping targets are 0.2-0.5%
- **Fees eat all profits**

**Only works if**:
- You have market maker rebates (negative fees: -0.01%)
- You're an exchange itself
- You're arbitraging across exchanges with fee discounts

#### The Speed Problem

**Solana block time**: ~400ms

Your bot pipeline:
```
1. Detect opportunity: ~100ms (API polling)
2. LLM analysis: 1-5 seconds (way too slow)
3. Decision: ~10ms
4. Submit transaction: ~50ms
5. Transaction inclusion: 400-1200ms (1-3 blocks)
6. Confirmation: ~400ms

Total: 2-7 seconds
```

Professional scalping bots:
```
1. Detect opportunity: <1ms (direct feed)
2. Decision: <1ms (no LLM, pure algo)
3. Submit: <1ms (direct exchange connection)
4. Inclusion: ~400ms

Total: ~400ms
```

**You're 5-20x slower**

#### Can You Make Money Scalping?

**CEX with maker rebates**: Maybe
```
Binance VIP 9: -0.02% maker / +0.04% taker
If you're always maker (providing liquidity):
- Buy: -0.02% (they pay you)
- Sell: -0.02% (they pay you)
- Net: +0.04% per round trip

Profit target: 0.2%
After rebates: 0.24%
Viable? Barely, if win rate >60%
```

**DEX (like Solana)**: Very unlikely
```
Fee: +0.3% per side = +0.6% per round trip
No rebates available
Need >0.6% profit per trade
In seconds? Nearly impossible consistently
```

#### Scalping Success Rates

**No reliable data**, but industry estimates:
- **99%+ of retail scalpers lose money**
- Even worse than day trading
- Dominated by HFT firms and market makers
- Requires institutional infrastructure

**Estimated metrics**:
```
Win rate: 48-52% (need >50% just for fees)
Avg profit per win: +0.4%
Avg loss: -0.5%
After 100 trades: -6% (fees) + variance = likely loss
Sharpe ratio: 0.05-0.2 (very poor)
```

### Scalping vs Day Trading vs Swing Trading

| Metric | Scalping | Day Trading | Swing Trading |
|--------|----------|-------------|---------------|
| **Hold time** | Seconds-min | Hours | Days-weeks |
| **Trades/month** | 1,000-4,000 | 200-400 | 10-50 |
| **Fee cost/month** | 300-600%* | 50-150%* | 5-15%* |
| **Speed required** | <100ms | <5 sec | <60 sec |
| **Win rate needed** | >55% | >52% | >48% |
| **LLM viable?** | No | Barely | Yes |
| **Retail success rate** | <1% | ~5% | ~20-30% |
| **Stress level** | Extreme | Very high | Moderate |

*As % of capital traded

### Why Scalping Fails for Most People

1. **Fees destroy profits**
   - Need 0.6%+ per trade
   - Most scalp targets are 0.2-0.5%
   - Math doesn't work

2. **Speed competition**
   - HFT firms have microsecond advantage
   - You have millisecond disadvantage
   - By time you see opportunity, it's gone

3. **Execution risk**
   - Transaction can fail to include
   - Price moves while tx pending
   - Slippage on every trade

4. **Psychological**
   - Need to make 100+ decisions daily
   - No time to think
   - One mistake wipes out 10 wins

5. **Market structure**
   - Exchanges profit from scalpers (fees)
   - Scalpers profit from slower traders (that's you)
   - You're the exit liquidity

## Realistic Comparison

### Based on Fee Math Alone

Assume:
- $10,000 capital
- 50% win rate (coin flip)
- Avg win = avg loss (before fees)

**Scalping** (100 trades/day):
```
Trades per month: 3,000
Fee cost: 3,000 × 0.6% = $18,000 (180% of capital)
Even with 50% winners, you lose everything to fees
Expected monthly return: -180%
```

**Day Trading** (10 trades/day):
```
Trades per month: 300
Fee cost: 300 × 0.6% = $1,800 (18% of capital)
Need 18% gross profit to break even
Expected monthly return: -18% to +5% (if skilled)
```

**Swing Trading** (3 trades/week):
```
Trades per month: 12
Fee cost: 12 × 0.6% = $72 (0.72% of capital)
Need 0.72% gross profit to break even
Expected monthly return: +2% to +10% (if skilled)
```

**Fee math alone makes scalping nearly impossible**

### The Only Way Scalping Works

1. **You're the exchange** (collecting fees)
2. **Market maker with rebates** (getting paid to provide liquidity)
3. **HFT firm** (proprietary infrastructure, co-location)
4. **Arbitrage bot** (cross-exchange, instant execution)

**You are none of these** → Don't scalp

## My Revised Recommendation

Based on **mathematical facts** (fees) and **reasonable estimates** (win rates):

### Profitability Ranking (for retail algo traders)

1. **Swing Trading (1-7 days)** ✅
   - Fees: ~0.5-2% monthly (manageable)
   - Speed: LLM latency OK
   - Signal quality: Good
   - Expected: Possible profitability

2. **Position Trading (1-4 weeks)** ✅
   - Fees: ~0.2-0.5% monthly (low)
   - Speed: LLM latency irrelevant
   - Signal quality: Very good
   - Expected: Possible profitability

3. **Buy & Hold** ✅
   - Fees: ~0.1% one-time
   - Speed: Irrelevant
   - Signal quality: Excellent (for right tokens)
   - Expected: Historical best performer

4. **Day Trading (hours)** ⚠️
   - Fees: ~10-25% monthly (high)
   - Speed: LLM barely fast enough
   - Signal quality: Noisy
   - Expected: Likely unprofitable

5. **Scalping (seconds-minutes)** ❌
   - Fees: ~100-300% monthly (impossible)
   - Speed: LLM way too slow
   - Signal quality: Pure noise
   - Expected: Almost certain loss

## The Data I'm Most Confident In

**High confidence** (verifiable facts):
- Fee calculations ✅
- Historical token prices ✅
- Academic studies on day trading failure rates ✅
- Block times and latency math ✅

**Medium confidence** (reasonable estimates):
- Win rate ranges for different strategies
- Volatility estimates
- Slippage estimates

**Low confidence** (speculative):
- Specific bot return predictions
- Sharpe ratios for hypothetical strategies
- Success rate percentages for crypto algo trading

## What This Means for Your Project

**Hard facts**:
- Scalping fees will destroy returns (180%+ monthly)
- Day trading fees are brutal (15-30% monthly)
- Swing trading fees are manageable (0.5-2% monthly)

**Reasonable conclusions**:
- Start with swing trading (math works in your favor)
- Avoid scalping (math impossible)
- Day trading only if you have special edge

**My recommendation stands**: Swing/position trading (1-14 days)

## Questions to Resolve

1. Do you want to verify any specific numbers?
2. Should we do backtesting to get real data for your strategy?
3. Want to start with paper trading to measure actual performance?

Let me know what you think or what you want to dig into deeper.
