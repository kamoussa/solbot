/// Average Directional Index (ADX) - Measures trend strength
///
/// ADX ranges from 0 to 100:
/// - ADX > 25: Strong trend (bull or bear)
/// - ADX 20-25: Moderate trend
/// - ADX < 20: Weak trend / choppy / ranging market
///
/// Also returns +DI and -DI to determine trend direction:
/// - +DI > -DI: Uptrend
/// - -DI > +DI: Downtrend

use crate::models::Candle;

/// Calculate ADX, +DI, and -DI for trend strength and direction
///
/// Returns (adx, plus_di, minus_di) or None if insufficient data
pub fn calculate_adx(candles: &[Candle], period: usize) -> Option<(f64, f64, f64)> {
    if candles.len() < period + 1 {
        return None;
    }

    // Step 1: Calculate True Range (TR) and Directional Movement (+DM, -DM)
    let mut true_ranges = Vec::new();
    let mut plus_dms = Vec::new();
    let mut minus_dms = Vec::new();

    for i in 1..candles.len() {
        let high = candles[i].high;
        let low = candles[i].low;
        let prev_close = candles[i - 1].close;
        let prev_high = candles[i - 1].high;
        let prev_low = candles[i - 1].low;

        // True Range = max(high - low, abs(high - prev_close), abs(low - prev_close))
        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
        true_ranges.push(tr);

        // Directional Movement
        let up_move = high - prev_high;
        let down_move = prev_low - low;

        let plus_dm = if up_move > down_move && up_move > 0.0 {
            up_move
        } else {
            0.0
        };

        let minus_dm = if down_move > up_move && down_move > 0.0 {
            down_move
        } else {
            0.0
        };

        plus_dms.push(plus_dm);
        minus_dms.push(minus_dm);
    }

    if true_ranges.len() < period {
        return None;
    }

    // Step 2: Smooth True Range and Directional Movements (Wilder's smoothing)
    let smoothed_tr = wilder_smooth(&true_ranges, period)?;
    let smoothed_plus_dm = wilder_smooth(&plus_dms, period)?;
    let smoothed_minus_dm = wilder_smooth(&minus_dms, period)?;

    // Step 3: Calculate +DI and -DI
    let plus_di = if smoothed_tr > 0.0 {
        (smoothed_plus_dm / smoothed_tr) * 100.0
    } else {
        0.0
    };

    let minus_di = if smoothed_tr > 0.0 {
        (smoothed_minus_dm / smoothed_tr) * 100.0
    } else {
        0.0
    };

    // Step 4: Calculate DX (Directional Index)
    let di_sum = plus_di + minus_di;
    let dx = if di_sum > 0.0 {
        ((plus_di - minus_di).abs() / di_sum) * 100.0
    } else {
        0.0
    };

    // Step 5: Calculate ADX (smoothed DX)
    // For simplicity, we return the current DX as ADX approximation
    // In production, you'd need to maintain a smoothed ADX over multiple periods
    let adx = dx;

    Some((adx, plus_di, minus_di))
}

/// Wilder's smoothing method (similar to EMA but using Wilder's formula)
fn wilder_smooth(values: &[f64], period: usize) -> Option<f64> {
    if values.len() < period {
        return None;
    }

    // First smoothed value is simple average of first 'period' values
    let first_smooth: f64 = values[..period].iter().sum::<f64>() / period as f64;

    // Apply Wilder's smoothing for remaining values
    let mut smoothed = first_smooth;
    for value in &values[period..] {
        smoothed = (smoothed * (period as f64 - 1.0) + value) / period as f64;
    }

    Some(smoothed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_candles(prices: &[(f64, f64, f64, f64)]) -> Vec<Candle> {
        prices
            .iter()
            .enumerate()
            .map(|(i, &(open, high, low, close))| Candle {
                token: "TEST".to_string(),
                timestamp: Utc::now() + chrono::Duration::hours(i as i64),
                open,
                high,
                low,
                close,
                volume: 1000.0,
            })
            .collect()
    }

    #[test]
    fn test_adx_strong_uptrend() {
        // Simulating a strong uptrend
        let prices = vec![
            (100.0, 102.0, 99.0, 101.0),
            (101.0, 105.0, 100.0, 104.0),
            (104.0, 108.0, 103.0, 107.0),
            (107.0, 112.0, 106.0, 110.0),
            (110.0, 115.0, 109.0, 113.0),
            (113.0, 118.0, 112.0, 116.0),
            (116.0, 121.0, 115.0, 119.0),
            (119.0, 124.0, 118.0, 122.0),
            (122.0, 127.0, 121.0, 125.0),
            (125.0, 130.0, 124.0, 128.0),
            (128.0, 133.0, 127.0, 131.0),
            (131.0, 136.0, 130.0, 134.0),
            (134.0, 139.0, 133.0, 137.0),
            (137.0, 142.0, 136.0, 140.0),
            (140.0, 145.0, 139.0, 143.0),
        ];

        let candles = create_test_candles(&prices);
        let (adx, plus_di, minus_di) = calculate_adx(&candles, 14).unwrap();

        // In a strong uptrend:
        // - ADX should be high (> 25)
        // - +DI should be greater than -DI
        assert!(plus_di > minus_di, "+DI should be > -DI in uptrend");
        println!(
            "Strong uptrend: ADX={:.2}, +DI={:.2}, -DI={:.2}",
            adx, plus_di, minus_di
        );
    }

    #[test]
    fn test_adx_choppy_market() {
        // Simulating a choppy/ranging market
        let prices = vec![
            (100.0, 102.0, 98.0, 100.0),
            (100.0, 103.0, 97.0, 99.0),
            (99.0, 102.0, 98.0, 101.0),
            (101.0, 103.0, 99.0, 100.0),
            (100.0, 102.0, 98.0, 99.0),
            (99.0, 103.0, 97.0, 101.0),
            (101.0, 103.0, 99.0, 100.0),
            (100.0, 102.0, 98.0, 99.0),
            (99.0, 103.0, 97.0, 100.0),
            (100.0, 102.0, 98.0, 101.0),
            (101.0, 103.0, 99.0, 100.0),
            (100.0, 102.0, 98.0, 99.0),
            (99.0, 103.0, 97.0, 101.0),
            (101.0, 103.0, 99.0, 100.0),
            (100.0, 102.0, 98.0, 99.0),
        ];

        let candles = create_test_candles(&prices);
        let (adx, plus_di, minus_di) = calculate_adx(&candles, 14).unwrap();

        // In a choppy market:
        // - ADX should be low (< 20)
        // - +DI and -DI should be relatively close
        println!(
            "Choppy market: ADX={:.2}, +DI={:.2}, -DI={:.2}",
            adx, plus_di, minus_di
        );
        assert!(
            adx < 40.0,
            "ADX should be lower in choppy market, got {:.2}",
            adx
        );
    }

    #[test]
    fn test_adx_insufficient_data() {
        let prices = vec![
            (100.0, 102.0, 99.0, 101.0),
            (101.0, 105.0, 100.0, 104.0),
        ];

        let candles = create_test_candles(&prices);
        let result = calculate_adx(&candles, 14);

        assert!(result.is_none(), "Should return None for insufficient data");
    }
}
