use chrono::DateTime;
use std::collections::BTreeMap;

use crate::api::MarketChartData;
use crate::models::Candle;
use crate::Result;

const BUCKET_INTERVAL_SECS: i64 = 300; // 5 minutes

/// Converts irregular price points from CoinGecko to uniform 5-minute OHLC candles
pub struct CandleConverter {
    interval_secs: i64,
}

impl CandleConverter {
    /// Create a new converter with default 5-minute interval
    pub fn new() -> Self {
        Self {
            interval_secs: BUCKET_INTERVAL_SECS,
        }
    }

    /// Convert market chart data to OHLC candles with gap filling
    ///
    /// CoinGecko's 1-day API returns irregular/sparse data with large gaps.
    /// This function fills gaps > 5 minutes by interpolating candles using the last known price.
    pub fn convert_to_candles(&self, symbol: &str, data: MarketChartData) -> Result<Vec<Candle>> {
        if data.prices.is_empty() {
            return Ok(Vec::new());
        }

        // Sort and deduplicate timestamps
        let prices = self.sort_and_dedupe(data.prices);

        // Bucket into 5-minute windows
        let buckets = self.bucket_into_windows(prices);

        if buckets.is_empty() {
            return Ok(Vec::new());
        }

        // Convert buckets to candles and fill gaps
        let mut candles = Vec::new();
        let bucket_timestamps: Vec<i64> = buckets.keys().copied().collect();

        for i in 0..bucket_timestamps.len() {
            let bucket_timestamp = bucket_timestamps[i];
            let bucket_prices = buckets.get(&bucket_timestamp).unwrap(); // Safe: we're iterating keys

            if bucket_prices.is_empty() {
                // Empty bucket shouldn't happen (bucket_into_windows only creates non-empty buckets)
                tracing::warn!("Skipping empty bucket at timestamp {}", bucket_timestamp);
                continue;
            }

            // Add real candle
            let candle = self.synthesize_candle(symbol, bucket_timestamp, bucket_prices.clone());
            candles.push(candle);

            // Fill gaps to next bucket (if any)
            if i + 1 < bucket_timestamps.len() {
                let filled = self.fill_gap(
                    symbol,
                    bucket_timestamp,
                    bucket_timestamps[i + 1],
                    *bucket_prices.last().unwrap(), // Safe: we checked !is_empty above
                );
                candles.extend(filled);
            }
        }

        Ok(candles)
    }

    /// Fill gaps between two timestamps with interpolated candles
    fn fill_gap(&self, symbol: &str, from: i64, to: i64, last_price: f64) -> Vec<Candle> {
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

    /// Synthesize a candle from a bucket of prices
    fn synthesize_candle(&self, symbol: &str, bucket_timestamp: i64, prices: Vec<f64>) -> Candle {
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
            volume: 0.0, // As per research findings: CoinGecko returns 24h rolling volume
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
}
