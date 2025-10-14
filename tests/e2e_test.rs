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

#[tokio::test]
#[ignore] // Requires Redis running
async fn test_e2e_persistence_workflow() {
    use cryptobot::execution::PriceFeedManager;
    use cryptobot::persistence::RedisPersistence;
    use cryptobot::models::Token;

    println!("\n=== Testing E2E Persistence Workflow ===\n");

    // 1. Setup
    println!("1. Setting up Redis and PriceFeedManager...");
    let redis_url = "redis://127.0.0.1:6379";
    let mut persistence = RedisPersistence::new(redis_url)
        .await
        .expect("Redis should be running");

    // Use test token to avoid interference with real data
    let test_token_symbol = "E2E_TEST_SOL";
    let _ = persistence.cleanup_old(test_token_symbol, 0).await;

    let tokens = vec![Token {
        symbol: "SOL".to_string(), // Use real SOL for API
        mint_address: "So11111111111111111111111111111111111111112".to_string(),
        name: "Solana".to_string(),
        decimals: 9,
    }];

    let mut price_manager = PriceFeedManager::new(tokens.clone(), 100);
    println!("   ✓ Setup complete");

    // 2. Fetch real prices
    println!("\n2. Fetching real prices from DexScreener...");
    let results = price_manager.fetch_all().await;
    assert_eq!(results.len(), 1);
    assert!(results[0].is_ok(), "Price fetch should succeed");

    let snapshot = results[0].as_ref().unwrap();
    println!("   ✓ Fetched SOL price: ${:.2}", snapshot.price);

    // 3. Save to Redis (using test token symbol)
    println!("\n3. Saving snapshots to Redis...");
    let mut candles = price_manager.buffer()
        .get_candles("SOL")
        .expect("Should have candles");
    assert_eq!(candles.len(), 1);

    // Change token symbol to test version for isolation
    for candle in &mut candles {
        candle.token = test_token_symbol.to_string();
    }

    persistence.save_candles(test_token_symbol, &candles)
        .await
        .expect("Should save to Redis");

    let count = persistence.count_snapshots(test_token_symbol).await.unwrap();
    println!("   ✓ Saved {} snapshot(s) to Redis", count);
    assert_eq!(count, 1, "Should have exactly 1 snapshot");

    // 4. Simulate bot restart - create new manager
    println!("\n4. Simulating bot restart...");
    let mut new_manager = PriceFeedManager::new(tokens.clone(), 100);

    // Load historical data from Redis
    println!("   Loading historical data from Redis...");
    let historical = persistence.load_candles(test_token_symbol, 24)
        .await
        .expect("Should load from Redis");

    assert_eq!(historical.len(), 1, "Should have exactly 1 historical candle");
    println!("   ✓ Loaded {} historical snapshot(s)", historical.len());

    // Add historical data to new manager (restore original token symbol)
    for mut candle in historical.clone() {
        candle.token = "SOL".to_string();
        new_manager.buffer().add_candle(candle).unwrap();
    }

    // 5. Verify data persisted correctly
    println!("\n5. Verifying data persistence...");
    let restored_candles = new_manager.buffer()
        .get_candles("SOL")
        .expect("Should have restored candles");

    assert_eq!(restored_candles.len(), 1);
    assert_eq!(restored_candles[0].close, candles[0].close);
    println!("   ✓ Original price: ${:.2}", candles[0].close);
    println!("   ✓ Restored price: ${:.2}", restored_candles[0].close);
    println!("   ✓ Data integrity verified");

    // 6. Fetch another snapshot and verify accumulation
    println!("\n6. Fetching additional snapshot...");
    let results2 = new_manager.fetch_all().await;
    assert!(results2[0].is_ok());

    let all_candles = new_manager.buffer()
        .get_candles("SOL")
        .expect("Should have all candles");
    assert_eq!(all_candles.len(), 2, "Should have 1 restored + 1 new");
    println!("   ✓ Now have {} total snapshots", all_candles.len());

    // 7. Cleanup
    println!("\n7. Cleaning up test data...");
    let removed = persistence.cleanup_old(test_token_symbol, 0).await.unwrap();
    println!("   ✓ Cleaned up {} test snapshots", removed);

    println!("\n=== E2E Persistence Test Complete ✅ ===\n");
}

#[tokio::test]
#[ignore] // Requires Redis running
async fn test_e2e_full_bot_simulation() {
    use cryptobot::execution::PriceFeedManager;
    use cryptobot::persistence::RedisPersistence;
    use cryptobot::strategy::momentum::MomentumStrategy;
    use cryptobot::strategy::Strategy;
    use cryptobot::models::Token;

    println!("\n=== Testing Full Bot Simulation ===\n");

    // Setup
    let redis_url = "redis://127.0.0.1:6379";
    let mut persistence = RedisPersistence::new(redis_url)
        .await
        .expect("Redis should be running");

    let tokens = vec![
        Token {
            symbol: "SOL".to_string(),
            mint_address: "So11111111111111111111111111111111111111112".to_string(),
            name: "Solana".to_string(),
            decimals: 9,
        },
        Token {
            symbol: "JUP".to_string(),
            mint_address: "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN".to_string(),
            name: "Jupiter".to_string(),
            decimals: 6,
        },
    ];

    let mut price_manager = PriceFeedManager::new(tokens.clone(), 100);
    let strategy = MomentumStrategy::default();

    println!("1. Fetching prices for {} tokens...", tokens.len());

    // Fetch prices
    let results = price_manager.fetch_all().await;
    let mut success_count = 0;

    for (i, result) in results.iter().enumerate() {
        let token = &tokens[i];
        match result {
            Ok(snapshot) => {
                println!("   ✓ {}: ${:.4}", token.symbol, snapshot.price);
                success_count += 1;

                // Save to Redis
                if let Ok(candles) = price_manager.buffer().get_candles(&token.symbol) {
                    if let Some(latest) = candles.last() {
                        persistence.save_candles(&token.symbol, &[latest.clone()])
                            .await
                            .expect("Should save to Redis");
                    }
                }
            }
            Err(e) => {
                println!("   ✗ {}: Failed - {}", token.symbol, e);
            }
        }
    }

    assert!(success_count > 0, "At least one price fetch should succeed");

    // Try to generate signals (won't have enough data yet)
    println!("\n2. Checking signal generation...");
    for token in &tokens {
        if let Ok(candles) = price_manager.buffer().get_candles(&token.symbol) {
            if candles.len() >= strategy.samples_needed(30) {
                match strategy.generate_signal(&candles) {
                    Ok(signal) => {
                        println!("   ✓ {}: Signal = {:?}", token.symbol, signal);
                    }
                    Err(_) => {
                        println!("   ⧗ {}: Not enough data for signal", token.symbol);
                    }
                }
            } else {
                println!("   ⧗ {}: Collecting data ({}/{} needed)",
                    token.symbol,
                    candles.len(),
                    strategy.samples_needed(30)
                );
            }
        }
    }

    // Verify Redis persistence
    println!("\n3. Verifying Redis persistence...");
    for token in &tokens {
        let count = persistence.count_snapshots(&token.symbol).await.unwrap();
        if count > 0 {
            println!("   ✓ {}: {} snapshot(s) in Redis", token.symbol, count);
        }
    }

    println!("\n=== Full Bot Simulation Complete ✅ ===\n");
}
