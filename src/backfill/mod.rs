pub mod converter;
pub mod validator;

use crate::api::CoinGeckoClient;
use crate::persistence::RedisPersistence;
use crate::Result;

pub use converter::CandleConverter;
pub use validator::CandleValidator;

/// Statistics from a backfill operation
#[derive(Debug, Clone, PartialEq)]
pub struct BackfillStats {
    pub fetched_points: usize,
    pub converted_candles: usize,
    pub skipped_existing: usize,
    pub stored_new: usize,
    pub validation_failures: usize,
}

/// Backfill historical data for a token
pub async fn backfill_token(
    symbol: &str,
    mint_address: &str,
    days: u32,
    force_overwrite: bool,
    coingecko: &CoinGeckoClient,
    persistence: &mut RedisPersistence,
) -> Result<BackfillStats> {
    tracing::info!("Backfilling {} days of data for {}", days, symbol);

    let mut stats = BackfillStats {
        fetched_points: 0,
        converted_candles: 0,
        skipped_existing: 0,
        stored_new: 0,
        validation_failures: 0,
    };

    // Find CoinGecko coin_id
    let coin_id = coingecko.find_coin_id(symbol, mint_address).await?;
    tracing::debug!("Found coin_id: {} for {}", coin_id, symbol);

    // Fetch market chart data
    let market_data = coingecko.get_market_chart(&coin_id, days).await?;
    stats.fetched_points = market_data.prices.len();
    tracing::debug!("Fetched {} price points", stats.fetched_points);

    // Auto-detect granularity based on CoinGecko's API behavior:
    // - 1 day: 5-minute candles
    // - 2-90 days: hourly candles
    // - 90+ days: daily candles
    let (converter, granularity_desc) = if days == 1 {
        (CandleConverter::new(), "5-minute")
    } else if days <= 90 {
        (CandleConverter::for_hourly(), "hourly")
    } else {
        tracing::info!(
            "Backfill for {} days will use daily candles (suitable for long-term backtesting)",
            days
        );
        (CandleConverter::for_daily(), "daily")
    };

    tracing::info!(
        "Converting {} price points to {} candles (days={}, granularity={})",
        stats.fetched_points,
        granularity_desc,
        days,
        granularity_desc
    );

    // Convert to candles
    let candles = converter.convert_to_candles(symbol, market_data)?;
    stats.converted_candles = candles.len();
    tracing::debug!(
        "Converted to {} {} candles",
        stats.converted_candles,
        granularity_desc
    );

    // Get existing timestamps for overlap detection (if not forcing overwrite)
    let existing_timestamps = if !force_overwrite {
        persistence
            .load_candles(symbol, days as u64 * 24)
            .await?
            .into_iter()
            .map(|c| c.timestamp)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    // Validate and store candles
    let validator = CandleValidator::new();
    let mut candles_to_store = Vec::new();

    for candle in candles {
        // Validate
        if let Err(e) = validator.validate(&candle) {
            tracing::warn!(
                "Validation failed for candle at {}: {}",
                candle.timestamp,
                e
            );
            stats.validation_failures += 1;
            continue;
        }

        // Check for overlap (within 60 seconds)
        if !force_overwrite {
            let is_duplicate = existing_timestamps
                .iter()
                .any(|&ts| (candle.timestamp - ts).num_seconds().abs() < 60);

            if is_duplicate {
                stats.skipped_existing += 1;
                continue;
            }
        }

        candles_to_store.push(candle);
    }

    // Store candles
    if !candles_to_store.is_empty() {
        persistence.save_candles(symbol, &candles_to_store).await?;
        stats.stored_new = candles_to_store.len();
        tracing::info!(
            "✓ Stored {} new candles for {} (skipped {}, failed validation {})",
            stats.stored_new,
            symbol,
            stats.skipped_existing,
            stats.validation_failures
        );
    } else {
        tracing::info!("No new candles to store for {}", symbol);
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration test: Full backfill flow for SOL
    #[tokio::test]
    #[ignore] // Requires COINGECKO_API_KEY and Redis
    async fn test_backfill_sol_integration() {
        let api_key = std::env::var("COINGECKO_API_KEY").expect("COINGECKO_API_KEY not set");
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

        // Initialize clients
        let coingecko = CoinGeckoClient::new(api_key).await.unwrap();
        let mut redis = RedisPersistence::new(&redis_url).await.unwrap();

        // Clear existing data
        let _ = redis.cleanup_old("TEST_SOL", 0).await;

        // Run backfill
        let stats = backfill_token(
            "TEST_SOL",
            "So11111111111111111111111111111111111111112",
            1, // 1 day
            false,
            &coingecko,
            &mut redis,
        )
        .await
        .unwrap();

        // Verify results
        assert!(
            stats.fetched_points > 100,
            "Should fetch at least 100 points"
        );
        assert!(
            stats.converted_candles > 100,
            "Should convert at least 100 candles"
        );
        assert!(stats.stored_new > 0, "Should store some candles");
        assert_eq!(
            stats.validation_failures, 0,
            "Should have no validation failures"
        );

        // Verify data in Redis
        let loaded = redis.load_candles("TEST_SOL", 24).await.unwrap();
        assert!(loaded.len() > 0, "Should have data in Redis");

        // Cleanup
        let _ = redis.cleanup_old("TEST_SOL", 0).await;
    }

    /// Integration test: Backfill with overlap detection
    #[tokio::test]
    #[ignore] // Requires COINGECKO_API_KEY and Redis
    async fn test_backfill_overlap_detection() {
        let api_key = std::env::var("COINGECKO_API_KEY").expect("COINGECKO_API_KEY not set");
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

        let coingecko = CoinGeckoClient::new(api_key).await.unwrap();
        let mut redis = RedisPersistence::new(&redis_url).await.unwrap();

        // Clear and run first backfill
        let _ = redis.cleanup_old("TEST_OVERLAP", 0).await;
        let stats1 = backfill_token(
            "TEST_OVERLAP",
            "So11111111111111111111111111111111111111112",
            1,
            false,
            &coingecko,
            &mut redis,
        )
        .await
        .unwrap();

        assert!(stats1.stored_new > 0, "First backfill should store data");

        // Run second backfill (should skip existing)
        let stats2 = backfill_token(
            "TEST_OVERLAP",
            "So11111111111111111111111111111111111111112",
            1,
            false,
            &coingecko,
            &mut redis,
        )
        .await
        .unwrap();

        assert!(stats2.skipped_existing > 0, "Should skip existing candles");
        assert_eq!(stats2.stored_new, 0, "Should not store duplicate data");

        // Cleanup
        let _ = redis.cleanup_old("TEST_OVERLAP", 0).await;
    }

    /// Integration test: Non-existent token error handling
    #[tokio::test]
    #[ignore] // Requires COINGECKO_API_KEY and Redis
    async fn test_backfill_nonexistent_token() {
        let api_key = std::env::var("COINGECKO_API_KEY").expect("COINGECKO_API_KEY not set");
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

        let coingecko = CoinGeckoClient::new(api_key).await.unwrap();
        let mut redis = RedisPersistence::new(&redis_url).await.unwrap();

        // Try to backfill non-existent token
        let result = backfill_token(
            "FAKE",
            "FakeAddress12345678901234567890123456",
            1,
            false,
            &coingecko,
            &mut redis,
        )
        .await;

        assert!(result.is_err(), "Should fail for non-existent token");
        assert!(
            result.unwrap_err().to_string().contains("not found"),
            "Error should mention token not found"
        );
    }

    /// Integration test: Force overwrite flag
    #[tokio::test]
    #[ignore] // Requires COINGECKO_API_KEY and Redis
    async fn test_backfill_force_overwrite() {
        let api_key = std::env::var("COINGECKO_API_KEY").expect("COINGECKO_API_KEY not set");
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

        let coingecko = CoinGeckoClient::new(api_key).await.unwrap();
        let mut redis = RedisPersistence::new(&redis_url).await.unwrap();

        // Clear and run first backfill
        let _ = redis.cleanup_old("TEST_FORCE", 0).await;
        let stats1 = backfill_token(
            "TEST_FORCE",
            "So11111111111111111111111111111111111111112",
            1,
            false,
            &coingecko,
            &mut redis,
        )
        .await
        .unwrap();

        let first_stored = stats1.stored_new;
        assert!(first_stored > 0, "First backfill should store data");

        // Run with force overwrite
        let stats2 = backfill_token(
            "TEST_FORCE",
            "So11111111111111111111111111111111111111112",
            1,
            true, // Force overwrite
            &coingecko,
            &mut redis,
        )
        .await
        .unwrap();

        // With force, it should store again (duplicates allowed)
        assert!(stats2.stored_new > 0, "Force overwrite should store data");
        assert_eq!(
            stats2.skipped_existing, 0,
            "Should not skip with force flag"
        );

        // Cleanup
        let _ = redis.cleanup_old("TEST_FORCE", 0).await;
    }
}
