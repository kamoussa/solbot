use chrono::DateTime;
use std::collections::BTreeMap;

use crate::api::MarketChartData;
use crate::models::Candle;
use crate::Result;

const BUCKET_INTERVAL_5MIN: i64 = 300; // 5 minutes
const BUCKET_INTERVAL_1HOUR: i64 = 3600; // 1 hour

/// Converts irregular price points from CoinGecko to uniform OHLC candles
///
/// Supports multiple granularities:
/// - 5-minute candles (for 1-day CoinGecko data)
/// - Hourly candles (for 2-90 day CoinGecko data)
pub struct CandleConverter {
    interval_secs: i64,
}

impl CandleConverter {
    /// Create a new converter with default 5-minute interval
    ///
    /// Use this for CoinGecko 1-day backfills which return 5-minute data
    pub fn new() -> Self {
        Self {
            interval_secs: BUCKET_INTERVAL_5MIN,
        }
    }

    /// Create a converter with custom interval
    ///
    /// # Arguments
    /// * `interval_secs` - Bucket size in seconds (e.g., 300 for 5min, 3600 for 1hour)
    pub fn new_with_interval(interval_secs: i64) -> Self {
        Self { interval_secs }
    }

    /// Create a converter for hourly candles
    ///
    /// Use this for CoinGecko 2-90 day backfills which return hourly data
    pub fn for_hourly() -> Self {
        Self {
            interval_secs: BUCKET_INTERVAL_1HOUR,
        }
    }

    /// Get the interval in seconds for this converter
    pub fn interval_secs(&self) -> i64 {
        self.interval_secs
    }

    /// Convert market chart data to OHLC candles with gap filling
    ///
    /// CoinGecko's 1-day API returns irregular/sparse data with large gaps.
    /// This function fills gaps > 5 minutes by interpolating candles using the last known price and volume.
    pub fn convert_to_candles(&self, symbol: &str, data: MarketChartData) -> Result<Vec<Candle>> {
        if data.prices.is_empty() {
            return Ok(Vec::new());
        }

        // Sort and deduplicate timestamps
        let prices = self.sort_and_dedupe(data.prices);
        let volumes = self.sort_and_dedupe(data.total_volumes);

        // Bucket into windows based on desired candle interval
        let price_buckets = self.bucket_into_windows(prices);
        let volume_buckets = self.bucket_into_windows(volumes);

        if price_buckets.is_empty() {
            return Ok(Vec::new());
        }

        // Convert buckets to candles and fill gaps
        let mut candles = Vec::new();
        let bucket_timestamps: Vec<i64> = price_buckets.keys().copied().collect();

        for i in 0..bucket_timestamps.len() {
            let bucket_timestamp = bucket_timestamps[i];
            let bucket_prices = price_buckets.get(&bucket_timestamp).unwrap(); // Safe: we're iterating keys
            let bucket_volumes = volume_buckets.get(&bucket_timestamp);

            if bucket_prices.is_empty() {
                // Empty bucket shouldn't happen (bucket_into_windows only creates non-empty buckets)
                tracing::warn!("Skipping empty bucket at timestamp {}", bucket_timestamp);
                continue;
            }

            // Get volume for this candle (use last volume in bucket, or 0.0 if no volume data)
            let volume = bucket_volumes
                .and_then(|v| v.last().copied())
                .unwrap_or(0.0);

            // Add real candle
            let candle =
                self.synthesize_candle(symbol, bucket_timestamp, bucket_prices.clone(), volume);
            candles.push(candle);

            // Fill gaps to next bucket (if any)
            if i + 1 < bucket_timestamps.len() {
                let filled = self.fill_gap(
                    symbol,
                    bucket_timestamp,
                    bucket_timestamps[i + 1],
                    *bucket_prices.last().unwrap(), // Safe: we checked !is_empty above
                    volume,                         // Carry forward last known volume
                );
                candles.extend(filled);
            }
        }

        Ok(candles)
    }

