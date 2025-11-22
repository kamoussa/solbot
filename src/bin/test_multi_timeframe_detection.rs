/// Multi-Timeframe Regime Detection Test
///
/// Uses daily candles for regime detection (smoother, clearer trends)
/// while keeping hourly granularity for strategy execution.

use chrono::{DateTime, Utc};
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

/// Convert hourly candles to daily candles
fn hourly_to_daily(hourly_candles: &[Candle]) -> Vec<Candle> {
    let mut daily_candles = Vec::new();
    let mut i = 0;

    while i < hourly_candles.len() {
        let start_date = hourly_candles[i].timestamp.date_naive();
        let mut daily_candle = hourly_candles[i].clone();

        let mut high = daily_candle.high;
        let mut low = daily_candle.low;
        let mut volume = daily_candle.volume;
        let mut close = daily_candle.close;

        // Aggregate all candles for this day
        let mut j = i + 1;
        while j < hourly_candles.len() && hourly_candles[j].timestamp.date_naive() == start_date {
            high = high.max(hourly_candles[j].high);
            low = low.min(hourly_candles[j].low);
            volume += hourly_candles[j].volume;
            close = hourly_candles[j].close;
            j += 1;
        }

        daily_candle.high = high;
        daily_candle.low = low;
        daily_candle.volume = volume;
        daily_candle.close = close;

        daily_candles.push(daily_candle);
        i = j;
    }

    daily_candles
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║     MULTI-TIMEFRAME REGIME DETECTION TEST            ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("Using DAILY candles for regime detection\n");

    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    println!("📡 Connecting to Redis...");
    let mut redis = RedisPersistence::new(&redis_url).await?;

    println!("📊 Loading hourly SOL data...");
    let hourly_candles = redis.load_all_candles("SOL").await?;
    println!("  ✓ Loaded {} hourly candles\n", hourly_candles.len());

    println!("📈 Converting to daily candles...");
    let daily_candles = hourly_to_daily(&hourly_candles);
    println!("  ✓ Created {} daily candles\n", daily_candles.len());

    // Test detection on daily timeframe
    let detector = RegimeDetector::default();

    println!("Testing daily timeframe ADX...");

    // For each hourly candle, find the corresponding daily regime
    let mut comparisons = Vec::new();
    let mut daily_idx = 0;

    for hourly in &hourly_candles {
        // Find the daily candle that contains this hour
        while daily_idx < daily_candles.len() - 1
            && daily_candles[daily_idx + 1].timestamp <= hourly.timestamp {
            daily_idx += 1;
        }

        // Use daily candles up to this point for regime detection
        if daily_idx >= 14 {
            let daily_window = &daily_candles[..=daily_idx];
            if let Some(detected) = detector.detect_regime(daily_window) {
                let perfect = perfect_regime(hourly.timestamp);
                comparisons.push((perfect, detected));
            }
        }
    }

    println!("  ✓ Analyzed {} periods\n", comparisons.len());

    // Calculate accuracy
    let mut confusion: HashMap<(MarketRegime, MarketRegime), usize> = HashMap::new();
    let mut correct = 0;

    for (perfect, detected) in &comparisons {
        *confusion.entry((*perfect, *detected)).or_insert(0) += 1;
        if perfect == detected {
            correct += 1;
        }
    }

    let accuracy = (correct as f64 / comparisons.len() as f64) * 100.0;

    println!("═══════════════════════════════════════════════════════");
    println!("              DAILY TIMEFRAME RESULTS                  ");
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

    let mut total_by_perfect: HashMap<MarketRegime, usize> = HashMap::new();
    for (perfect, _) in &comparisons {
        *total_by_perfect.entry(*perfect).or_insert(0) += 1;
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

    println!("\n═══════════════════════════════════════════════════════");
    println!("              COMPARISON                               ");
    println!("═══════════════════════════════════════════════════════\n");

    println!("Hourly Timeframe:   49.5% accuracy");
    println!("Daily Timeframe:    {:.1}% accuracy", accuracy);
    println!();

    if accuracy > 55.0 {
        println!("✅ IMPROVEMENT! Daily timeframe provides better regime detection");
        println!("   → Use daily candles for regime classification");
        println!("   → Keep hourly candles for precise strategy execution");
    } else if accuracy > 50.0 {
        println!("⚠️  MARGINAL improvement from daily timeframe");
        println!("   → May help, but still not reliable enough");
    } else {
        println!("❌ NO improvement from daily timeframe");
        println!("   → Timeframe is not the issue");
        println!("   → ADX itself cannot detect these regimes");
    }

    println!("\n═══════════════════════════════════════════════════════\n");

    Ok(())
}
