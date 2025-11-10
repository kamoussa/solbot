/// Realistic Hybrid Strategy Backtest with ADX-based Regime Detection
///
/// Tests hybrid strategy using ADX to detect market regimes in real-time.
/// Compares realistic detection accuracy vs perfect hindsight performance.
///
/// This validates whether ADX can reliably detect regimes well enough to beat DCA.

use cryptobot::backtest::BacktestRunner;
use cryptobot::models::{Candle, Signal};
use cryptobot::persistence::RedisPersistence;
use cryptobot::regime::{CompositeRegimeDetector, MarketRegime, RegimeDetector};
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::buy_and_hold::BuyAndHoldStrategy;
use cryptobot::strategy::mean_reversion::MeanReversionStrategy;
use cryptobot::strategy::momentum::MomentumStrategy;
use cryptobot::strategy::signals::SignalConfig;
use cryptobot::strategy::Strategy;
use cryptobot::Result;

/// Hybrid strategy that uses ADX-based regime detection
struct RealisticHybridStrategy {
    momentum: MomentumStrategy,
    mean_reversion: MeanReversionStrategy,
    dca: BuyAndHoldStrategy,
    regime_detector: RegimeDetector,
}

impl RealisticHybridStrategy {
    fn new(
        momentum: MomentumStrategy,
        mean_reversion: MeanReversionStrategy,
        dca: BuyAndHoldStrategy,
    ) -> Self {
        Self {
            momentum,
            mean_reversion,
            dca,
            regime_detector: RegimeDetector::default(),
        }
    }
}

impl Strategy for RealisticHybridStrategy {
    fn generate_signal(&self, candles: &[Candle]) -> cryptobot::Result<Signal> {
        if candles.is_empty() {
            return Ok(Signal::Hold);
        }

        // Detect current regime using ADX
        let regime = match self.regime_detector.detect_regime(candles) {
            Some(r) => r,
            None => return Ok(Signal::Hold), // Not enough data yet
        };

        // Select strategy based on detected regime
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
            MarketRegime::ChoppyUnclear | MarketRegime::ChoppyClear => {
                // Use DCA for choppy markets (avoid whipsaws)
                self.dca.generate_signal(candles)
            }
        }
    }

    fn name(&self) -> &str {
        "Hybrid (ADX Regime Detection)"
    }

    fn min_candles_required(&self) -> usize {
        // Need enough for ADX calculation + longest strategy requirement
        44
    }
}

/// Hybrid strategy using Composite multi-indicator regime detection
struct CompositeHybridStrategy {
    momentum: MomentumStrategy,
    mean_reversion: MeanReversionStrategy,
    dca: BuyAndHoldStrategy,
    composite_detector: CompositeRegimeDetector,
}

impl CompositeHybridStrategy {
    fn new(
        momentum: MomentumStrategy,
        mean_reversion: MeanReversionStrategy,
        dca: BuyAndHoldStrategy,
    ) -> Self {
        Self {
            momentum,
            mean_reversion,
            dca,
            composite_detector: CompositeRegimeDetector::default(),
        }
    }
}

impl Strategy for CompositeHybridStrategy {
    fn generate_signal(&self, candles: &[Candle]) -> cryptobot::Result<Signal> {
        if candles.is_empty() {
            return Ok(Signal::Hold);
        }

        // Detect current regime using Composite detector
        let regime = match self.composite_detector.detect_regime(candles) {
            Some(r) => r,
            None => return Ok(Signal::Hold), // Not enough data yet
        };

        // Select strategy based on detected regime
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
            MarketRegime::ChoppyUnclear | MarketRegime::ChoppyClear => {
                // Use DCA for choppy markets (avoid whipsaws)
                self.dca.generate_signal(candles)
            }
        }
    }

    fn name(&self) -> &str {
        "Hybrid (Composite Regime Detection)"
    }

    fn min_candles_required(&self) -> usize {
        // Need enough for composite indicator calculation + longest strategy requirement
        44
    }
}

