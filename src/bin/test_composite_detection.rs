/// Composite vs ADX-Only Regime Detection Comparison
///
/// Tests whether multi-indicator composite detection improves accuracy
/// over ADX-only detection (48.3% baseline).

use chrono::{DateTime, Utc};
use cryptobot::models::Candle;
use cryptobot::persistence::RedisPersistence;
use cryptobot::regime::{CompositeRegimeDetector, MarketRegime, RegimeDetector};
use cryptobot::Result;
use std::collections::HashMap;

/// Perfect hindsight regime labels
fn perfect_regime(timestamp: DateTime<Utc>) -> MarketRegime {
    // Nov 2024 - Jan 19, 2025: Bull trend ($186 → $287, +54%)
    if timestamp < DateTime::parse_from_rfc3339("2025-01-19T00:00:00Z").unwrap() {
        return MarketRegime::BullTrend;
    }

    // Jan 19 - Apr 7, 2025: Crash/Bear ($287 → $97, -66%)
    if timestamp < DateTime::parse_from_rfc3339("2025-04-07T00:00:00Z").unwrap() {
        return MarketRegime::BearCrash;
    }

    // Apr 7 - Nov 2025: Choppy unclear recovery (whipsaws, no clear range)
    MarketRegime::ChoppyUnclear
}

