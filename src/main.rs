use cryptobot::execution::PriceFeedManager;
use cryptobot::models::Token;
use cryptobot::Result;
use tokio::time::{interval, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("cryptobot=info")
        .init();

    tracing::info!("CryptoBot starting - KISS mode (Keep It Simple Stupid)");

    // Start with simple tokens - just SOL and JUP
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

    // Initialize price feed manager (keep 100 snapshots per token)
    let mut price_manager = PriceFeedManager::new(tokens.clone(), 100);

    tracing::info!("Watching {} tokens:", tokens.len());
    for token in &tokens {
        tracing::info!("  - {} ({})", token.symbol, token.name);
    }

    // Main event loop - fetch prices every 5 minutes
    let mut ticker = interval(Duration::from_secs(5)); // 5 minutes

    loop {
        ticker.tick().await;

        tracing::info!("=== Tick ===");

        // Fetch all prices
        let results = price_manager.fetch_all().await;

        // Process results - just log prices for now
        for (i, result) in results.iter().enumerate() {
            let token = &tokens[i];

            match result {
                Ok(snapshot) => {
                    // Get historical data count
                    let history_count = price_manager.buffer()
                        .get_candles(&token.symbol)
                        .map(|c| c.len())
                        .unwrap_or(0);

                    tracing::info!(
                        "{}: ${:.4} ({} snapshots collected)",
                        token.symbol,
                        snapshot.price,
                        history_count
                    );
                }
                Err(e) => {
                    tracing::error!("{}: Failed to fetch price: {}", token.symbol, e);
                }
            }
        }

        tracing::info!("=== End Tick ===\n");
    }
}
