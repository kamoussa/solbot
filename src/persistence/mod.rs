use crate::models::Candle;
use crate::Result;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use tokio::time::{timeout, Duration};

/// Simple snapshot for Redis storage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSnapshot {
    price: f64,
    volume: f64,
    timestamp: DateTime<Utc>,
}

/// Redis persistence for price snapshots
///
/// Uses sorted sets with timestamps as scores for efficient time-range queries
pub struct RedisPersistence {
    conn: ConnectionManager,
}

impl RedisPersistence {
    /// Connect to Redis
    ///
    /// # Arguments
    /// * `redis_url` - Redis connection URL (e.g., "redis://127.0.0.1:6379")
    ///
    /// # Example
    /// ```
    /// let persistence = RedisPersistence::new("redis://127.0.0.1:6379").await?;
    /// ```
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = Client::open(redis_url)?;

        // Add 5 second timeout to connection attempt
        let conn = timeout(Duration::from_secs(5), ConnectionManager::new(client))
            .await
            .map_err(|_| "Redis connection timeout after 5 seconds")??;

        tracing::info!("Connected to Redis at {}", redis_url);

        Ok(Self { conn })
    }

    /// Save candles to Redis
    ///
    /// Stores in sorted set: `snapshots:{token}` with timestamp as score
    pub async fn save_candles(&mut self, token: &str, candles: &[Candle]) -> Result<()> {
        let key = format!("snapshots:{}", token);

        for candle in candles {
            let snapshot = StoredSnapshot {
                price: candle.close,
                volume: candle.volume,
                timestamp: candle.timestamp,
            };

            let value = serde_json::to_string(&snapshot)?;
            let score = candle.timestamp.timestamp() as f64;

            self.conn.zadd::<_, _, _, ()>(&key, value, score).await?;
        }

        tracing::debug!("Saved {} candles for {} to Redis", candles.len(), token);

        Ok(())
    }

    /// Load recent candles from Redis
    ///
    /// # Arguments
    /// * `token` - Token symbol
    /// * `hours_back` - How many hours of history to load
    ///
    /// # Returns
    /// Vec of candles sorted by timestamp (oldest first)
    pub async fn load_candles(&mut self, token: &str, hours_back: u64) -> Result<Vec<Candle>> {
        let key = format!("snapshots:{}", token);

        // Calculate cutoff timestamp
        let cutoff = Utc::now() - chrono::Duration::hours(hours_back as i64);
        let min_score = cutoff.timestamp() as f64;

        // Get all snapshots after cutoff
        let results: Vec<String> = self.conn
            .zrangebyscore(&key, min_score, "+inf")
            .await?;

        let mut candles = Vec::new();

        for json_str in results {
            let snapshot: StoredSnapshot = serde_json::from_str(&json_str)?;

            candles.push(Candle {
                token: token.to_string(),
                timestamp: snapshot.timestamp,
                open: snapshot.price,
                high: snapshot.price,
                low: snapshot.price,
                close: snapshot.price,
                volume: snapshot.volume,
            });
        }

        tracing::info!("Loaded {} historical candles for {} from Redis", candles.len(), token);

        Ok(candles)
    }

    /// Clean up old snapshots to prevent unbounded growth
    ///
    /// Removes snapshots older than specified hours
    pub async fn cleanup_old(&mut self, token: &str, keep_hours: u64) -> Result<usize> {
        let key = format!("snapshots:{}", token);

        let cutoff = Utc::now() - chrono::Duration::hours(keep_hours as i64);
        let max_score = cutoff.timestamp() as f64;

        let removed: usize = self.conn
            .zrembyscore(&key, "-inf", max_score)
            .await?;

        if removed > 0 {
            tracing::debug!("Cleaned up {} old snapshots for {}", removed, token);
        }

        Ok(removed)
    }

    /// Get count of stored snapshots for a token
    pub async fn count_snapshots(&mut self, token: &str) -> Result<usize> {
        let key = format!("snapshots:{}", token);
        let count: usize = self.conn.zcard(&key).await?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_candle(token: &str, hours_ago: i64, price: f64) -> Candle {
        Candle {
            token: token.to_string(),
            timestamp: Utc::now() - chrono::Duration::hours(hours_ago),
            open: price,
            high: price,
            low: price,
            close: price,
            volume: price * 1000.0,
        }
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    async fn test_connection_timeout() {
        // Try to connect to non-existent Redis
        let result = RedisPersistence::new("redis://192.0.2.1:6379").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    async fn test_save_and_load_single_candle() {
        let mut persistence = RedisPersistence::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to connect to Redis");

        // Clean up first
        let _ = persistence.cleanup_old("TEST_SINGLE", 0).await;

        let candle = create_test_candle("TEST_SINGLE", 1, 100.0);
        persistence.save_candles("TEST_SINGLE", &[candle.clone()]).await.unwrap();

        let loaded = persistence.load_candles("TEST_SINGLE", 24).await.unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].close, 100.0);
        assert_eq!(loaded[0].token, "TEST_SINGLE");

        // Cleanup
        let _ = persistence.cleanup_old("TEST_SINGLE", 0).await;
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    async fn test_save_multiple_candles() {
        let mut persistence = RedisPersistence::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to connect to Redis");

        // Clean up first
        let _ = persistence.cleanup_old("TEST_MULTI", 0).await;

        let candles = vec![
            create_test_candle("TEST_MULTI", 3, 100.0),
            create_test_candle("TEST_MULTI", 2, 101.0),
            create_test_candle("TEST_MULTI", 1, 102.0),
        ];

        persistence.save_candles("TEST_MULTI", &candles).await.unwrap();

        let loaded = persistence.load_candles("TEST_MULTI", 24).await.unwrap();

        assert_eq!(loaded.len(), 3);
        // Should be sorted oldest first
        assert_eq!(loaded[0].close, 100.0);
        assert_eq!(loaded[1].close, 101.0);
        assert_eq!(loaded[2].close, 102.0);

        // Cleanup
        let _ = persistence.cleanup_old("TEST_MULTI", 0).await;
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    async fn test_load_with_time_filter() {
        let mut persistence = RedisPersistence::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to connect to Redis");

        // Clean up first
        let _ = persistence.cleanup_old("TEST_FILTER", 0).await;

        let candles = vec![
            create_test_candle("TEST_FILTER", 48, 100.0), // 2 days ago
            create_test_candle("TEST_FILTER", 12, 101.0), // 12 hours ago
            create_test_candle("TEST_FILTER", 1, 102.0),  // 1 hour ago
        ];

        persistence.save_candles("TEST_FILTER", &candles).await.unwrap();

        // Load only last 24 hours
        let loaded = persistence.load_candles("TEST_FILTER", 24).await.unwrap();

        // Should only get the 2 recent ones (12h and 1h ago)
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].close, 101.0);
        assert_eq!(loaded[1].close, 102.0);

        // Cleanup
        let _ = persistence.cleanup_old("TEST_FILTER", 0).await;
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    async fn test_cleanup_old_data() {
        let mut persistence = RedisPersistence::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to connect to Redis");

        // Clean up first
        let _ = persistence.cleanup_old("TEST_CLEANUP", 0).await;

        let candles = vec![
            create_test_candle("TEST_CLEANUP", 72, 100.0), // 3 days ago
            create_test_candle("TEST_CLEANUP", 12, 101.0), // 12 hours ago
        ];

        persistence.save_candles("TEST_CLEANUP", &candles).await.unwrap();

        // Cleanup anything older than 24 hours
        let removed = persistence.cleanup_old("TEST_CLEANUP", 24).await.unwrap();
        assert_eq!(removed, 1);

        // Verify only recent data remains
        let loaded = persistence.load_candles("TEST_CLEANUP", 72).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].close, 101.0);

        // Cleanup
        let _ = persistence.cleanup_old("TEST_CLEANUP", 0).await;
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    async fn test_count_snapshots() {
        let mut persistence = RedisPersistence::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to connect to Redis");

        // Clean up first
        let _ = persistence.cleanup_old("TEST_COUNT", 0).await;

        let count_before = persistence.count_snapshots("TEST_COUNT").await.unwrap();
        assert_eq!(count_before, 0);

        let candles = vec![
            create_test_candle("TEST_COUNT", 3, 100.0),
            create_test_candle("TEST_COUNT", 2, 101.0),
            create_test_candle("TEST_COUNT", 1, 102.0),
        ];

        persistence.save_candles("TEST_COUNT", &candles).await.unwrap();

        let count_after = persistence.count_snapshots("TEST_COUNT").await.unwrap();
        assert_eq!(count_after, 3);

        // Cleanup
        let _ = persistence.cleanup_old("TEST_COUNT", 0).await;
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    async fn test_empty_token() {
        let mut persistence = RedisPersistence::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to connect to Redis");

        let loaded = persistence.load_candles("NONEXISTENT_TOKEN", 24).await.unwrap();
        assert_eq!(loaded.len(), 0);

        let count = persistence.count_snapshots("NONEXISTENT_TOKEN").await.unwrap();
        assert_eq!(count, 0);
    }
}