/// Confidence-based Hybrid Strategy
///
/// Only trades on HIGH confidence signals, otherwise falls back to DCA.
/// Thresholds:
/// - Bull: >= 5.0 confidence → Momentum, else DCA
/// - Crash: >= 4.0 confidence → Mean Reversion, else DCA
struct ConfidenceHybridStrategy {
    momentum: MomentumStrategy,
    mean_reversion: MeanReversionStrategy,
    dca: BuyAndHoldStrategy,
    composite_detector: CompositeRegimeDetector,
    bull_confidence_threshold: f64,
    crash_confidence_threshold: f64,
}

impl ConfidenceHybridStrategy {
    fn new(
        momentum: MomentumStrategy,
        mean_reversion: MeanReversionStrategy,
        dca: BuyAndHoldStrategy,
        bull_confidence_threshold: f64,
        crash_confidence_threshold: f64,
    ) -> Self {
        Self {
            momentum,
            mean_reversion,
            dca,
            composite_detector: CompositeRegimeDetector::default(),
            bull_confidence_threshold,
            crash_confidence_threshold,
        }
    }
}

impl Strategy for ConfidenceHybridStrategy {
    fn generate_signal(&self, candles: &[Candle]) -> cryptobot::Result<Signal> {
        if candles.is_empty() {
            return Ok(Signal::Hold);
        }

        // Detect regime WITH confidence score
        let (regime, confidence) = match self.composite_detector.detect_regime_with_confidence(candles) {
            Some(result) => result,
            None => return Ok(Signal::Hold), // Not enough data yet
        };

        // Only trade on HIGH confidence signals, otherwise fallback to DCA
        match regime {
            MarketRegime::BullTrend => {
                if confidence >= self.bull_confidence_threshold {
                    // High confidence bull → use Momentum
                    if candles.len() < 25 {
                        return Ok(Signal::Hold);
                    }
                    self.momentum.generate_signal(candles)
                } else {
                    // Low confidence → fallback to DCA
                    self.dca.generate_signal(candles)
                }
            }
            MarketRegime::BearCrash => {
                if confidence >= self.crash_confidence_threshold {
                    // High confidence crash → use Mean Reversion
                    if candles.len() < 44 {
                        return Ok(Signal::Hold);
                    }
                    self.mean_reversion.generate_signal(candles)
                } else {
                    // Low confidence → fallback to DCA
                    self.dca.generate_signal(candles)
                }
            }
            MarketRegime::ChoppyUnclear | MarketRegime::ChoppyClear => {
                // Always use DCA for choppy markets
                self.dca.generate_signal(candles)
            }
        }
    }

    fn name(&self) -> &str {
        "Hybrid (Confidence-Based)"
    }

