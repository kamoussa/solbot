/// Regime Detection Accuracy Analysis
///
/// Compares ADX-based regime detection vs perfect hindsight labels
/// to identify detection errors and improvement opportunities.

use chrono::{DateTime, Utc};
use cryptobot::models::Candle;
use cryptobot::persistence::RedisPersistence;
use cryptobot::regime::{MarketRegime, RegimeDetector};
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
    // Note: This period had no clean range-bound behavior, all whipsaws
    MarketRegime::ChoppyUnclear
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RegimeComparison {
    timestamp: DateTime<Utc>,
    price: f64,
    perfect: MarketRegime,
    detected: MarketRegime,
    adx: f64,
    plus_di: f64,
    minus_di: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║       REGIME DETECTION ACCURACY ANALYSIS             ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("Comparing ADX detection vs perfect hindsight labels\n");

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

    println!("  ✓ Loaded {} candles\n", candles.len());

    // Create regime detector
    let detector = RegimeDetector::default();

    // Analyze regime detection accuracy
    let mut comparisons = Vec::new();
    let adx_period = 14;

    for i in adx_period..candles.len() {
        let window = &candles[..=i];
        let current = &candles[i];

        let perfect = perfect_regime(current.timestamp);

        if let Some(detected) = detector.detect_regime(window) {
            // Get ADX values for analysis
            if let Some((adx, plus_di, minus_di)) =
                cryptobot::indicators::calculate_adx(window, adx_period)
            {
                comparisons.push(RegimeComparison {
                    timestamp: current.timestamp,
                    price: current.close,
                    perfect,
                    detected,
                    adx,
                    plus_di,
                    minus_di,
                });
            }
        }
    }

    println!("═══════════════════════════════════════════════════════");
    println!("                  DETECTION ACCURACY                   ");
    println!("═══════════════════════════════════════════════════════\n");

    // Calculate confusion matrix
    let mut confusion: HashMap<(MarketRegime, MarketRegime), usize> = HashMap::new();
    let mut total_by_perfect: HashMap<MarketRegime, usize> = HashMap::new();
    let mut correct = 0;
    let mut total = comparisons.len();

    for comp in &comparisons {
        *confusion.entry((comp.perfect, comp.detected)).or_insert(0) += 1;
        *total_by_perfect.entry(comp.perfect).or_insert(0) += 1;

        if comp.perfect == comp.detected {
            correct += 1;
        }
    }

    let accuracy = (correct as f64 / total as f64) * 100.0;

    println!("Overall Accuracy: {:.1}% ({}/{} correct)", accuracy, correct, total);
    println!();

    // Confusion Matrix
    println!("Confusion Matrix (Perfect → Detected):");
    println!("─────────────────────────────────────────────────────────");
    println!("{:<20} {:>12} {:>12} {:>12} {:>12}", "Perfect ↓ / Detected →", "BullTrend", "BearCrash", "ChoppyClear", "ChoppyUnclear");
    println!("─────────────────────────────────────────────────────────");

    let regimes = [
        MarketRegime::BullTrend,
        MarketRegime::BearCrash,
        MarketRegime::ChoppyClear,
        MarketRegime::ChoppyUnclear,
    ];

    for perfect in &regimes {
        print!("{:<20}", format!("{:?}", perfect));
        for detected in &regimes {
            let count = confusion.get(&(*perfect, *detected)).unwrap_or(&0);
            let total_perfect = total_by_perfect.get(perfect).unwrap_or(&1);
            let pct = (*count as f64 / *total_perfect as f64) * 100.0;
            print!(" {:>7}({:>4.1}%)", count, pct);
        }
        println!();
    }

    println!("\n═══════════════════════════════════════════════════════");
    println!("              REGIME-BY-REGIME ANALYSIS                ");
    println!("═══════════════════════════════════════════════════════\n");

    // Analyze each regime period
    for regime in &regimes {
        let regime_comps: Vec<_> = comparisons
            .iter()
            .filter(|c| c.perfect == *regime)
            .collect();

        if regime_comps.is_empty() {
            continue;
        }

        let correct_count = regime_comps.iter().filter(|c| c.detected == c.perfect).count();
        let total_count = regime_comps.len();
        let accuracy = (correct_count as f64 / total_count as f64) * 100.0;

        println!("📊 {:?} Period:", regime);
        println!("   Total candles: {}", total_count);
        println!("   Correctly detected: {} ({:.1}%)", correct_count, accuracy);

        // Find first and last occurrence
        if let (Some(first), Some(last)) = (regime_comps.first(), regime_comps.last()) {
            println!("   Period: {} to {}", first.timestamp, last.timestamp);
            println!("   Price range: ${:.2} - ${:.2}", first.price, last.price);
        }

        // Show ADX statistics for this period
        let avg_adx = regime_comps.iter().map(|c| c.adx).sum::<f64>() / total_count as f64;
        let avg_plus_di = regime_comps.iter().map(|c| c.plus_di).sum::<f64>() / total_count as f64;
        let avg_minus_di =
            regime_comps.iter().map(|c| c.minus_di).sum::<f64>() / total_count as f64;

        println!("   Avg ADX: {:.1}", avg_adx);
        println!("   Avg +DI: {:.1}", avg_plus_di);
        println!("   Avg -DI: {:.1}", avg_minus_di);

        // Show most common misdetection
        let misdetections: Vec<_> = regime_comps
            .iter()
            .filter(|c| c.detected != c.perfect)
            .collect();

        if !misdetections.is_empty() {
            let mut misdetect_counts: HashMap<MarketRegime, usize> = HashMap::new();
            for comp in &misdetections {
                *misdetect_counts.entry(comp.detected).or_insert(0) += 1;
            }

            if let Some((most_common, count)) = misdetect_counts.iter().max_by_key(|(_, c)| *c) {
                let pct = (*count as f64 / misdetections.len() as f64) * 100.0;
                println!(
                    "   Most common misdetection: {:?} ({} times, {:.1}%)",
                    most_common, count, pct
                );
            }
        }

        println!();
    }

    println!("═══════════════════════════════════════════════════════");
    println!("                DETECTION FAILURES                     ");
    println!("═══════════════════════════════════════════════════════\n");

    // Show critical misdetections (transitions and extremes)
    println!("Showing first 10 misdetections in each regime:\n");

    for regime in &regimes {
        let misdetections: Vec<_> = comparisons
            .iter()
            .filter(|c| c.perfect == *regime && c.detected != c.perfect)
            .take(10)
            .collect();

        if misdetections.is_empty() {
            continue;
        }

        println!("❌ {:?} Misdetections:", regime);
        println!(
            "{:<20} {:>8} {:>12} {:>8} {:>8} {:>8}",
            "Date", "Price", "Detected As", "ADX", "+DI", "-DI"
        );
        println!("─────────────────────────────────────────────────────────");

        for comp in misdetections {
            println!(
                "{:<20} ${:>7.2} {:>12?} {:>7.1} {:>7.1} {:>7.1}",
                comp.timestamp.format("%Y-%m-%d"),
                comp.price,
                comp.detected,
                comp.adx,
                comp.plus_di,
                comp.minus_di
            );
        }
        println!();
    }

    println!("\n═══════════════════════════════════════════════════════");
    println!("                  RECOMMENDATIONS                      ");
    println!("═══════════════════════════════════════════════════════\n");

    // Provide recommendations based on accuracy
    if accuracy < 40.0 {
        println!("❌ POOR DETECTION (< 40% accuracy)");
        println!("   → ADX alone is insufficient for regime detection");
        println!("   → Need additional indicators (volume, volatility, price structure)");
        println!("   → Consider abandoning regime-based approach");
    } else if accuracy < 60.0 {
        println!("⚠️  MARGINAL DETECTION (40-60% accuracy)");
        println!("   → ADX provides some signal but with significant noise");
        println!("   → May improve with threshold tuning or additional filters");
        println!("   → Cost-benefit analysis needed before proceeding");
    } else if accuracy < 80.0 {
        println!("✅ MODERATE DETECTION (60-80% accuracy)");
        println!("   → ADX captures major regime shifts");
        println!("   → Fine-tuning thresholds could improve further");
        println!("   → May be viable with additional confirmation signals");
    } else {
        println!("✅ EXCELLENT DETECTION (> 80% accuracy)");
        println!("   → ADX effectively identifies market regimes");
        println!("   → Proceed with hybrid strategy implementation");
    }

    // Check if specific regimes are poorly detected
    println!("\nRegime-Specific Issues:");
    for regime in &regimes {
        let regime_comps: Vec<_> = comparisons
            .iter()
            .filter(|c| c.perfect == *regime)
            .collect();
        let correct = regime_comps.iter().filter(|c| c.detected == c.perfect).count();
        let regime_accuracy = (correct as f64 / regime_comps.len() as f64) * 100.0;

        if regime_accuracy < 50.0 {
            println!("   ⚠️  {:?}: {:.1}% accuracy (needs improvement)", regime, regime_accuracy);
        }
    }

    println!("\n═══════════════════════════════════════════════════════\n");

    Ok(())
}
