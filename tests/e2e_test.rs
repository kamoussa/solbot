use cryptobot::*;
use cryptobot::api::{DexScreenerClient, JupiterClient};
use cryptobot::indicators::{calculate_rsi, calculate_sma};
use cryptobot::risk::{CircuitBreakers, TradingState};

#[tokio::test]
async fn test_e2e_workflow() {
    // Initialize logging
    let _ = tracing_subscriber::fmt::try_init();

    println!("=== Starting E2E Test ===\n");

    // 1. Test DexScreener API
    println!("1. Testing DexScreener API...");
    let dex_client = DexScreenerClient::new();
    let sol_mint = "So11111111111111111111111111111111111111112";

    let price_data = dex_client.get_price(sol_mint).await;
    assert!(price_data.is_ok(), "DexScreener API failed");

    let price = price_data.unwrap();
    println!("   ✓ SOL Price: ${:.2}", price.price);
    println!("   ✓ 24h Volume: ${:.0}", price.volume_24h);
    assert!(price.price > 0.0);
    assert_eq!(price.token, "SOL");

    // 2. Test Jupiter API
    println!("\n2. Testing Jupiter API...");
    let jup_client = JupiterClient::new();
    let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    let amount = 1_000_000_000; // 1 SOL

    let quote = jup_client.get_quote(sol_mint, usdc_mint, amount, 50).await;
    assert!(quote.is_ok(), "Jupiter API failed");

    let quote = quote.unwrap();
    println!("   ✓ Quote: 1 SOL = {:.2} USDC", quote.price);
    println!("   ✓ Price Impact: {:.4}%", quote.price_impact_pct);
    assert!(quote.price > 0.0);

    // 3. Test Technical Indicators
    println!("\n3. Testing Technical Indicators...");
    let prices = vec![
        100.0, 102.0, 101.0, 103.0, 105.0, 104.0, 106.0, 108.0,
        107.0, 109.0, 111.0, 110.0, 112.0, 114.0, 113.0,
    ];

    let rsi = calculate_rsi(&prices, 14);
    assert!(rsi.is_some(), "RSI calculation failed");
    println!("   ✓ RSI(14): {:.2}", rsi.unwrap());

    let sma = calculate_sma(&prices, 10);
    assert!(sma.is_some(), "SMA calculation failed");
    println!("   ✓ SMA(10): {:.2}", sma.unwrap());

    // 4. Test Circuit Breakers
    println!("\n4. Testing Circuit Breakers...");
    let breakers = CircuitBreakers::default();
    let mut state = TradingState::new(10000.0);

    // Test normal operation
    let result = breakers.check(&state);
    assert!(result.is_ok(), "Circuit breaker triggered on healthy state");
    println!("   ✓ Normal state: OK");

    // Test daily loss limit
    state.daily_pnl = -600.0; // -6%
    state.portfolio_value = 9400.0;
    let result = breakers.check(&state);
    assert!(result.is_err(), "Circuit breaker should trip on -6% daily loss");
    println!("   ✓ Daily loss limit: Triggered correctly");

    // 5. Simulate a simple trading decision
    println!("\n5. Simulating Trading Decision...");

    // Calculate current indicators
    let current_rsi = rsi.unwrap();
    let current_price = prices.last().unwrap();

    println!("   Current Price: ${:.2}", current_price);
    println!("   Current RSI: {:.2}", current_rsi);

    // Simple momentum strategy
    let signal = if current_rsi < 30.0 {
        "BUY (Oversold)"
    } else if current_rsi > 70.0 {
        "SELL (Overbought)"
    } else {
        "HOLD (Neutral)"
    };

    println!("   Signal: {}", signal);

    // 6. Test Position Creation
    println!("\n6. Testing Position Management...");
    use cryptobot::models::{Position, PositionStatus};
    use uuid::Uuid;
    use chrono::Utc;

    let position = Position {
        id: Uuid::new_v4(),
        token: "SOL".to_string(),
        entry_price: price.price,
        quantity: 10.0,
        entry_time: Utc::now(),
        stop_loss: price.price * 0.92, // -8% stop loss
        take_profit: None,
        status: PositionStatus::Open,
    };

    println!("   ✓ Position Created:");
    println!("     Entry: ${:.2}", position.entry_price);
    println!("     Quantity: {}", position.quantity);
    println!("     Stop Loss: ${:.2}", position.stop_loss);
    println!("     Value: ${:.2}", position.entry_price * position.quantity);

    println!("\n=== E2E Test Complete ✅ ===");
}

#[tokio::test]
async fn test_api_comparison() {
    println!("\n=== Comparing DexScreener vs Jupiter Prices ===\n");

    let dex_client = DexScreenerClient::new();
    let jup_client = JupiterClient::new();

    let sol_mint = "So11111111111111111111111111111111111111112";
    let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

    // Get price from DexScreener
    let dex_price = dex_client.get_price(sol_mint).await;
    assert!(dex_price.is_ok());
    let dex_price = dex_price.unwrap().price;

    // Get price from Jupiter (1 SOL -> USDC)
    let jup_quote = jup_client.get_quote(sol_mint, usdc_mint, 1_000_000_000, 50).await;
    assert!(jup_quote.is_ok());
    let quote = jup_quote.unwrap();

    // Jupiter returns raw units: out_amount (USDC micro-units) / in_amount (lamports)
    // SOL has 9 decimals, USDC has 6 decimals
    // To get USDC per SOL: (out_amount / 10^6) / (in_amount / 10^9) = (out_amount / in_amount) * 1000
    let jup_price = quote.price * 1000.0;

    println!("DexScreener SOL Price: ${:.2}", dex_price);
    println!("Jupiter Quote (1 SOL): ${:.2} USDC", jup_price);
    println!("  (Raw ratio: {:.6})", quote.price);

    // Prices should be within 5% of each other (rough sanity check)
    let price_diff_pct = ((dex_price - jup_price).abs() / dex_price) * 100.0;
    println!("Price Difference: {:.2}%", price_diff_pct);

    assert!(
        price_diff_pct < 5.0,
        "Price difference too large: {:.2}%",
        price_diff_pct
    );

    println!("\n✅ Prices are consistent\n");
}