fn calculate_accuracy(
    candles: &[Candle],
    detector_name: &str,
    detect_fn: impl Fn(&[Candle]) -> Option<MarketRegime>,
) -> (f64, HashMap<(MarketRegime, MarketRegime), usize>) {
    let mut confusion: HashMap<(MarketRegime, MarketRegime), usize> = HashMap::new();
    let mut correct = 0;
    let mut total = 0;

    for i in 20..candles.len() {
        let window = &candles[..=i];
        let current = &candles[i];
        let perfect = perfect_regime(current.timestamp);

        if let Some(detected) = detect_fn(window) {
            *confusion.entry((perfect, detected)).or_insert(0) += 1;
            if perfect == detected {
                correct += 1;
            }
            total += 1;
        }
    }

    let accuracy = if total > 0 {
        (correct as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    println!("  {}: {:.1}% ({}/{} correct)", detector_name, accuracy, correct, total);

    (accuracy, confusion)
}

fn print_confusion_matrix(
    confusion: &HashMap<(MarketRegime, MarketRegime), usize>,
    title: &str,
) {
    println!("\n{}", title);
    println!("─────────────────────────────────────────────────────────");
    println!("{:<20} {:>12} {:>12} {:>12} {:>12}",
        "Perfect ↓", "BullTrend", "BearCrash", "ChoppyClear", "ChoppyUnclear");
    println!("─────────────────────────────────────────────────────────");

    let regimes = [
        MarketRegime::BullTrend,
        MarketRegime::BearCrash,
        MarketRegime::ChoppyClear,
        MarketRegime::ChoppyUnclear,
    ];

    let mut total_by_perfect: HashMap<MarketRegime, usize> = HashMap::new();
    for ((perfect, _), count) in confusion {
        *total_by_perfect.entry(*perfect).or_insert(0) += count;
    }

    for perfect in &regimes {
        print!("{:<20}", format!("{:?}", perfect));
        for detected in &regimes {
            let count = confusion.get(&(*perfect, *detected)).unwrap_or(&0);
            let total = total_by_perfect.get(perfect).unwrap_or(&1);
            let pct = (*count as f64 / *total as f64) * 100.0;
            print!(" {:>7}({:>4.1}%)", count, pct);
        }
        println!();
    }
}

fn print_per_regime_accuracy(
    confusion: &HashMap<(MarketRegime, MarketRegime), usize>,
    title: &str,
) {
    println!("\n{}", title);
    println!("─────────────────────────────────────────────────────────");

    let regimes = [
        MarketRegime::BullTrend,
        MarketRegime::BearCrash,
        MarketRegime::ChoppyClear,
        MarketRegime::ChoppyUnclear,
    ];

    let mut total_by_perfect: HashMap<MarketRegime, usize> = HashMap::new();
    for ((perfect, _), count) in confusion {
        *total_by_perfect.entry(*perfect).or_insert(0) += count;
    }

    for regime in &regimes {
        if let Some(total) = total_by_perfect.get(regime) {
            let correct_count = confusion.get(&(*regime, *regime)).unwrap_or(&0);
            let regime_accuracy = (*correct_count as f64 / *total as f64) * 100.0;
            println!("{:<20} {:>6.1}% ({}/{})",
                format!("{:?}", regime), regime_accuracy, correct_count, total);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║    COMPOSITE vs ADX-ONLY DETECTION COMPARISON        ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("Testing multi-indicator composite vs ADX-only (48.3% baseline)\n");

    // Connect to Redis
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    println!("📡 Connecting to Redis...");
    let mut redis = RedisPersistence::new(&redis_url).await?;

    println!("📊 Loading SOL data...");
    let candles = redis.load_all_candles("SOL").await?;

    if candles.is_empty() {
        return Err("No candles found for SOL in Redis".into());
    }

    println!("  ✓ Loaded {} candles\n", candles.len());

    println!("═══════════════════════════════════════════════════════");
    println!("              TESTING DETECTORS                        ");
    println!("═══════════════════════════════════════════════════════\n");

    // Test ADX-only detector
    let adx_detector = RegimeDetector::default();
    let (adx_accuracy, adx_confusion) = calculate_accuracy(
        &candles,
        "ADX-Only",
        |window| adx_detector.detect_regime(window),
    );

    // Test Composite detector
    let composite_detector = CompositeRegimeDetector::default();
    let (composite_accuracy, composite_confusion) = calculate_accuracy(
        &candles,
        "Composite",
        |window| composite_detector.detect_regime(window),
    );

    let improvement = composite_accuracy - adx_accuracy;

    println!("\n═══════════════════════════════════════════════════════");
    println!("                  RESULTS SUMMARY                      ");
    println!("═══════════════════════════════════════════════════════\n");

    println!("ADX-Only:    {:.1}%", adx_accuracy);
    println!("Composite:   {:.1}%", composite_accuracy);
    println!("Improvement: {:+.1}%\n", improvement);

    // Print detailed confusion matrices
    print_confusion_matrix(&adx_confusion, "ADX-Only Confusion Matrix");
    print_confusion_matrix(&composite_confusion, "Composite Confusion Matrix");

    // Print per-regime accuracy
    print_per_regime_accuracy(&adx_confusion, "ADX-Only Per-Regime Accuracy");
    print_per_regime_accuracy(&composite_confusion, "Composite Per-Regime Accuracy");

    println!("\n═══════════════════════════════════════════════════════");
    println!("                   VERDICT                             ");
    println!("═══════════════════════════════════════════════════════\n");

    if composite_accuracy > 60.0 {
        println!("✅ BREAKTHROUGH! Composite detector is viable for deployment");
        println!("   → {:.1}% accuracy is sufficient for regime-based trading", composite_accuracy);
        println!("   → {:+.1}% improvement over ADX-only", improvement);
        println!("\nRecommendation:");
        println!("  1. Proceed with backtest using composite detector");
        println!("  2. Target: beat DCA's +1.96% return");
        println!("  3. If backtest successful → deploy to production");
    } else if composite_accuracy > 55.0 {
        println!("⚠️  PROMISING! Composite detector shows significant improvement");
        println!("   → {:.1}% accuracy ({:+.1}% improvement)", composite_accuracy, improvement);
        println!("   → May be viable with additional tuning");
        println!("\nRecommendation:");
        println!("  1. Run backtest to test actual trading performance");
        println!("  2. Consider adding more filters/indicators");
        println!("  3. Or test LLM-powered detection for further improvement");
    } else if composite_accuracy > adx_accuracy {
        println!("⚠️  MARGINAL IMPROVEMENT: Composite detector is better but still insufficient");
        println!("   → {:.1}% accuracy ({:+.1}% improvement)", composite_accuracy, improvement);
        println!("   → Too low for reliable regime-based trading");
        println!("\nRecommendation:");
        println!("  1. Test LLM-powered detection (target >60%)");
        println!("  2. Or stick with pure DCA strategy (+1.96% proven)");
    } else {
        println!("❌ NO IMPROVEMENT: Composite detector performs worse than ADX-only");
        println!("   → {:.1}% accuracy ({:+.1}% vs ADX)", composite_accuracy, improvement);
        println!("\nRecommendation:");
        println!("  1. Debug composite scoring logic");
        println!("  2. Or abandon quantitative detection → test LLM approach");
        println!("  3. Or stick with DCA (proven +1.96%)");
    }

    println!("\n═══════════════════════════════════════════════════════\n");

    Ok(())
}
