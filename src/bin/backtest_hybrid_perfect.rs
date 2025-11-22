/// Perfect Hindsight Hybrid Strategy Backtest
///
/// Tests what performance we COULD achieve with perfect regime detection.
/// Manually labels time periods with regimes and switches strategies accordingly.
///
/// This validates whether regime-based strategy switching is worth building.

use chrono::{DateTime, Utc};
use cryptobot::backtest::BacktestRunner;
use cryptobot::models::{Candle, Signal};
use cryptobot::persistence::RedisPersistence;
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::buy_and_hold::BuyAndHoldStrategy;
use cryptobot::strategy::mean_reversion::MeanReversionStrategy;
use cryptobot::strategy::momentum::MomentumStrategy;
use cryptobot::strategy::signals::SignalConfig;
use cryptobot::strategy::Strategy;
use cryptobot::Result;

#[derive(Debug, Clone, Copy, PartialEq)]
enum MarketRegime {
    BullTrend,       // Strong uptrend - use Momentum
    BearCrash,       // Panic selloff - use Mean Reversion (buy the dip)
    ChoppyUnclear,   // Recovery with whipsaws - use DCA (avoid timing risk)
    // Note: ChoppyClear (clean range-bound) would use Mean Reversion,
    //       but we don't have this regime in our 365-day period
}

/// Manually label regimes based on SOL price history (perfect hindsight)
fn detect_regime(timestamp: DateTime<Utc>) -> MarketRegime {
    // Nov 2024 - Jan 19, 2025: Bull trend ($186 → $287, +54%)
    if timestamp < DateTime::parse_from_rfc3339("2025-01-19T00:00:00Z").unwrap() {
        return MarketRegime::BullTrend;
    }

    // Jan 19 - Apr 7, 2025: Crash/Bear ($287 → $97, -66%)
    if timestamp < DateTime::parse_from_rfc3339("2025-04-07T00:00:00Z").unwrap() {
        return MarketRegime::BearCrash;
    }

    // Apr 7 - Nov 2025: Choppy unclear recovery ($97 → $156, whipsaws, no clear range)
    MarketRegime::ChoppyUnclear
}

/// Hybrid strategy that switches based on regime
struct HybridStrategy {
    momentum: MomentumStrategy,
    mean_reversion: MeanReversionStrategy,
    dca: BuyAndHoldStrategy,
}

impl HybridStrategy {
    fn new(momentum: MomentumStrategy, mean_reversion: MeanReversionStrategy, dca: BuyAndHoldStrategy) -> Self {
        Self {
            momentum,
            mean_reversion,
            dca,
        }
    }
}

impl Strategy for HybridStrategy {
    fn generate_signal(&self, candles: &[Candle]) -> cryptobot::Result<Signal> {
        if candles.is_empty() {
            return Ok(Signal::Hold);
        }

        let current_candle = candles.last().unwrap();
        let regime = detect_regime(current_candle.timestamp);

        // Select strategy based on regime
        match regime {
            MarketRegime::BullTrend => {
                // Use Momentum in bull trends
                if candles.len() < 25 {
                    return Ok(Signal::Hold);
                }
                self.momentum.generate_signal(candles)
            }
            MarketRegime::BearCrash => {
                // Use Mean Reversion for crash dips (buy the panic)
                if candles.len() < 44 {
                    return Ok(Signal::Hold);
                }
                self.mean_reversion.generate_signal(candles)
            }
            MarketRegime::ChoppyUnclear => {
                // Use DCA for unclear/whipsaw recovery (avoid timing risk)
                self.dca.generate_signal(candles)
            }
        }
    }

    fn name(&self) -> &str {
        "Hybrid (Perfect Hindsight)"
    }

