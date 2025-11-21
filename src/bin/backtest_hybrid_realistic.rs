/// Realistic Hybrid Strategy Backtest with ADX-based Regime Detection
///
/// Tests hybrid strategy using ADX to detect market regimes in real-time.
/// Compares realistic detection accuracy vs perfect hindsight performance.
///
/// This validates whether ADX can reliably detect regimes well enough to beat DCA.

use cryptobot::backtest::BacktestRunner;
use cryptobot::models::{Candle, Signal};
use cryptobot::persistence::RedisPersistence;
use cryptobot::regime::{CompositeRegimeDetector, LLMStrategySelector, MarketRegime, RegimeDetector, Strategy as StrategyEnum, TradingSignal, PositionContext};
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::buy_and_hold::BuyAndHoldStrategy;
use cryptobot::strategy::dca::DCAStrategy;
use cryptobot::strategy::mean_reversion::MeanReversionStrategy;
use cryptobot::strategy::momentum::MomentumStrategy;
use cryptobot::strategy::signals::SignalConfig;
use cryptobot::strategy::Strategy;
use cryptobot::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Hybrid strategy that uses ADX-based regime detection
struct RealisticHybridStrategy {
    momentum: MomentumStrategy,
    mean_reversion: MeanReversionStrategy,
    dca: BuyAndHoldStrategy,
    regime_detector: RegimeDetector,
    regime_counts: Arc<Mutex<HashMap<String, usize>>>,
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
            regime_counts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn get_regime_stats(&self) -> (usize, HashMap<String, usize>) {
        let counts = self.regime_counts.lock().unwrap();
        let total: usize = counts.values().sum();
        (total, counts.clone())
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

        // Track regime
        let regime_name = format!("{:?}", regime);
        *self.regime_counts.lock().unwrap().entry(regime_name).or_insert(0) += 1;

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
            MarketRegime::ChoppyClear => {
                // OPTION D: Use Mean Reversion for clean ranges (clear support/resistance)
                if candles.len() < 44 {
                    return Ok(Signal::Hold);
                }
                self.mean_reversion.generate_signal(candles)
            }
            MarketRegime::ChoppyUnclear => {
                // OPTION D: Stay in cash for whipsaws (no clear pattern)
                Ok(Signal::Hold)
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
            MarketRegime::ChoppyClear => {
                // OPTION D: Use Mean Reversion for clean ranges (clear support/resistance)
                if candles.len() < 44 {
                    return Ok(Signal::Hold);
                }
                self.mean_reversion.generate_signal(candles)
            }
            MarketRegime::ChoppyUnclear => {
                // OPTION D: Stay in cash for whipsaws (no clear pattern)
                Ok(Signal::Hold)
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
                    // Low confidence → OPTION B: stay in cash
                    Ok(Signal::Hold)
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
                    // Low confidence → OPTION B: stay in cash
                    Ok(Signal::Hold)
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

/// LLM-based Hybrid Strategy with sampling
///
/// Uses GPT-4 to detect regime with high accuracy, but samples every N hours
/// to reduce API costs ($0.0085 per call).
///
/// Sampling strategy:
/// Anti-Thrashing Regime Tracker
///
/// Drawdown analysis result
#[derive(Debug)]
struct DrawdownAnalysis {
    high_30d: f64,
    high_90d: f64,
    drawdown_from_30d_pct: f64,
    drawdown_from_90d_pct: f64,
    days_since_30d_high: usize,
    days_since_90d_high: usize,
    trend_30d_pct: f64,  // % change over last 30 days
}

/// Calculate drawdown metrics from price candles
fn calculate_drawdown_analysis(candles: &[Candle], current_price: f64) -> DrawdownAnalysis {
    // Find highest price in last 30 days (720 hours)
    let candles_30d: Vec<&Candle> = candles.iter().rev().take(720).collect();
    let high_30d = candles_30d.iter()
        .map(|c| c.high)
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(current_price);

    // Days since 30d high
    let days_since_30d_high = candles_30d.iter()
        .position(|c| c.high == high_30d)
        .unwrap_or(0) / 24;

    // Find highest price in last 90 days (2160 hours)
    let candles_90d: Vec<&Candle> = candles.iter().rev().take(2160).collect();
    let high_90d = candles_90d.iter()
        .map(|c| c.high)
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(current_price);

    // Days since 90d high
    let days_since_90d_high = candles_90d.iter()
        .position(|c| c.high == high_90d)
        .unwrap_or(0) / 24;

    // Drawdowns
    let drawdown_from_30d_pct = ((current_price - high_30d) / high_30d) * 100.0;
    let drawdown_from_90d_pct = ((current_price - high_90d) / high_90d) * 100.0;

    // 30-day trend (compare current to 30 days ago)
    let price_30d_ago = candles.iter().rev().nth(720).map(|c| c.close).unwrap_or(current_price);
    let trend_30d_pct = ((current_price - price_30d_ago) / price_30d_ago) * 100.0;

    DrawdownAnalysis {
        high_30d,
        high_90d,
        drawdown_from_30d_pct,
        drawdown_from_90d_pct,
        days_since_30d_high,
        days_since_90d_high,
        trend_30d_pct,
    }
}

/// Format drawdown analysis for LLM context
fn format_drawdown_context(dd: &DrawdownAnalysis, current_price: f64) -> String {
    let mut context = String::new();

    context.push_str("**DRAWDOWN ANALYSIS:**\n");
    context.push_str(&format!("  - Current price: ${:.2}\n", current_price));
    context.push_str(&format!("  - 30-day high: ${:.2} ({:.1}% drawdown, {} days ago)\n",
        dd.high_30d, dd.drawdown_from_30d_pct, dd.days_since_30d_high));
    context.push_str(&format!("  - 90-day high: ${:.2} ({:.1}% drawdown, {} days ago)\n",
        dd.high_90d, dd.drawdown_from_90d_pct, dd.days_since_90d_high));
    context.push_str(&format!("  - 30-day trend: {:.1}% {}\n",
        dd.trend_30d_pct.abs(),
        if dd.trend_30d_pct >= 0.0 { "UP" } else { "DOWN" }));

    // Interpretation
    if dd.drawdown_from_30d_pct < -20.0 {
        context.push_str(&format!(
            "  → ⚠️  SIGNIFICANT DOWNTREND: Price is {:.1}% below 30-day high. This is a SUSTAINED SELLOFF, not just noise.\n",
            dd.drawdown_from_30d_pct.abs()
        ));
        if dd.trend_30d_pct < 0.0 {
            context.push_str(&format!(
                "  → Price down {:.1}% over 30 days confirms bearish momentum. Dead cat bounces are likely.\n",
                dd.trend_30d_pct.abs()
            ));
        }
    } else if dd.drawdown_from_30d_pct > -5.0 && dd.trend_30d_pct > 5.0 {
        context.push_str("  → ✅ HEALTHY UPTREND: Near recent highs with positive 30d trend.\n");
    }

    context
}

/// Strategy Tracker for Option 3 (Direct Strategy Selection)
///
/// Similar to RegimeTracker but filters strategy recommendations instead of regimes.
/// Prevents rapid strategy switching by enforcing minimum duration and confidence thresholds.
///
/// Thresholds match the LLM prompt to ensure consistency:
/// - DCA: 0.65 confidence (default, easy to use)
/// - Momentum: 0.70-0.85 confidence (for young uptrends)
/// - Mean Reversion: 0.75+ confidence (for panic crashes)
///
/// Position-aware switching:
/// - Blocks switches FROM Momentum/MeanReversion when a position is open
/// - Allows switches FROM DCA anytime (no positions to protect)
struct StrategyTracker {
    current_strategy: Option<StrategyEnum>,
    strategy_start_sample: usize,
    min_duration_hours: usize,  // Minimum hours to hold a strategy (384 hours = 16 days)
    default_confidence_threshold: f64,  // 0.70 = Momentum/MeanReversion threshold
    dca_entry_threshold: f64,  // 0.65 = DCA entry (default)
    dca_exit_threshold: f64,  // 0.70 = Allow exit when LLM has 0.70-0.75 confidence
    switches_count: usize,
    // NEW: Track recent strategy history for trend memory
    strategy_history: Vec<(StrategyEnum, usize, f64)>,  // (strategy, start_sample, entry_confidence)
    entry_confidence: f64,  // Confidence when we entered current strategy
}

impl StrategyTracker {
    fn new() -> Self {
        Self {
            current_strategy: None,
            strategy_start_sample: 0,
            min_duration_hours: 0,  // NO minimum hold - allow tactical flexibility
            default_confidence_threshold: 0.80,  // LOWERED: 80% confidence to catch real bull runs (updated for 48h sampling)
            dca_entry_threshold: 0.65,  // Match prompt: DCA 0.65
            dca_exit_threshold: 0.80,  // LOWERED: 80% confidence to exit DCA (matches updated LLM "GOOD Momentum" tier)
            switches_count: 0,
            strategy_history: Vec::new(),
            entry_confidence: 0.0,
        }
    }

    /// Filter LLM's strategy recommendation through anti-thrashing rules
    ///
    /// Thresholds match the UPDATED LLM prompt (lowered for 48h sampling):
    /// - TO DCA: 0.65 confidence (default, unchanged)
    /// - TO Momentum: 0.80 confidence (LOWERED from 0.85 to catch real bull runs)
    /// - TO Mean Reversion: 0.80 confidence (LOWERED from 0.85, includes gradual bears)
    /// - FROM DCA: 0.80 confidence (allow exit when LLM has conviction in alternative)
    fn should_accept_strategy(
        &mut self,
        llm_strategy: StrategyEnum,
        llm_confidence: f64,
        current_sample: usize,
    ) -> StrategyEnum {
        // First detection - accept it
        if self.current_strategy.is_none() {
            self.current_strategy = Some(llm_strategy);
            self.strategy_start_sample = current_sample;
            self.entry_confidence = llm_confidence;
            println!("  🎯 STRATEGY TRACKER: Initial strategy {:?} (confidence: {:.2})", llm_strategy, llm_confidence);
            return llm_strategy;
        }

        let current = self.current_strategy.unwrap();
        let samples_in_strategy = current_sample - self.strategy_start_sample;

        // Same strategy - no change needed
        if llm_strategy == current {
            return current;
        }

        // ANTI-THRASHING RULES:

        // Rule 1: Too soon to switch (must hold for minimum duration in hours)
        if samples_in_strategy < self.min_duration_hours {
            println!(
                "  🚫 STRATEGY TRACKER: BLOCKED switch {:?} → {:?} (only {} hours, need {} hours / {:.1} days)",
                current, llm_strategy, samples_in_strategy, self.min_duration_hours, self.min_duration_hours as f64 / 24.0
            );
            return current;
        }

        // Rule 2: Check confidence threshold (SPECIAL HANDLING FOR DCA)
        let required_confidence = if llm_strategy == StrategyEnum::DCA {
            // Switching TO DCA: Use lower threshold (DCA is the winner!)
            self.dca_entry_threshold
        } else {
            // Switching TO Momentum/MeanReversion: Use normal threshold
            self.default_confidence_threshold
        };

        if llm_confidence < required_confidence {
            println!(
                "  🚫 STRATEGY TRACKER: BLOCKED switch {:?} → {:?} (confidence {:.2} < {:.2})",
                current, llm_strategy, llm_confidence, required_confidence
            );
            return current;
        }

        // Rule 3: Check when exiting DCA to alternative strategy
        if current == StrategyEnum::DCA {
            if llm_confidence < self.dca_exit_threshold {
                println!(
                    "  🚫 STRATEGY TRACKER: BLOCKED exit from DCA → {:?} (confidence {:.2} < {:.2})",
                    llm_strategy, llm_confidence, self.dca_exit_threshold
                );
                return current;
            }
        }

        // All checks passed - allow switch
        self.switches_count += 1;

        // Record old strategy to history before switching
        self.strategy_history.push((current, self.strategy_start_sample, self.entry_confidence));
        // Keep only last 3 entries for context
        if self.strategy_history.len() > 3 {
            self.strategy_history.remove(0);
        }

        self.current_strategy = Some(llm_strategy);
        self.strategy_start_sample = current_sample;
        self.entry_confidence = llm_confidence;
        println!(
            "  ✅ STRATEGY TRACKER: ACCEPTED switch {:?} → {:?} (confidence: {:.2}, held for {} hours / {:.1} days, switch #{})",
            current, llm_strategy, llm_confidence, samples_in_strategy, samples_in_strategy as f64 / 24.0, self.switches_count
        );
        llm_strategy
    }

    fn get_stats(&self) -> (usize, Option<StrategyEnum>) {
        (self.switches_count, self.current_strategy)
    }

    /// Generate formatted strategy context for LLM
    fn get_strategy_context(&self, current_sample: usize) -> String {
        let mut context = String::new();

        if let Some(current) = self.current_strategy {
            let hours_active = current_sample - self.strategy_start_sample;
            let days_active = hours_active as f64 / 24.0;

            context.push_str(&format!(
                "**CURRENT STRATEGY STATE:**\n  - Active strategy: {:?} (since {:.1} days ago at {:.2} confidence)\n",
                current, days_active, self.entry_confidence
            ));

            // Show recent history if any
            if !self.strategy_history.is_empty() {
                context.push_str("  - Recent history: ");
                for (strategy, start_sample, _confidence) in &self.strategy_history {
                    let duration_hours = if let Some(next_entry) = self.strategy_history.iter()
                        .position(|(s, ss, _)| s == strategy && ss == start_sample)
                        .and_then(|idx| self.strategy_history.get(idx + 1))
                    {
                        next_entry.1 - start_sample
                    } else {
                        self.strategy_start_sample - start_sample
                    };
                    let duration_days = duration_hours as f64 / 24.0;
                    context.push_str(&format!("{:?} ({:.1}d) → ", strategy, duration_days));
                }
                context.push_str(&format!("{:?} ({:.1}d) ← YOU ARE HERE\n", current, days_active));
            }

            // Anti-thrashing warning
            if hours_active < 96 {  // Less than 4 days
                context.push_str(&format!(
                    "  - ⚠️  WARNING: You switched to {:?} only {:.1} days ago. Avoid thrashing unless there's a STRONG reversal signal (confidence ≥ 0.90).\n",
                    current, days_active
                ));
            }
        } else {
            context.push_str("**CURRENT STRATEGY STATE:** No active strategy yet (first decision)\n");
        }

        context
    }
}

/// - Sample regime detection every `sample_interval` hours
/// - Cache regime for intermediate hours
/// - Anti-thrashing: Enforce minimum duration & confidence thresholds
/// - Reduces 8760 calls to ~180 calls with 48h sampling

/// Option 3: LLM Direct Strategy Selection (eliminates regime translation layer)
///
/// Uses LLM to directly recommend which strategy to use based on market conditions,
/// without the intermediate regime classification step that was causing mismatches.
///
/// Key advantages over regime-based approach:
/// - No regime → strategy translation layer (source of errors)
/// - LLM sees historical performance data (DCA: +1.96%, etc.)
/// - Conservative bias toward proven DCA strategy
/// - Anti-thrashing with 85% confidence (90% to exit DCA)
/// - Position-aware switching: blocks strategy changes when position is open
struct LLMDirectStrategyHybrid {
    momentum: MomentumStrategy,
    mean_reversion: MeanReversionStrategy,
    dca: BuyAndHoldStrategy,
    llm_selector: std::sync::Mutex<LLMStrategySelector>,
    strategy_tracker: std::sync::Mutex<StrategyTracker>,
    sample_interval: usize,
    call_count: std::sync::Mutex<usize>,
    cached_strategy: std::sync::Mutex<Option<StrategyEnum>>,
    strategy_log: std::sync::Mutex<Vec<(String, String, f64, String)>>, // (timestamp, strategy, confidence, reasoning)
}

impl LLMDirectStrategyHybrid {
    fn new(
        momentum: MomentumStrategy,
        mean_reversion: MeanReversionStrategy,
        dca: BuyAndHoldStrategy,
        llm_selector: LLMStrategySelector,
        sample_interval: usize,
    ) -> Self {
        Self {
            momentum,
            mean_reversion,
            dca,
            llm_selector: std::sync::Mutex::new(llm_selector),
            strategy_tracker: std::sync::Mutex::new(StrategyTracker::new()),
            sample_interval,
            call_count: std::sync::Mutex::new(0),
            cached_strategy: std::sync::Mutex::new(None),
            strategy_log: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn get_strategy_stats(&self) -> (usize, Option<StrategyEnum>) {
        self.strategy_tracker.lock().unwrap().get_stats()
    }

    fn save_strategy_log(&self, path: &str) -> std::io::Result<()> {
        use std::io::Write;
        let log = self.strategy_log.lock().unwrap();
        let mut file = std::fs::File::create(path)?;
        writeln!(file, "timestamp,strategy,confidence,reasoning")?;
        for (ts, strategy, conf, reasoning) in log.iter() {
            // Escape reasoning for CSV (replace quotes and commas)
            let escaped_reasoning = reasoning.replace("\"", "\"\"");
            writeln!(file, "{},{},{:.4},\"{}\"", ts, strategy, conf, escaped_reasoning)?;
        }
        println!("  💾 Saved {} strategy selections to {}", log.len(), path);
        Ok(())
    }
}

impl Strategy for LLMDirectStrategyHybrid {
    fn generate_signal(&self, candles: &[Candle]) -> cryptobot::Result<Signal> {
        if candles.is_empty() {
            return Ok(Signal::Hold);
        }

        // Increment call count and check if we should sample
        let mut call_count = self.call_count.lock().unwrap();
        let current_call = *call_count;
        *call_count += 1;
        drop(call_count);

        let should_sample = current_call % self.sample_interval == 0;

        if current_call % 100 == 0 || should_sample {
            println!(
                "🔍 DIRECT STRATEGY LLM DEBUG: call #{}, should_sample={}, sample_interval={}",
                current_call, should_sample, self.sample_interval
            );
        }

        let strategy = if should_sample {
            println!("🤖 LLM API CALL (Direct Strategy) at call #{}", current_call);

            // Generate strategy context from tracker
            let strategy_context = {
                let tracker = self.strategy_tracker.lock().unwrap();
                tracker.get_strategy_context(current_call)
            };

            // Generate drawdown analysis
            let current_price = candles.last().unwrap().close;
            let drawdown_analysis = calculate_drawdown_analysis(candles, current_price);
            let drawdown_context = format_drawdown_context(&drawdown_analysis, current_price);

            let strategy_result = tokio::task::block_in_place(|| {
                let mut selector = self.llm_selector.lock().unwrap();
                tokio::runtime::Handle::current().block_on(async {
                    selector.select_strategy_with_confidence_and_context(
                        candles,
                        Some(strategy_context),
                        Some(drawdown_context)
                    ).await
                })
            });

            match strategy_result {
                Ok((llm_strategy, llm_confidence, llm_reasoning)) => {
                    let timestamp = candles.last().unwrap().timestamp.to_rfc3339();
                    let strategy_str = format!("{:?}", llm_strategy);
                    self.strategy_log.lock().unwrap().push((timestamp.clone(), strategy_str, llm_confidence, llm_reasoning.clone()));
                    println!("  ✅ LLM RAW: {:?} (confidence: {:.2}) at {}", llm_strategy, llm_confidence, timestamp);
                    println!("      💭 Reasoning: {}", llm_reasoning);

                    // Filter through StrategyTracker to prevent thrashing
                    // Pass current position state to enable position-aware switching
                    let mut tracker = self.strategy_tracker.lock().unwrap();
                    let filtered_strategy = tracker.should_accept_strategy(llm_strategy, llm_confidence, current_call);
                    drop(tracker);

                    // Update cached strategy with FILTERED result
                    *self.cached_strategy.lock().unwrap() = Some(filtered_strategy);

                    filtered_strategy
                }
                Err(e) => {
                    println!("  ❌ LLM API ERROR: {}", e);
                    match *self.cached_strategy.lock().unwrap() {
                        Some(s) => {
                            println!("  ↩️  Using last cached strategy: {:?}", s);
                            s
                        }
                        None => return Ok(Signal::Hold),
                    }
                }
            }
        } else {
            // Use cached strategy
            match *self.cached_strategy.lock().unwrap() {
                Some(s) => s,
                None => return Ok(Signal::Hold),
            }
        };

        // Execute the selected strategy directly
        let signal = match strategy {
            StrategyEnum::Momentum => {
                if candles.len() < 25 {
                    return Ok(Signal::Hold);
                }
                self.momentum.generate_signal(candles)?
            }
            StrategyEnum::MeanReversion => {
                if candles.len() < 44 {
                    return Ok(Signal::Hold);
                }
                self.mean_reversion.generate_signal(candles)?
            }
            StrategyEnum::DCA => {
                self.dca.generate_signal(candles)?
            }
        };

        Ok(signal)
    }

    fn name(&self) -> &str {
        "Hybrid (LLM Direct Strategy)"
    }

    fn min_candles_required(&self) -> usize {
        50
    }
}

/// Option 4: LLM Direct Trading Signals (Buy/Sell/Hold)
///
/// Eliminates both the regime detection layer AND the strategy selection layer.
/// LLM directly provides Buy/Sell/Hold signals with full position awareness.
///
/// Key features:
/// - Position-aware: LLM knows if we have an open position, entry price, P&L, days held
/// - Hard-enforced 3-day minimum hold period (enforced in code, not just prompt)
/// - No strategy thrashing - direct trading decisions
/// - Swing trading timeframe (1-7 day holds typical)
struct LLMDirectTradingSignals {
    llm_selector: std::sync::Mutex<LLMStrategySelector>,
    sample_interval: usize,
    min_hold_hours: usize,  // Minimum 3 days = 72 hours
    call_count: std::sync::Mutex<usize>,
    position_state: std::sync::Mutex<PositionState>,
    signal_log: std::sync::Mutex<Vec<(String, String, f64, String)>>, // (timestamp, signal, confidence, reasoning)
}

#[derive(Clone)]
struct PositionState {
    has_position: bool,
    entry_price: f64,
    entry_time: usize,  // call_count when we entered
}

impl PositionState {
    fn new() -> Self {
        Self {
            has_position: false,
            entry_price: 0.0,
            entry_time: 0,
        }
    }
}

impl LLMDirectTradingSignals {
    fn new(llm_selector: LLMStrategySelector, sample_interval: usize, min_hold_hours: usize) -> Self {
        Self {
            llm_selector: std::sync::Mutex::new(llm_selector),
            sample_interval,
            min_hold_hours,
            call_count: std::sync::Mutex::new(0),
            position_state: std::sync::Mutex::new(PositionState::new()),
            signal_log: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn save_signal_log(&self, path: &str) -> std::io::Result<()> {
        use std::io::Write;
        let log = self.signal_log.lock().unwrap();
        let mut file = std::fs::File::create(path)?;
        writeln!(file, "timestamp,signal,confidence,reasoning")?;
        for (ts, signal, conf, reasoning) in log.iter() {
            let escaped_reasoning = reasoning.replace("\"", "\"\"");
            writeln!(file, "{},{},{:.4},\"{}\"", ts, signal, conf, escaped_reasoning)?;
        }
        println!("  💾 Saved {} trading signals to {}", log.len(), path);
        Ok(())
    }
}

impl Strategy for LLMDirectTradingSignals {
    fn generate_signal(&self, candles: &[Candle]) -> cryptobot::Result<Signal> {
        if candles.is_empty() {
            return Ok(Signal::Hold);
        }

        // Increment call count
        let mut call_count = self.call_count.lock().unwrap();
        let current_call = *call_count;
        *call_count += 1;
        drop(call_count);

        let should_sample = current_call % self.sample_interval == 0;

        if current_call % 100 == 0 || should_sample {
            println!(
                "🔍 DIRECT TRADING LLM DEBUG: call #{}, should_sample={}, sample_interval={}",
                current_call, should_sample, self.sample_interval
            );
        }

        // Build position context
        let position_state = self.position_state.lock().unwrap().clone();
        let current_price = candles.last().unwrap().close;
        let hours_held = if position_state.has_position {
            current_call - position_state.entry_time
        } else {
            0
        };
        let days_held = hours_held as f64 / 24.0;

        let pnl_percent = if position_state.has_position {
            Some(((current_price - position_state.entry_price) / position_state.entry_price) * 100.0)
        } else {
            None
        };

        let position_context = PositionContext {
            has_position: position_state.has_position,
            entry_price: if position_state.has_position { Some(position_state.entry_price) } else { None },
            current_price,
            pnl_percent,
            days_held: if position_state.has_position { Some(days_held) } else { None },
        };

        drop(position_state);

        // Check minimum hold period BEFORE calling LLM
        let can_sell = !position_context.has_position || hours_held >= self.min_hold_hours;

        if should_sample {
            println!("🤖 LLM API CALL (Direct Trading Signal) at call #{}", current_call);
            println!("  📊 Position: {}, Days held: {:.1}, Can sell: {}",
                if position_context.has_position { "LONG" } else { "CASH" },
                position_context.days_held.unwrap_or(0.0),
                can_sell
            );

            let signal_result = tokio::task::block_in_place(|| {
                let mut selector = self.llm_selector.lock().unwrap();
                tokio::runtime::Handle::current().block_on(async {
                    selector.get_trading_signal_with_context(candles, &position_context).await
                })
            });

            match signal_result {
                Ok((trading_signal, confidence, reasoning)) => {
                    let timestamp = candles.last().unwrap().timestamp.to_rfc3339();
                    let signal_str = trading_signal.as_str().to_string();
                    self.signal_log.lock().unwrap().push((
                        timestamp.clone(),
                        signal_str.clone(),
                        confidence,
                        reasoning.clone()
                    ));

                    println!("  ✅ LLM RAW: {:?} (confidence: {:.2})", trading_signal, confidence);
                    if let Some(pnl) = pnl_percent {
                        println!("      💰 Current P&L: {:.2}%", pnl);
                    }
                    println!("      💭 Reasoning: {}", reasoning);

                    // Convert TradingSignal to Signal, enforcing min hold period
                    let final_signal = match trading_signal {
                        TradingSignal::Buy => {
                            if position_context.has_position {
                                println!("      ⚠️  Already have position, converting Buy → Hold");
                                Signal::Hold
                            } else {
                                println!("      ✅ Executing Buy");
                                // Update position state
                                let mut pos = self.position_state.lock().unwrap();
                                pos.has_position = true;
                                pos.entry_price = current_price;
                                pos.entry_time = current_call;
                                Signal::Buy
                            }
                        }
                        TradingSignal::Sell => {
                            if !position_context.has_position {
                                println!("      ⚠️  No position to sell, converting Sell → Hold");
                                Signal::Hold
                            } else if !can_sell {
                                println!("      🚫 BLOCKED: Min hold period not met ({:.1}d < 3d), forcing Hold", days_held);
                                Signal::Hold
                            } else {
                                println!("      ✅ Executing Sell (held {:.1} days)", days_held);
                                // Update position state
                                let mut pos = self.position_state.lock().unwrap();
                                pos.has_position = false;
                                pos.entry_price = 0.0;
                                pos.entry_time = 0;
                                Signal::Sell
                            }
                        }
                        TradingSignal::Hold => Signal::Hold,
                    };

                    return Ok(final_signal);
                }
                Err(e) => {
                    println!("  ❌ LLM API ERROR: {}", e);
                    println!("  ↩️  Defaulting to Hold");
                    return Ok(Signal::Hold);
                }
            }
        }

        // Not sampling - just hold
        Ok(Signal::Hold)
    }

    fn name(&self) -> &str {
        "LLM Direct Trading (Buy/Sell/Hold)"
    }

    fn min_candles_required(&self) -> usize {
        50
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file
    dotenvy::dotenv().ok();

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
    println!("      • ADX < 20                     → Hold/Cash (choppy)");

    let hybrid_runner = BacktestRunner::new(initial_capital, circuit_breakers.clone());
    let hybrid_metrics =
        hybrid_runner.run(&realistic_hybrid, candles.clone(), "SOL", poll_interval, fee_rate)?;

    // Print regime distribution
    let (total_samples, regime_counts) = realistic_hybrid.get_regime_stats();
    if total_samples > 0 {
        println!("\n    📊 Regime Distribution ({} samples):", total_samples);
        let mut regimes: Vec<_> = regime_counts.iter().collect();
        regimes.sort_by(|a, b| b.1.cmp(a.1)); // Sort by count desc
        for (regime, count) in regimes {
            let pct = (*count as f64 / total_samples as f64) * 100.0;
            println!("       {} {} samples ({:.1}%)", regime, count, pct);
        }
    }

    // Test 2: Composite Hybrid (Multi-indicator regime detection)
    println!("\n  🔬 Testing Composite Hybrid with multi-indicator detection...");
    println!("    Detection logic:");
    println!("      • ATR + Volume + Structure + RSI + MA → Score-based regime");
    println!("      • Bull: Momentum    Crash: Mean Reversion    Choppy: Hold/Cash");

    let composite_runner = BacktestRunner::new(initial_capital, circuit_breakers.clone());
    let composite_metrics =
        composite_runner.run(&composite_hybrid, candles.clone(), "SOL", poll_interval, fee_rate)?;

    // Test 3: Confidence-based Hybrid (Only trade on high-confidence signals)
    println!("\n  🔬 Testing Confidence-based Hybrid (HIGH confidence only)...");
    println!("    Detection logic:");
    println!("      • Bull >= 5.0 confidence   → Momentum");
    println!("      • Bull < 5.0 confidence    → Hold/Cash (fallback)");
    println!("      • Crash >= 4.0 confidence  → Mean Reversion");
    println!("      • Crash < 4.0 confidence   → Hold/Cash (fallback)");
    println!("      • Choppy (any confidence)  → Hold/Cash");

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

    // Test 4: LLM-based Hybrid (SKIPPED - Option 1 removed to save time)
    println!("\n  ⏭️  Skipping LLM Regime-Based test (Option 1 - focusing on Option 3 only)");
    let llm_metrics: Option<cryptobot::backtest::BacktestMetrics> = None;

    // Test 5: LLM Direct Strategy Selection (Option 3) - SKIPPED to save time/credits
    println!("\n  ⏭️  Skipping LLM Direct Strategy test (Option 3 - focusing on Option 4)");
    let llm_direct_metrics: Option<cryptobot::backtest::BacktestMetrics> = None;

    // Test 6: LLM Direct Trading Signals (Buy/Sell/Hold) - COMMENTED OUT (not performant)
    /*
    let llm_trading_metrics = if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        println!("\n  🔬 Testing LLM Direct Trading Signals...");
        println!("    ✨ NEWEST APPROACH: Direct Buy/Sell/Hold signals!");
        println!("    Features:");
        println!("      • LLM directly provides: Buy, Sell, or Hold signals");
        println!("      • Position-aware: Knows entry price, P&L%, days held");
        println!("      • Hard-enforced 3-day minimum hold period");
        println!("      • No strategy layer - direct trading decisions");
        println!("      • Samples every 48 hours");
        println!("      • Est. API calls: ~183 (~$0.33 with gpt-4o-mini)");

        let llm_selector = LLMStrategySelector::new_no_cache(api_key);
        let llm_trading = LLMDirectTradingSignals::new(
            llm_selector,
            48,  // Sample every 48 hours
            72,  // 3-day minimum hold period (72 hours)
        );

        let llm_trading_runner = BacktestRunner::new(initial_capital, circuit_breakers.clone());
        let metrics = llm_trading_runner.run(&llm_trading, candles.clone(), "SOL", poll_interval, fee_rate)?;

        // Save trading signals for analysis
        llm_trading.save_signal_log("/tmp/llm_direct_trading_signals.csv").ok();

        Some(metrics)
    } else {
        println!("\n  ⏭️  Skipping LLM Direct Trading test (OPENAI_API_KEY not set)");
        None
    };
    */
    println!("\n  ⏭️  Skipping LLM Direct Trading test (not performant - commented out)");
    let llm_trading_metrics: Option<cryptobot::backtest::BacktestMetrics> = None;

    // Test 7: Pure Buy-and-Hold baseline
    println!("\n  🔬 Testing Pure Buy-and-Hold baseline...");
    println!("    • Buys once at start");
    println!("    • No automatic exits (holds forever)");
    println!("    • True HODL strategy");
    let pure_bnh_runner = BacktestRunner::new(initial_capital, circuit_breakers.clone());
    let pure_bnh_metrics = pure_bnh_runner.run(&buy_and_hold, candles.clone(), "SOL", poll_interval, fee_rate)?;

    // Test 8: True DCA (Dollar Cost Averaging) baseline
    println!("\n  🔬 Testing True DCA (Weekly) baseline...");
    println!("    • Buys fixed amount every 7 days (168 hours)");
    println!("    • Accumulates positions over time");
    println!("    • No automatic exits (pure accumulation)");
    println!("    • Averages entry price across purchases");
    let dca_weekly = DCAStrategy::weekly();
    let dca_runner = BacktestRunner::new(initial_capital, circuit_breakers.clone());
    let dca_metrics = dca_runner.run(&dca_weekly, candles.clone(), "SOL", poll_interval, fee_rate)?;

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

    // Show LLM results first if available
    if let Some(ref llm_trading) = llm_trading_metrics {
        println!(
            "✨ LLM Direct Trading (Opt 4)  {:+6.2}%    {:4}     {:+.2}%",
            llm_trading.net_return_pct,
            llm_trading.total_trades,
            llm_trading.net_return_pct - dca_metrics.net_return_pct
        );
    }

    if let Some(ref llm_direct) = llm_direct_metrics {
        println!(
            "🚀 LLM Direct Strategy (Opt 3) {:+6.2}%    {:4}     {:+.2}%",
            llm_direct.net_return_pct,
            llm_direct.total_trades,
            llm_direct.net_return_pct - dca_metrics.net_return_pct
        );
    }

    if let Some(ref llm) = llm_metrics {
        println!(
            "🤖 LLM Regime-based (Opt 1)    {:+6.2}%    {:4}     {:+.2}%",
            llm.net_return_pct,
            llm.total_trades,
            llm.net_return_pct - dca_metrics.net_return_pct
        );
    }

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
    println!("Perfect Hybrid (manual labels):         +3.42% (beats DCA by +1.45%)");

    if let Some(ref llm_trading) = llm_trading_metrics {
        println!(
            "✨ LLM Direct Trading Signals (Opt 4):  {:+.2}% (vs DCA: {:+.2}%)",
            llm_trading.net_return_pct,
            llm_trading.net_return_pct - dca_metrics.net_return_pct
        );
    }

    if let Some(ref llm_direct) = llm_direct_metrics {
        println!(
            "🚀 LLM Direct Strategy (Option 3):      {:+.2}% (vs DCA: {:+.2}%)",
            llm_direct.net_return_pct,
            llm_direct.net_return_pct - dca_metrics.net_return_pct
        );
    }

    if let Some(ref llm) = llm_metrics {
        println!(
            "🤖 LLM Regime-based (Option 1):         {:+.2}% (vs DCA: {:+.2}%)",
            llm.net_return_pct,
            llm.net_return_pct - dca_metrics.net_return_pct
        );
    }

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
    println!("\n📈 BASELINES:");
    println!("Pure Buy-and-Hold (HODL):               {:+.2}%", pure_bnh_metrics.net_return_pct);
    println!("True DCA (Weekly):                      {:+.2}%", dca_metrics.net_return_pct);

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
        println!("   • Stays in cash during choppy/uncertain markets (avoids forced trades)");
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
