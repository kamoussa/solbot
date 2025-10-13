/// Calculate Relative Strength Index (RSI)
///
/// RSI measures the magnitude of recent price changes to evaluate
/// overbought or oversold conditions.
///
/// Values:
/// - RSI > 70: Overbought
/// - RSI < 30: Oversold
///
pub fn calculate_rsi(prices: &[f64], period: usize) -> Option<f64> {
    if prices.len() < period + 1 {
        return None;
    }

    let mut gains = Vec::new();
    let mut losses = Vec::new();

    // Calculate price changes
    for i in 1..prices.len() {
        let change = prices[i] - prices[i - 1];
        if change > 0.0 {
            gains.push(change);
            losses.push(0.0);
        } else {
            gains.push(0.0);
            losses.push(change.abs());
        }
    }

    if gains.len() < period {
        return None;
    }

    // Calculate average gain and loss
    let avg_gain: f64 = gains.iter().rev().take(period).sum::<f64>() / period as f64;
    let avg_loss: f64 = losses.iter().rev().take(period).sum::<f64>() / period as f64;

    if avg_loss == 0.0 {
        return Some(100.0);
    }

    let rs = avg_gain / avg_loss;
    let rsi = 100.0 - (100.0 / (1.0 + rs));

    Some(rsi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rsi_calculation() {
        // Test with known values
        let prices = vec![
            44.0, 44.25, 44.5, 43.75, 44.0, 44.5, 45.0, 45.5, 45.25, 45.5,
            46.0, 46.5, 46.25, 46.0, 46.5,
        ];

        let rsi = calculate_rsi(&prices, 14);
        assert!(rsi.is_some());

        let rsi_value = rsi.unwrap();
        assert!(rsi_value > 0.0 && rsi_value < 100.0);
    }

    #[test]
    fn test_rsi_insufficient_data() {
        let prices = vec![100.0, 102.0, 101.0];
        let rsi = calculate_rsi(&prices, 14);
        assert!(rsi.is_none());
    }

    #[test]
    fn test_rsi_all_gains() {
        let prices = vec![100.0, 101.0, 102.0, 103.0, 104.0, 105.0];
        let rsi = calculate_rsi(&prices, 5);
        assert!(rsi.is_some());
        assert_eq!(rsi.unwrap(), 100.0);  // All gains = RSI 100
    }
}