    /// Fill gaps between two timestamps with interpolated candles
    fn fill_gap(
        &self,
        symbol: &str,
        from: i64,
        to: i64,
        last_price: f64,
        last_volume: f64,
    ) -> Vec<Candle> {
        let gap = to - from;

        if gap <= self.interval_secs {
            return Vec::new(); // No gap to fill
        }

        let num_missing = ((gap / self.interval_secs) - 1) as usize;
        if num_missing > 0 {
            tracing::debug!(
                "Filling {} missing candles for {} ({}s gap from {} to {})",
                num_missing,
                symbol,
                gap,
                from,
                to
            );
        }

        let mut filled = Vec::new();
        let mut fill_timestamp = from + self.interval_secs;

        while fill_timestamp < to {
            let interpolated = self.synthesize_candle(
                symbol,
                fill_timestamp,
                vec![last_price], // Flat candle: O=H=L=C=last_price
                last_volume,      // Carry forward last known volume
            );
            filled.push(interpolated);
            fill_timestamp += self.interval_secs;
        }

        filled
    }

    /// Sort by timestamp and remove duplicates (keeping last value for each timestamp)
    fn sort_and_dedupe(&self, mut prices: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
        if prices.is_empty() {
            return prices;
        }

        // Sort by timestamp
        prices.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());

        // Deduplicate - keep last value for each timestamp
        let mut deduped = Vec::new();
        let mut last_timestamp = prices[0][0];
        let mut last_price = prices[0][1];

        for point in prices.iter().skip(1) {
            if point[0] == last_timestamp {
                // Duplicate timestamp, update price
                last_price = point[1];
            } else {
                // New timestamp, save previous
                deduped.push([last_timestamp, last_price]);
                last_timestamp = point[0];
                last_price = point[1];
            }
        }

        // Don't forget the last point
        deduped.push([last_timestamp, last_price]);

        deduped
    }

    /// Bucket price points into time windows aligned to interval boundaries
    fn bucket_into_windows(&self, prices: Vec<[f64; 2]>) -> BTreeMap<i64, Vec<f64>> {
        let mut buckets: BTreeMap<i64, Vec<f64>> = BTreeMap::new();

        for point in prices {
            let timestamp_ms = point[0] as i64;
            let price = point[1];

            // Convert to seconds and align to bucket boundary
            let timestamp_secs = timestamp_ms / 1000;
            let bucket = (timestamp_secs / self.interval_secs) * self.interval_secs;

            buckets.entry(bucket).or_insert_with(Vec::new).push(price);
        }

        buckets
    }

    /// Synthesize a candle from a bucket of prices and volume
    fn synthesize_candle(
        &self,
        symbol: &str,
        bucket_timestamp: i64,
        prices: Vec<f64>,
        volume: f64,
    ) -> Candle {
        let open = prices[0];
        let close = *prices.last().unwrap();
        let high = prices.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        let low = prices.iter().fold(f64::INFINITY, |a, &b| a.min(b));

        Candle {
            token: symbol.to_string(),
            timestamp: DateTime::from_timestamp(bucket_timestamp, 0).unwrap(),
            open,
            high,
            low,
            close,
            // Use 24h rolling volume from CoinGecko (same as DexScreener uses in production)
            volume,
        }
    }
}

impl Default for CandleConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_empty_data() {
        let converter = CandleConverter::new();
        let data = MarketChartData {
            prices: vec![],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();
        assert_eq!(candles.len(), 0);
    }