    fn min_candles_required(&self) -> usize {
        // Need enough for both strategies
        44
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║   PERFECT HINDSIGHT HYBRID STRATEGY BACKTEST         ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("Testing ceiling performance with perfect regime detection\n");

    // Connect to Redis
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    println!("📡 Connecting to Redis at {}...", redis_url);
    let mut redis = RedisPersistence::new(&redis_url).await?;

    // Load SOL data
    println!("📊 Loading SOL data from Redis...");
    let candles = redis.load_all_candles("SOL").await?;

    if candles.is_empty() {
        return Err("No candles found for SOL in Redis".into());
    }

    println!("  ✓ Loaded {} candles", candles.len());
    println!(
        "  Period: {} to {}",
        candles.first().unwrap().timestamp,
        candles.last().unwrap().timestamp
    );

    // Detect granularity
    let poll_interval = if candles.len() > 1 {
        let interval_secs = (candles[1].timestamp - candles[0].timestamp).num_seconds();
        if interval_secs >= 3000 {
            60 // hourly
        } else {
            5 // 5-minute
        }
    } else {
        60
    };

    // Create strategies
    let mut momentum_config = SignalConfig::default();
    momentum_config.rsi_oversold = 50.0; // Use optimized threshold
    let momentum = MomentumStrategy::new(momentum_config).with_poll_interval(poll_interval);
    let mean_reversion = MeanReversionStrategy::default().with_poll_interval(poll_interval);
    let buy_and_hold = BuyAndHoldStrategy::default();
    let hybrid = HybridStrategy::new(momentum.clone(), mean_reversion.clone(), buy_and_hold.clone());

    let initial_capital = 10_000.0;
    let fee_rate = 0.0075; // 0.75% realistic fees
    let circuit_breakers = CircuitBreakers::default();

    // Test 1: Perfect Hybrid (switches based on regime)
    println!("\n  🔬 Testing Perfect Hybrid strategy...");
    println!("    Regime switching logic:");
    println!("      • Bull Trend (Nov-Jan)     → Momentum");
    println!("      • Bear Crash (Jan-Apr)     → Mean Reversion (buy the dip)");
    println!("      • Choppy Unclear (Apr-Nov) → DCA (avoid whipsaws)");

    let hybrid_runner = BacktestRunner::new(initial_capital, circuit_breakers.clone());
    let hybrid_metrics = hybrid_runner
        .run(&hybrid, candles.clone(), "SOL", poll_interval, fee_rate)?;

    // Test 2: DCA baseline
    println!("\n  🔬 Testing DCA baseline...");
    let dca_runner = BacktestRunner::new(initial_capital, circuit_breakers.clone());
    let dca_metrics = dca_runner.run(&buy_and_hold, candles.clone(), "SOL", poll_interval, fee_rate)?;

    // Test 3: Momentum only
    println!("\n  🔬 Testing Momentum only...");
    let momentum_runner = BacktestRunner::new(initial_capital, circuit_breakers.clone());
    let momentum_metrics =
        momentum_runner.run(&momentum, candles.clone(), "SOL", poll_interval, fee_rate)?;

    // Test 4: Mean Reversion only
    println!("\n  🔬 Testing Mean Reversion only...");
    let mr_runner = BacktestRunner::new(initial_capital, circuit_breakers);
    let mr_metrics = mr_runner.run(&mean_reversion, candles, "SOL", poll_interval, fee_rate)?;

    // Report results
    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║              RESULTS COMPARISON                       ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    println!("Strategy                    Return%   Trades   vs DCA");
    println!("─────────────────────────────────────────────────────────");
    println!(
        "Perfect Hybrid              {:+6.2}%    {:4}     {:+.2}%",
        hybrid_metrics.net_return_pct,
        hybrid_metrics.total_trades,
        hybrid_metrics.net_return_pct - dca_metrics.net_return_pct
    );
    println!(
        "DCA (Baseline)              {:+6.2}%    {:4}       —",
        dca_metrics.net_return_pct, dca_metrics.total_trades
    );
    println!(
        "Momentum Only               {:+6.2}%    {:4}     {:+.2}%",
        momentum_metrics.net_return_pct,
        momentum_metrics.total_trades,
        momentum_metrics.net_return_pct - dca_metrics.net_return_pct
    );
    println!(
        "Mean Reversion Only         {:+6.2}%    {:4}     {:+.2}%",
        mr_metrics.net_return_pct,
        mr_metrics.total_trades,
        mr_metrics.net_return_pct - dca_metrics.net_return_pct
    );

    println!("\n─────────────────────────────────────────────────────────");

    // Verdict
    println!("\n🎯 VERDICT:");
    if hybrid_metrics.net_return_pct > dca_metrics.net_return_pct + 1.0 {
        println!(
            "✅ Perfect Hybrid beats DCA by {:.2}% - regime detection is WORTH IT!",
            hybrid_metrics.net_return_pct - dca_metrics.net_return_pct
        );
        println!("   Next step: Build ADX-based regime detector");
    } else if hybrid_metrics.net_return_pct > dca_metrics.net_return_pct {
        println!(
            "⚠️  Perfect Hybrid beats DCA by only {:.2}% - marginal benefit",
            hybrid_metrics.net_return_pct - dca_metrics.net_return_pct
        );
        println!("   Consider: Real detector will have lag/errors, may not beat DCA");
    } else {
        println!("❌ Perfect Hybrid LOSES to DCA - regime switching won't help");
        println!("   Even with perfect foresight, switching strategies underperforms!");
        println!("   Recommendation: Stick with DCA or try multi-token approach");
    }

    println!("\n═══════════════════════════════════════════════════════\n");

    Ok(())
}
