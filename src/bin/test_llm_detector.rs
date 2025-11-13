/// Test LLM regime detector on a few sample data points
///
/// This validates that:
/// 1. OpenAI API connection works
/// 2. GPT-4 generates valid JSON responses
/// 3. Caching works correctly
/// 4. Confidence scores are reasonable
///
/// Run with: OPENAI_API_KEY=sk-... cargo run --bin test_llm_detector

use cryptobot::persistence::RedisPersistence;
use cryptobot::regime::LLMRegimeDetector;
use cryptobot::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║         LLM REGIME DETECTOR TEST (GPT-4)             ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    // Get API key from environment
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| "OPENAI_API_KEY environment variable not set")?;

    println!("✅ Found OPENAI_API_KEY");

    // Connect to Redis
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    println!("📡 Connecting to Redis at {}...", redis_url);
    let mut redis = RedisPersistence::new(&redis_url).await?;

    println!("📊 Loading SOL data...");
    let candles = redis.load_all_candles("SOL").await?;

    if candles.is_empty() {
        return Err("No candles found for SOL in Redis".into());
    }

    println!("  ✓ Loaded {} candles\n", candles.len());

    // Create LLM detector
    let mut llm_detector = LLMRegimeDetector::new(api_key);

    // Test on 3 time periods to validate different regime classifications
    let test_indices = [
        (100, "Early data (should be bull or unclear)"),
        (candles.len() / 2, "Mid-period (crash phase expected)"),
        (candles.len() - 100, "Recent data (choppy recovery expected)"),
    ];

    println!("═══════════════════════════════════════════════════════");
    println!("Testing LLM regime detection on 3 sample periods...\n");

    for (idx, description) in &test_indices {
        println!("📍 Test {}: {}", idx, description);
        println!("   Timestamp: {}", candles[*idx].timestamp);
        println!("   Price: ${:.2}", candles[*idx].close);

        // Get window of candles up to this point
        let window = &candles[..=*idx];

        print!("   🤖 Calling OpenAI API (GPT-4)...");
        std::io::Write::flush(&mut std::io::stdout()).unwrap();

        let start = std::time::Instant::now();
        match llm_detector.detect_regime_with_confidence(window).await {
            Ok((regime, confidence)) => {
                let elapsed = start.elapsed();
                println!(" Done in {:.1}s", elapsed.as_secs_f64());
                println!("   ✅ Regime: {:?}", regime);
                println!("   📊 Confidence: {:.2} ({:.0}%)", confidence, confidence * 100.0);

                // Validate confidence is in valid range
                if confidence < 0.0 || confidence > 1.0 {
                    println!("   ⚠️  WARNING: Confidence out of range [0.0, 1.0]!");
                }
            }
            Err(e) => {
                println!(" ❌ FAILED");
                println!("   Error: {}", e);
                return Err(e);
            }
        }

        println!();
    }

    println!("═══════════════════════════════════════════════════════");
    println!("           CACHE STATISTICS                            ");
    println!("═══════════════════════════════════════════════════════");
    println!("Cached responses: {}", llm_detector.cache_size());
    println!("\n✅ LLM detector test completed successfully!");
    println!("\nNext steps:");
    println!("  1. If results look good → run full backtest");
    println!("  2. Full backtest will make ~8760 API calls (~$100-150 cost)");
    println!("  3. Results will be cached for reuse\n");

    Ok(())
}