    #[test]
    fn test_convert_single_point() {
        let converter = CandleConverter::new();
        let data = MarketChartData {
            prices: vec![[1000000000.0, 100.0]], // timestamp in ms
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();
        assert_eq!(candles.len(), 1);

        let candle = &candles[0];
        assert_eq!(candle.open, 100.0);
        assert_eq!(candle.high, 100.0);
        assert_eq!(candle.low, 100.0);
        assert_eq!(candle.close, 100.0);
        assert_eq!(candle.volume, 0.0);
    }

    #[test]
    fn test_convert_uniform_data() {
        let converter = CandleConverter::new();

        // Perfect 5-min intervals (300,000 ms = 300 seconds)
        let data = MarketChartData {
            prices: vec![[0.0, 100.0], [300000.0, 101.0], [600000.0, 102.0]],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();
        assert_eq!(candles.len(), 3);

        assert_eq!(candles[0].close, 100.0);
        assert_eq!(candles[1].close, 101.0);
        assert_eq!(candles[2].close, 102.0);
    }

    #[test]
    fn test_convert_irregular_data() {
        let converter = CandleConverter::new();

        // Irregular intervals - multiple points in same bucket
        let data = MarketChartData {
            prices: vec![
                [0.0, 100.0],      // Bucket 0-5min
                [180000.0, 100.5], // 3min - same bucket
                [360000.0, 101.0], // 6min - new bucket (5-10min)
                [540000.0, 101.5], // 9min - new bucket (9-10min? no, still in 5-10min bucket)
            ],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        // Should have 2 buckets: 0-5min and 5-10min
        assert_eq!(candles.len(), 2);

        // First bucket contains 100.0 and 100.5
        assert_eq!(candles[0].open, 100.0);
        assert_eq!(candles[0].high, 100.5);
        assert_eq!(candles[0].low, 100.0);
        assert_eq!(candles[0].close, 100.5);

        // Second bucket contains 101.0 and 101.5
        assert_eq!(candles[1].open, 101.0);
        assert_eq!(candles[1].high, 101.5);
        assert_eq!(candles[1].low, 101.0);
        assert_eq!(candles[1].close, 101.5);
    }

    #[test]
    fn test_convert_sparse_data_fills_gaps() {
        let converter = CandleConverter::new();

        // Data with gaps - should fill empty buckets with interpolated values
        let data = MarketChartData {
            prices: vec![
                [0.0, 100.0],      // Bucket 0 (0min)
                [300000.0, 101.0], // Bucket 1 (5min)
                // GAP - no data at bucket 2 (10min) - will interpolate with 101.0
                [900000.0, 102.0], // Bucket 3 (15min)
            ],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        // Should have 4 candles: 3 real + 1 interpolated at 10min
        assert_eq!(candles.len(), 4);
        assert_eq!(candles[0].close, 100.0); // 0min - real
        assert_eq!(candles[1].close, 101.0); // 5min - real
        assert_eq!(candles[2].close, 101.0); // 10min - interpolated (uses last price)
        assert_eq!(candles[3].close, 102.0); // 15min - real

        // Interpolated candle should have all OHLC equal to last price
        assert_eq!(candles[2].open, 101.0);
        assert_eq!(candles[2].high, 101.0);
        assert_eq!(candles[2].low, 101.0);
    }

    #[test]
    fn test_sort_out_of_order_timestamps() {
        let converter = CandleConverter::new();

        // Out of order timestamps
        let data = MarketChartData {
            prices: vec![
                [300000.0, 101.0], // Out of order
                [0.0, 100.0],
                [600000.0, 102.0],
            ],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        // Should sort before processing
        assert_eq!(candles.len(), 3);
        assert_eq!(candles[0].close, 100.0);
        assert_eq!(candles[1].close, 101.0);
        assert_eq!(candles[2].close, 102.0);
    }

    #[test]
    fn test_dedupe_duplicate_timestamps() {
        let converter = CandleConverter::new();

        // Duplicate timestamps - should use last value
        let data = MarketChartData {
            prices: vec![
                [0.0, 100.0],
                [0.0, 100.5], // Duplicate timestamp (use this one)
                [300000.0, 101.0],
            ],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        // Should dedupe and use last value
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open, 100.5); // Used last value for duplicate
        assert_eq!(candles[0].close, 100.5);
    }

    #[test]
    fn test_synthesize_candle_ohlc() {
        let converter = CandleConverter::new();

        // Multiple prices in one bucket - test OHLC calculation
        let data = MarketChartData {
            prices: vec![
                [0.0, 100.0],      // Open
                [60000.0, 102.0],  // High
                [120000.0, 99.0],  // Low
                [180000.0, 101.0], // Close
            ],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open, 100.0);
        assert_eq!(candles[0].high, 102.0);
        assert_eq!(candles[0].low, 99.0);
        assert_eq!(candles[0].close, 101.0);
    }

    #[test]
    fn test_bucket_alignment() {
        let converter = CandleConverter::new();

        // Test that timestamps align to 5-min boundaries
        let data = MarketChartData {
            prices: vec![
                [123000.0, 100.0], // ~2min mark -> aligns to 0 bucket
                [456000.0, 101.0], // ~7.6min mark -> aligns to 300 bucket (5min)
            ],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        assert_eq!(candles.len(), 2);

        // First candle should align to 0 seconds
        assert_eq!(candles[0].timestamp.timestamp(), 0);

        // Second candle should align to 300 seconds (5 min)
        assert_eq!(candles[1].timestamp.timestamp(), 300);
    }

    // ===== Tests for Hourly Converter =====

    #[test]
    fn test_hourly_converter_creation() {
        let converter = CandleConverter::for_hourly();
        assert_eq!(converter.interval_secs(), 3600);
    }

    #[test]
    fn test_custom_interval_converter() {
        let converter = CandleConverter::new_with_interval(1800); // 30 minutes
        assert_eq!(converter.interval_secs(), 1800);
    }

    #[test]
    fn test_hourly_converter_uniform_data() {
        let converter = CandleConverter::for_hourly();

        // Perfect 1-hour intervals (3,600,000 ms = 3600 seconds)
        let data = MarketChartData {
            prices: vec![
                [0.0, 100.0],
                [3600000.0, 101.0],
                [7200000.0, 102.0],
                [10800000.0, 103.0],
            ],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        assert_eq!(candles.len(), 4);
        assert_eq!(candles[0].close, 100.0);
        assert_eq!(candles[1].close, 101.0);
        assert_eq!(candles[2].close, 102.0);
        assert_eq!(candles[3].close, 103.0);

        // Check timestamps are hourly aligned
        assert_eq!(candles[0].timestamp.timestamp(), 0);
        assert_eq!(candles[1].timestamp.timestamp(), 3600);
        assert_eq!(candles[2].timestamp.timestamp(), 7200);
        assert_eq!(candles[3].timestamp.timestamp(), 10800);
    }

    #[test]
    fn test_hourly_converter_irregular_data() {
        let converter = CandleConverter::for_hourly();

        // Irregular intervals - multiple points in same hour
        let data = MarketChartData {
            prices: vec![
                [0.0, 100.0],       // Bucket 0-1h
                [1800000.0, 100.5], // 30min - same bucket
                [3600000.0, 101.0], // 1h - new bucket
                [5400000.0, 101.5], // 1.5h - still in 1-2h bucket
            ],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        // Should have 2 buckets: 0-1h and 1-2h
        assert_eq!(candles.len(), 2);

        // First bucket contains 100.0 and 100.5
        assert_eq!(candles[0].open, 100.0);
        assert_eq!(candles[0].high, 100.5);
        assert_eq!(candles[0].low, 100.0);
        assert_eq!(candles[0].close, 100.5);

        // Second bucket contains 101.0 and 101.5
        assert_eq!(candles[1].open, 101.0);
        assert_eq!(candles[1].high, 101.5);
        assert_eq!(candles[1].low, 101.0);
        assert_eq!(candles[1].close, 101.5);
    }

    #[test]
    fn test_hourly_converter_sparse_data_fills_gaps() {
        let converter = CandleConverter::for_hourly();

        // Data with gaps - should fill empty buckets
        let data = MarketChartData {
            prices: vec![
                [0.0, 100.0],       // Hour 0
                [3600000.0, 101.0], // Hour 1
                // GAP - no data at hour 2 - will interpolate with 101.0
                [10800000.0, 102.0], // Hour 3
            ],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        // Should have 4 candles: 3 real + 1 interpolated at hour 2
        assert_eq!(candles.len(), 4);
        assert_eq!(candles[0].close, 100.0); // Hour 0 - real
        assert_eq!(candles[1].close, 101.0); // Hour 1 - real
        assert_eq!(candles[2].close, 101.0); // Hour 2 - interpolated
        assert_eq!(candles[3].close, 102.0); // Hour 3 - real

        // Interpolated candle should be flat
        assert_eq!(candles[2].open, 101.0);
        assert_eq!(candles[2].high, 101.0);
        assert_eq!(candles[2].low, 101.0);
    }

    #[test]
    fn test_hourly_converter_no_artificial_5min_candles() {
        let converter = CandleConverter::for_hourly();

        // Simulate CoinGecko hourly data (2-90 day backfill)
        // 24 points for 1 day of hourly data
        let mut prices = Vec::new();
        for hour in 0..24 {
            let timestamp_ms = (hour * 3600 * 1000) as f64;
            let price = 100.0 + hour as f64; // Gradually increasing price
            prices.push([timestamp_ms, price]);
        }

        let data = MarketChartData {
            prices,
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        // Should have exactly 24 hourly candles, NOT 24 * 12 = 288 fake 5-min candles
        assert_eq!(
            candles.len(),
            24,
            "Should create 24 hourly candles, not 288 5-minute candles"
        );

        // Each candle should be 1 hour apart
        for i in 1..candles.len() {
            let interval = (candles[i].timestamp - candles[i - 1].timestamp).num_seconds();
            assert_eq!(
                interval, 3600,
                "Candles should be 1 hour (3600s) apart, got {}s",
                interval
            );
        }
    }

    #[test]
    fn test_volume_data_extraction() {
        let converter = CandleConverter::new();

        // Simulate CoinGecko data with both prices and volumes
        let data = MarketChartData {
            prices: vec![[0.0, 100.0], [300000.0, 101.0], [600000.0, 102.0]],
            total_volumes: vec![
                [0.0, 1000000.0],      // $1M volume at first candle
                [300000.0, 1500000.0], // $1.5M volume at second candle
                [600000.0, 2000000.0], // $2M volume at third candle
            ],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();
        assert_eq!(candles.len(), 3);

        // Verify volumes were extracted correctly (not hardcoded to 0.0)
        assert_eq!(candles[0].volume, 1000000.0);
        assert_eq!(candles[1].volume, 1500000.0);
        assert_eq!(candles[2].volume, 2000000.0);
    }

    #[test]
    fn test_volume_data_missing_graceful_fallback() {
        let converter = CandleConverter::new();

        // Simulate CoinGecko data with prices but no volumes
        let data = MarketChartData {
            prices: vec![[0.0, 100.0], [300000.0, 101.0]],
            total_volumes: vec![], // Empty volumes
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();
        assert_eq!(candles.len(), 2);

        // Should gracefully fall back to 0.0 when no volume data
        assert_eq!(candles[0].volume, 0.0);
        assert_eq!(candles[1].volume, 0.0);
    }

    #[test]
    fn test_hourly_converter_7day_simulation() {
        let converter = CandleConverter::for_hourly();

        // Simulate 7 days of hourly data (168 hours)
        let mut prices = Vec::new();
        for hour in 0..168 {
            let timestamp_ms = (hour * 3600 * 1000) as f64;
            let price = 100.0 + (hour as f64 * 0.1); // Gradual trend
            prices.push([timestamp_ms, price]);
        }

        let data = MarketChartData {
            prices,
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        // Should have 168 hourly candles for 7 days
        assert_eq!(
            candles.len(),
            168,
            "Should have 168 hourly candles for 7 days"
        );

        // Verify first and last prices
        assert_eq!(candles[0].close, 100.0);
        assert!((candles[167].close - 116.7).abs() < 0.1); // ~100 + 16.7

        // Verify no gaps
        for i in 1..candles.len() {
            let interval = (candles[i].timestamp - candles[i - 1].timestamp).num_seconds();
            assert_eq!(interval, 3600, "Should have no gaps in hourly data");
        }
    }

    #[test]
    fn test_hourly_converter_gap_filling_large_gaps() {
        let converter = CandleConverter::for_hourly();

        // Data with large gap (4 hours missing)
        let data = MarketChartData {
            prices: vec![
                [0.0, 100.0],       // Hour 0
                [3600000.0, 101.0], // Hour 1
                // Missing: Hours 2, 3, 4, 5
                [21600000.0, 106.0], // Hour 6
            ],
            total_volumes: vec![],
        };

        let candles = converter.convert_to_candles("SOL", data).unwrap();

        // Should fill 4 missing hours
        assert_eq!(candles.len(), 7, "Should fill 4 missing hours");

        // All interpolated candles should use last known price (101.0)
        assert_eq!(candles[2].close, 101.0); // Hour 2
        assert_eq!(candles[3].close, 101.0); // Hour 3
        assert_eq!(candles[4].close, 101.0); // Hour 4
        assert_eq!(candles[5].close, 101.0); // Hour 5
        assert_eq!(candles[6].close, 106.0); // Hour 6 (real data)
    }
}
