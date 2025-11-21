/// Minimal DCA backtest test
/// Run with: RUST_LOG=cryptobot=info cargo run --bin test_dca_minimal

use cryptobot::backtest::runner::BacktestRunner;
use cryptobot::backtest::synthetic::{MarketScenario, SyntheticDataGenerator};
use cryptobot::risk::CircuitBreakers;
use cryptobot::strategy::dca::DCAStrategy;
use cryptobot::strategy::Strategy;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Init logging
    tracing_subscriber::fmt()
        .with_env_filter("cryptobot=debug")
        .try_init()
        .ok();

    println!("\n🔬 Minimal DCA Backtest Test\n");

    // Generate 8760 hourly candles (1 year)
    let mut gen = SyntheticDataGenerator::new(42);
    let candles = gen.generate(MarketScenario::Uptrend, 8760, 60);  // Uptrend market, hourly

    println!("Generated {} hourly candles", candles.len());
    println!("First candle: {}", candles[0].timestamp);
    println!("Last candle:  {}\n", candles.last().unwrap().timestamp);

    // Create DCA strategy (weekly = 168 hours)
    let dca = DCAStrategy::weekly();
    println!("Testing DCA strategy: {}", dca.name());
    println!("Expected trades: ~52 (8760 hours / 168 hours)\n");

    // Run backtest
    let circuit_breakers = CircuitBreakers::default();
    let runner = BacktestRunner::new(10000.0, circuit_breakers);

    let metrics = runner.run(&dca, candles, "TEST", 60, 0.0)?;

    // Print full metrics report (shows accumulation count)
    metrics.print_report();

    // Verify accumulation count
    if metrics.accumulation_count >= 51 && metrics.accumulation_count <= 53 {
        println!("\n✅ SUCCESS! DCA made {} accumulations as expected (~52)", metrics.accumulation_count);
    } else {
        println!("\n❌ FAIL! DCA made {} accumulations, expected ~52", metrics.accumulation_count);
    }

    Ok(())
}
