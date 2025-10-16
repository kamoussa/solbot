/// Calculate Simple Moving Average (SMA)
pub fn calculate_sma(prices: &[f64], period: usize) -> Option<f64> {
    if prices.len() < period {
        return None;
    }

    let sum: f64 = prices.iter().rev().take(period).sum();
    Some(sum / period as f64)
}

/// Calculate Exponential Moving Average (EMA)
pub fn calculate_ema(prices: &[f64], period: usize) -> Option<f64> {
    if prices.len() < period {
        return None;
    }

    let multiplier = 2.0 / (period as f64 + 1.0);

    // Start with SMA
    let initial_sma = calculate_sma(&prices[0..period], period)?;

    // Calculate EMA
    let mut ema = initial_sma;
    for price in &prices[period..] {
        ema = (price - ema) * multiplier + ema;
    }

    Some(ema)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sma() {
        let prices = vec![100.0, 102.0, 104.0, 106.0, 108.0];
        let sma = calculate_sma(&prices, 5);
        assert_eq!(sma, Some(104.0));
    }

    #[test]
    fn test_sma_insufficient_data() {
        let prices = vec![100.0, 102.0];
        let sma = calculate_sma(&prices, 5);
        assert!(sma.is_none());
    }

    #[test]
    fn test_ema() {
        let prices = vec![100.0, 102.0, 104.0, 106.0, 108.0, 110.0];
        let ema = calculate_ema(&prices, 5);
        assert!(ema.is_some());
        assert!(ema.unwrap() > 104.0); // EMA should be above initial SMA
    }
}
