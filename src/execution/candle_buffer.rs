use crate::models::Candle;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

/// Thread-safe in-memory buffer for candle data
///
/// Maintains a rolling window of candles for each token
#[derive(Clone)]
pub struct CandleBuffer {
    data: Arc<RwLock<HashMap<String, VecDeque<Candle>>>>,
    max_candles: usize,
}

impl CandleBuffer {
    /// Create a new candle buffer
    ///
    /// # Arguments
    /// * `max_candles` - Maximum number of candles to keep per token
    pub fn new(max_candles: usize) -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
            max_candles,
        }
    }

    /// Add a candle for a token
    ///
    /// If the buffer is full, removes the oldest candle
    pub fn add_candle(&self, candle: Candle) -> Result<(), String> {
        let mut data = self.data.write().map_err(|e| e.to_string())?;

        let token_candles = data
            .entry(candle.token.clone())
            .or_insert_with(VecDeque::new);

        // Add new candle
        token_candles.push_back(candle);

        // Remove oldest if exceeds max
        while token_candles.len() > self.max_candles {
            token_candles.pop_front();
        }

        Ok(())
    }

    /// Get all candles for a token
    pub fn get_candles(&self, token: &str) -> Result<Vec<Candle>, String> {
        let data = self.data.read().map_err(|e| e.to_string())?;

        Ok(data
            .get(token)
            .map(|deque| deque.iter().cloned().collect())
            .unwrap_or_default())
    }

    /// Get the N most recent candles for a token
    pub fn get_recent_candles(&self, token: &str, n: usize) -> Result<Vec<Candle>, String> {
        let data = self.data.read().map_err(|e| e.to_string())?;

        Ok(data
            .get(token)
            .map(|deque| deque.iter().rev().take(n).rev().cloned().collect())
            .unwrap_or_default())
    }

    /// Get count of candles for a token
    pub fn candle_count(&self, token: &str) -> Result<usize, String> {
        let data = self.data.read().map_err(|e| e.to_string())?;
        Ok(data.get(token).map(|d| d.len()).unwrap_or(0))
    }

    /// Get all tracked tokens
    pub fn tokens(&self) -> Result<Vec<String>, String> {
        let data = self.data.read().map_err(|e| e.to_string())?;
        Ok(data.keys().cloned().collect())
    }

    /// Clear all data for a token
    pub fn clear_token(&self, token: &str) -> Result<(), String> {
        let mut data = self.data.write().map_err(|e| e.to_string())?;
        data.remove(token);
        Ok(())
    }

    /// Clear all data
    pub fn clear_all(&self) -> Result<(), String> {
        let mut data = self.data.write().map_err(|e| e.to_string())?;
        data.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_candle(token: &str, price: f64) -> Candle {
        Candle {
            token: token.to_string(),
            timestamp: Utc::now(),
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 1000.0,
        }
    }

    #[test]
    fn test_new_buffer() {
        let buffer = CandleBuffer::new(100);
        assert_eq!(buffer.max_candles, 100);
        assert!(buffer.tokens().unwrap().is_empty());
    }

    #[test]
    fn test_add_candle() {
        let buffer = CandleBuffer::new(100);
        let candle = create_test_candle("SOL", 100.0);

        let result = buffer.add_candle(candle);
        assert!(result.is_ok());
        assert_eq!(buffer.candle_count("SOL").unwrap(), 1);
    }

    #[test]
    fn test_get_candles() {
        let buffer = CandleBuffer::new(100);

        buffer.add_candle(create_test_candle("SOL", 100.0)).unwrap();
        buffer.add_candle(create_test_candle("SOL", 101.0)).unwrap();
        buffer.add_candle(create_test_candle("SOL", 102.0)).unwrap();

        let candles = buffer.get_candles("SOL").unwrap();
        assert_eq!(candles.len(), 3);
        assert_eq!(candles[0].close, 100.0);
        assert_eq!(candles[2].close, 102.0);
    }

    #[test]
    fn test_max_candles_limit() {
        let buffer = CandleBuffer::new(5);

        // Add 10 candles
        for i in 0..10 {
            buffer
                .add_candle(create_test_candle("SOL", 100.0 + i as f64))
                .unwrap();
        }

        let candles = buffer.get_candles("SOL").unwrap();
        assert_eq!(candles.len(), 5); // Should only keep last 5

        // Should have prices 105-109
        assert_eq!(candles[0].close, 105.0);
        assert_eq!(candles[4].close, 109.0);
    }

    #[test]
    fn test_multiple_tokens() {
        let buffer = CandleBuffer::new(100);

        buffer.add_candle(create_test_candle("SOL", 100.0)).unwrap();
        buffer.add_candle(create_test_candle("JUP", 200.0)).unwrap();
        buffer
            .add_candle(create_test_candle("BONK", 300.0))
            .unwrap();

        let tokens = buffer.tokens().unwrap();
        assert_eq!(tokens.len(), 3);
        assert!(tokens.contains(&"SOL".to_string()));
        assert!(tokens.contains(&"JUP".to_string()));
        assert!(tokens.contains(&"BONK".to_string()));
    }

    #[test]
    fn test_get_recent_candles() {
        let buffer = CandleBuffer::new(100);

        for i in 0..10 {
            buffer
                .add_candle(create_test_candle("SOL", 100.0 + i as f64))
                .unwrap();
        }

        let recent = buffer.get_recent_candles("SOL", 3).unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].close, 107.0); // Last 3: 107, 108, 109
        assert_eq!(recent[2].close, 109.0);
    }

    #[test]
    fn test_clear_token() {
        let buffer = CandleBuffer::new(100);

        buffer.add_candle(create_test_candle("SOL", 100.0)).unwrap();
        buffer.add_candle(create_test_candle("JUP", 200.0)).unwrap();

        buffer.clear_token("SOL").unwrap();

        assert_eq!(buffer.candle_count("SOL").unwrap(), 0);
        assert_eq!(buffer.candle_count("JUP").unwrap(), 1);
    }

    #[test]
    fn test_clear_all() {
        let buffer = CandleBuffer::new(100);

        buffer.add_candle(create_test_candle("SOL", 100.0)).unwrap();
        buffer.add_candle(create_test_candle("JUP", 200.0)).unwrap();

        buffer.clear_all().unwrap();

        assert!(buffer.tokens().unwrap().is_empty());
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;

        let buffer = CandleBuffer::new(100);
        let buffer_clone = buffer.clone();

        let handle = thread::spawn(move || {
            for i in 0..50 {
                buffer_clone
                    .add_candle(create_test_candle("SOL", 100.0 + i as f64))
                    .unwrap();
            }
        });

        for i in 50..100 {
            buffer
                .add_candle(create_test_candle("SOL", 100.0 + i as f64))
                .unwrap();
        }

        handle.join().unwrap();

        // Should have 100 candles (max limit)
        assert_eq!(buffer.candle_count("SOL").unwrap(), 100);
    }
}