    fn min_candles_required(&self) -> usize {
        44
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║   REALISTIC HYBRID STRATEGY BACKTEST (ADX-BASED)     ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("Testing hybrid strategy with ADX regime detection\n");

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

    println!("  ✓ Detected {} minute intervals\n", poll_interval);

    // Create strategies
    let mut momentum_config = SignalConfig::default();
    momentum_config.rsi_oversold = 50.0; // Use optimized threshold
    let momentum = MomentumStrategy::new(momentum_config).with_poll_interval(poll_interval);
    let mean_reversion = MeanReversionStrategy::default().with_poll_interval(poll_interval);
    let buy_and_hold = BuyAndHoldStrategy::default();
    let realistic_hybrid =
        RealisticHybridStrategy::new(momentum.clone(), mean_reversion.clone(), buy_and_hold.clone());
    let composite_hybrid =
        CompositeHybridStrategy::new(momentum.clone(), mean_reversion.clone(), buy_and_hold.clone());

    let initial_capital = 10_000.0;
    let fee_rate = 0.0075; // 0.75% realistic fees
    let circuit_breakers = CircuitBreakers::default();

    // Test 1: Realistic Hybrid (ADX-based regime detection)
    println!("  🔬 Testing Realistic Hybrid with ADX detection...");
    println!("    Detection logic:");
    println!("      • ADX > 25 + +DI > -DI         → Momentum (bull trend)");
    println!("      • ADX > 25 + -DI > +DI + -10%  → Mean Reversion (crash)");
    println!("      • ADX < 20                     → DCA (choppy)");

    let hybrid_runner = BacktestRunner::new(initial_capital, circuit_breakers.clone());
    let hybrid_metrics =
        hybrid_runner.run(&realistic_hybrid, candles.clone(), "SOL", poll_interval, fee_rate)?;

    // Test 2: Composite Hybrid (Multi-indicator regime detection)
    println!("\n  🔬 Testing Composite Hybrid with multi-indicator detection...");
    println!("    Detection logic:");
    println!("      • ATR + Volume + Structure + RSI + MA → Score-based regime");
    println!("      • Bull: Momentum    Crash: Mean Reversion    Choppy: DCA");

    let composite_runner = BacktestRunner::new(initial_capital, circuit_breakers.clone());
    let composite_metrics =
        composite_runner.run(&composite_hybrid, candles.clone(), "SOL", poll_interval, fee_rate)?;

    // Test 3: Confidence-based Hybrid (Only trade on high-confidence signals)
    println!("\n  🔬 Testing Confidence-based Hybrid (HIGH confidence only)...");
    println!("    Detection logic:");
    println!("      • Bull >= 5.0 confidence   → Momentum");
    println!("      • Bull < 5.0 confidence    → DCA (fallback)");
    println!("      • Crash >= 4.0 confidence  → Mean Reversion");
    println!("      • Crash < 4.0 confidence   → DCA (fallback)");
    println!("      • Choppy (any confidence)  → DCA");

    let confidence_hybrid = ConfidenceHybridStrategy::new(
        momentum.clone(),
        mean_reversion.clone(),
        buy_and_hold.clone(),
        5.0, // Bull confidence threshold
        4.0, // Crash confidence threshold
    );
    let confidence_runner = BacktestRunner::new(initial_capital, circuit_breakers.clone());
    let confidence_metrics =
        confidence_runner.run(&confidence_hybrid, candles.clone(), "SOL", poll_interval, fee_rate)?;

    // Test 4: DCA baseline
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

    println!("Strategy                       Return%   Trades   vs DCA");
    println!("────────────────────────────────────────────────────────────");
    println!(
        "Confidence-Based (Bull≥5.0)    {:+6.2}%    {:4}     {:+.2}%",
        confidence_metrics.net_return_pct,
        confidence_metrics.total_trades,
        confidence_metrics.net_return_pct - dca_metrics.net_return_pct
    );
    println!(
        "Composite Hybrid (Multi)       {:+6.2}%    {:4}     {:+.2}%",
        composite_metrics.net_return_pct,
        composite_metrics.total_trades,
        composite_metrics.net_return_pct - dca_metrics.net_return_pct
    );
    println!(
        "Realistic Hybrid (ADX)         {:+6.2}%    {:4}     {:+.2}%",
        hybrid_metrics.net_return_pct,
        hybrid_metrics.total_trades,
        hybrid_metrics.net_return_pct - dca_metrics.net_return_pct
    );
    println!(
        "DCA (Baseline)                 {:+6.2}%    {:4}       —",
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

    // Comparison with baselines
    println!("\n📊 COMPARISON WITH BASELINES:\n");
    println!("Perfect Hybrid (manual labels):      +3.42% (beats DCA by +1.45%)");
    println!(
        "Confidence-Based (Bull≥5.0, Crash≥4.0): {:+.2}% (vs DCA: {:+.2}%)",
        confidence_metrics.net_return_pct,
        confidence_metrics.net_return_pct - dca_metrics.net_return_pct
    );
    println!(
        "Composite Hybrid (all signals):         {:+.2}% (vs DCA: {:+.2}%)",
        composite_metrics.net_return_pct,
        composite_metrics.net_return_pct - dca_metrics.net_return_pct
    );
    println!(
        "ADX-Only Hybrid:                        {:+.2}% (vs DCA: {:+.2}%)",
        hybrid_metrics.net_return_pct,
        hybrid_metrics.net_return_pct - dca_metrics.net_return_pct
    );
    println!("DCA Baseline:                           {:+.2}%", dca_metrics.net_return_pct);

    // Verdict
    println!("\n🎯 VERDICT:");

    let confidence_vs_dca = confidence_metrics.net_return_pct - dca_metrics.net_return_pct;

    if confidence_vs_dca > 1.0 {
        println!("✅ BREAKTHROUGH! Confidence-based approach BEATS DCA!");
        println!(
            "   Returns: {:+.2}% vs DCA's {:+.2}% (+{:.2}% improvement)",
            confidence_metrics.net_return_pct, dca_metrics.net_return_pct, confidence_vs_dca
        );
        println!("   Trades: {} (vs DCA's {})", confidence_metrics.total_trades, dca_metrics.total_trades);
        println!("\n   Why it works:");
        println!("   • Only trades when detector has HIGH confidence (Bull≥5.0, Crash≥4.0)");
        println!("   • Falls back to proven DCA when uncertain");
        println!("   • Avoids low-confidence whipsaws that hurt regular hybrid");
        println!("\n   Recommendation: DEPLOY confidence-based hybrid strategy");
        println!("   Next steps:");
        println!("     1. Test on more tokens to validate robustness");
        println!("     2. Fine-tune confidence thresholds (try Bull≥5.5, Crash≥4.5)");
        println!("     3. Monitor trade frequency and regime detection accuracy");
    } else if confidence_vs_dca > 0.5 {
        println!("⚠️  PROMISING: Confidence-based approach shows improvement");
        println!(
            "   Returns: {:+.2}% vs DCA's {:+.2}% (+{:.2}% improvement)",
            confidence_metrics.net_return_pct, dca_metrics.net_return_pct, confidence_vs_dca
        );
        println!("   Trades: {} (vs DCA's {})", confidence_metrics.total_trades, dca_metrics.total_trades);
        println!("\n   Gap from perfect hindsight: {:.2}%", 3.42 - confidence_metrics.net_return_pct);
        println!("   Next steps:");
        println!("     1. Try different confidence thresholds");
        println!("     2. Analyze which regime detections were profitable");
        println!("     3. Consider if marginal gains justify added complexity");
    } else if confidence_vs_dca > 0.0 {
        println!("⚠️  MARGINAL: Confidence-based barely beats DCA");
        println!(
            "   Returns: {:+.2}% vs DCA's {:+.2}% (+{:.2}% improvement)",
            confidence_metrics.net_return_pct, dca_metrics.net_return_pct, confidence_vs_dca
        );
        println!("   Trades: {} (vs DCA's {})", confidence_metrics.total_trades, dca_metrics.total_trades);
        println!("\n   Conclusion: Gains too small to justify complexity");
        println!("   Recommendation: STICK WITH DCA");
        println!("   • DCA is simpler, proven, and nearly identical returns");
        println!("   • Regime detection doesn't add enough value");
    } else {
        println!("❌ FAILED: Confidence-based LOSES to DCA");
        println!(
            "   Returns: {:+.2}% vs DCA's {:+.2}% ({:.2}% worse)",
            confidence_metrics.net_return_pct, dca_metrics.net_return_pct, confidence_vs_dca
        );
        println!("   Trades: {} (vs DCA's {})", confidence_metrics.total_trades, dca_metrics.total_trades);
        println!("\n   Even high-confidence signals underperform DCA");
        println!("   Recommendation: ABANDON regime detection approach");
        println!("   Alternatives:");
        println!("     1. Pure DCA (proven {:+.2}%)", dca_metrics.net_return_pct);
        println!("     2. LLM-powered regime detection (higher accuracy?)");
        println!("     3. Multi-token diversification instead of regime switching");
    }

    println!("\n═══════════════════════════════════════════════════════\n");

    Ok(())
}
