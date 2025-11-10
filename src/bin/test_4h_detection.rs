/// 4-Hour Timeframe Regime Detection Test
///
/// Tests if 4h candles provide the optimal balance:
/// - Smooth enough to show clear trends (vs hourly noise)
/// - Granular enough to detect ranges (vs daily over-smoothing)

use chrono::{DateTime, Timelike, Utc};
use cryptobot::models::Candle;
use cryptobot::persistence::RedisPersistence;
use cryptobot::regime::{MarketRegime, RegimeDetector};
use cryptobot::Result;
use std::collections::HashMap;

/// Perfect hindsight regime labels
fn perfect_regime(timestamp: DateTime<Utc>) -> MarketRegime {
    if timestamp < DateTime::parse_from_rfc3339("2025-01-19T00:00:00Z").unwrap() {
        return MarketRegime::BullTrend;
    }
    if timestamp < DateTime::parse_from_rfc3339("2025-04-07T00:00:00Z").unwrap() {
        return MarketRegime::BearCrash;
    }
    MarketRegime::ChoppyUnclear
}

/// Convert hourly candles to 4-hour candles
fn hourly_to_4h(hourly_candles: &[Candle]) -> Vec<Candle> {
    let mut four_hour_candles = Vec::new();
    let mut i = 0;

    while i < hourly_candles.len() {
        let start_hour = hourly_candles[i].timestamp.hour();
        let start_4h_block = (start_hour / 4) * 4; // Round down to 0, 4, 8, 12, 16, 20

        let mut candle_4h = hourly_candles[i].clone();
        let mut high = candle_4h.high;
        let mut low = candle_4h.low;
        let mut volume = candle_4h.volume;
        let mut close = candle_4h.close;

        // Aggregate next 4 hours (or until next 4h block)
        let mut j = i + 1;
        let mut count = 1;
        while j < hourly_candles.len() && count < 4 {
            let current_hour = hourly_candles[j].timestamp.hour();
            let current_4h_block = (current_hour / 4) * 4;

            // Stop if we've moved to next 4h block
            if current_4h_block != start_4h_block {
                break;
            }

            high = high.max(hourly_candles[j].high);
            low = low.min(hourly_candles[j].low);
            volume += hourly_candles[j].volume;
            close = hourly_candles[j].close;
            count += 1;
            j += 1;
        }

        candle_4h.high = high;
        candle_4h.low = low;
        candle_4h.volume = volume;
        candle_4h.close = close;

        four_hour_candles.push(candle_4h);
        i = j;
    }

    four_hour_candles
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║       4-HOUR TIMEFRAME REGIME DETECTION TEST         ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("Testing optimal timeframe for ADX-based detection\n");

    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    println!("📡 Connecting to Redis...");
    let mut redis = RedisPersistence::new(&redis_url).await?;

    println!("📊 Loading hourly SOL data...");
    let hourly_candles = redis.load_all_candles("SOL").await?;
    println!("  ✓ Loaded {} hourly candles\n", hourly_candles.len());

    println!("📈 Converting to 4-hour candles...");
    let four_hour_candles = hourly_to_4h(&hourly_candles);
    println!("  ✓ Created {} 4-hour candles\n", four_hour_candles.len());

    // Test detection on 4h timeframe
    let detector = RegimeDetector::default();

    println!("Testing 4-hour timeframe ADX...");

    // For each hourly candle, find the corresponding 4h regime
    let mut comparisons = Vec::new();
    let mut four_h_idx = 0;

    for hourly in &hourly_candles {
        // Find the 4h candle that contains this hour
        while four_h_idx < four_hour_candles.len() - 1
            && four_hour_candles[four_h_idx + 1].timestamp <= hourly.timestamp {
            four_h_idx += 1;
        }

        // Use 4h candles up to this point for regime detection
        if four_h_idx >= 14 {
            let four_h_window = &four_hour_candles[..=four_h_idx];
            if let Some(detected) = detector.detect_regime(four_h_window) {
                let perfect = perfect_regime(hourly.timestamp);
                comparisons.push((perfect, detected));
            }
        }
    }

    println!("  ✓ Analyzed {} periods\n", comparisons.len());

    // Calculate accuracy
    let mut confusion: HashMap<(MarketRegime, MarketRegime), usize> = HashMap::new();
    let mut total_by_perfect: HashMap<MarketRegime, usize> = HashMap::new();
    let mut correct = 0;

    for (perfect, detected) in &comparisons {
        *confusion.entry((*perfect, *detected)).or_insert(0) += 1;
        *total_by_perfect.entry(*perfect).or_insert(0) += 1;
        if perfect == detected {
            correct += 1;
        }
    }

    let accuracy = (correct as f64 / comparisons.len() as f64) * 100.0;

    println!("═══════════════════════════════════════════════════════");
    println!("              4-HOUR TIMEFRAME RESULTS                 ");
    println!("═══════════════════════════════════════════════════════\n");

    println!("Overall Accuracy: {:.1}% ({}/{} correct)\n", accuracy, correct, comparisons.len());

    // Show confusion matrix
    println!("Confusion Matrix (Perfect → Detected):");
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

    // Show per-regime accuracy
    println!("\n─────────────────────────────────────────────────────────");
    println!("Per-Regime Accuracy:");
    println!("─────────────────────────────────────────────────────────");

    for regime in &regimes {
        if let Some(total) = total_by_perfect.get(regime) {
            let correct_count = confusion.get(&(*regime, *regime)).unwrap_or(&0);
            let regime_accuracy = (*correct_count as f64 / *total as f64) * 100.0;
            println!("{:<20} {:>6.1}% ({}/{})",
                format!("{:?}", regime), regime_accuracy, correct_count, total);
        }
    }

    println!("\n═══════════════════════════════════════════════════════");
    println!("              TIMEFRAME COMPARISON                     ");
    println!("═══════════════════════════════════════════════════════\n");

    println!("                Overall   Bull    Crash   Choppy");
    println!("─────────────────────────────────────────────────────────");
    println!("Hourly (1h):     49.5%   20.8%    0.9%   77.0%");
    println!("Daily (24h):     43.8%   21.7%   37.2%   52.4%");

    // Calculate per-regime for 4h
    let bull_acc = if let Some(total) = total_by_perfect.get(&MarketRegime::BullTrend) {
        let correct = confusion.get(&(MarketRegime::BullTrend, MarketRegime::BullTrend)).unwrap_or(&0);
        (*correct as f64 / *total as f64) * 100.0
    } else { 0.0 };

    let crash_acc = if let Some(total) = total_by_perfect.get(&MarketRegime::BearCrash) {
        let correct = confusion.get(&(MarketRegime::BearCrash, MarketRegime::BearCrash)).unwrap_or(&0);
        (*correct as f64 / *total as f64) * 100.0
    } else { 0.0 };

    let choppy_acc = if let Some(total) = total_by_perfect.get(&MarketRegime::ChoppyUnclear) {
        let correct = confusion.get(&(MarketRegime::ChoppyUnclear, MarketRegime::ChoppyUnclear)).unwrap_or(&0);
        (*correct as f64 / *total as f64) * 100.0
    } else { 0.0 };

    println!("4-Hour (4h):     {:>4.1}%   {:>4.1}%   {:>4.1}%   {:>4.1}%",
        accuracy, bull_acc, crash_acc, choppy_acc);

    println!("\n═══════════════════════════════════════════════════════");
    println!("                   VERDICT                             ");
    println!("═══════════════════════════════════════════════════════\n");

    if accuracy > 60.0 {
        println!("✅ BREAKTHROUGH! 4-hour timeframe provides significantly better detection");
        println!("   → {}% accuracy is viable for regime-based trading", accuracy);
        println!("   → Proceed to backtest with 4h regime detection");
        println!("\nRecommendation:");
        println!("  1. Use 4h candles for regime classification");
        println!("  2. Update regime detector every 4 hours");
        println!("  3. Execute strategies on hourly granularity");
    } else if accuracy > 55.0 {
        println!("⚠️  IMPROVEMENT! 4-hour timeframe is better than hourly/daily");
        println!("   → {}% accuracy shows promise", accuracy);
        println!("   → May be viable with additional indicators");
        println!("\nRecommendation:");
        println!("  1. Test with volume/volatility filters added");
        println!("  2. Or explore LLM-powered detection instead");
    } else if accuracy > 50.0 {
        println!("⚠️  MARGINAL: 4-hour timeframe shows small improvement");
        println!("   → {}% accuracy is still too low for reliable trading", accuracy);
        println!("\nRecommendation:");
        println!("  1. Explore LLM-powered detection");
        println!("  2. Or pivot to multi-token diversification");
    } else {
        println!("❌ NO IMPROVEMENT: 4-hour timeframe doesn't solve the problem");
        println!("   → {}% accuracy - ADX fundamentally cannot detect these regimes", accuracy);
        println!("\nRecommendation:");
        println!("  1. Abandon ADX-based regime detection");
        println!("  2. Test LLM-powered detection OR");
        println!("  3. Stick with DCA (proven +1.96%)");
    }

    println!("\n═══════════════════════════════════════════════════════\n");

    Ok(())
}
