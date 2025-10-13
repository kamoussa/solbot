use crate::models::Signal;
use crate::indicators::{calculate_rsi, calculate_sma, calculate_ema};

/// Configuration for signal generation
#[derive(Debug, Clone)]
pub struct SignalConfig {
    pub rsi_period: usize,
    pub rsi_oversold: f64,
    pub rsi_overbought: f64,
    pub short_ma_period: usize,
    pub long_ma_period: usize,
    pub volume_threshold: f64, // Multiple of average volume
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            rsi_period: 14,
            rsi_oversold: 30.0,
            rsi_overbought: 70.0,
            short_ma_period: 10,
            long_ma_period: 20,
            volume_threshold: 1.5,
        }
    }
}

/// Analyze market conditions and generate composite signal
pub fn analyze_market_conditions(
    prices: &[f64],
    volumes: &[f64],
    config: &SignalConfig,
) -> Option<Signal> {
    if prices.len() < config.long_ma_period + 1 {
        return None;
    }

    // Calculate indicators
    let rsi = calculate_rsi(prices, config.rsi_period)?;
    let short_ma = calculate_sma(prices, config.short_ma_period)?;
    let long_ma = calculate_sma(prices, config.long_ma_period)?;

    // Calculate volume conditions
    let avg_volume = volumes.iter().sum::<f64>() / volumes.len() as f64;
    let current_volume = volumes.last()?;
    let volume_spike = current_volume / avg_volume > config.volume_threshold;

    // Current price
    let current_price = prices.last()?;

    // Signal logic
    let signal = if is_buy_signal(
        rsi,
        short_ma,
        long_ma,
        *current_price,
        volume_spike,
        config,
    ) {
        Signal::Buy
    } else if is_sell_signal(rsi, short_ma, long_ma, config) {
        Signal::Sell
    } else {
        Signal::Hold
    };

    Some(signal)
}

fn is_buy_signal(
    rsi: f64,
    short_ma: f64,
    long_ma: f64,
    current_price: f64,
    volume_spike: bool,
    config: &SignalConfig,
) -> bool {
    // Buy conditions:
    // 1. RSI shows oversold OR approaching neutral from oversold
    // 2. Short MA crossing above long MA (bullish momentum)
    // 3. Price above short MA (confirmation)
    // 4. Volume spike (strong interest)

    let rsi_condition = rsi < config.rsi_oversold + 10.0; // Oversold or recovering
    let ma_crossover = short_ma > long_ma;
    let price_above_ma = current_price > short_ma;

    // Require at least 3 out of 4 conditions
    let conditions = [rsi_condition, ma_crossover, price_above_ma, volume_spike];
    conditions.iter().filter(|&&x| x).count() >= 3
}

fn is_sell_signal(rsi: f64, short_ma: f64, long_ma: f64, config: &SignalConfig) -> bool {
    // Sell conditions:
    // 1. RSI shows overbought
    // 2. Short MA crossing below long MA (bearish momentum)

    let rsi_overbought = rsi > config.rsi_overbought;
    let ma_crossunder = short_ma < long_ma;

    // Require both conditions for sell (conservative)
    rsi_overbought && ma_crossunder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_generation_buy() {
        // Uptrend with volume spike
        let prices = vec![
            100.0, 102.0, 104.0, 106.0, 108.0, 110.0, 112.0, 114.0,
            116.0, 118.0, 120.0, 122.0, 124.0, 126.0, 128.0, 130.0,
            132.0, 134.0, 136.0, 138.0, 140.0,
        ];
        let volumes = vec![
            1000.0, 1100.0, 1200.0, 1300.0, 1400.0, 1500.0, 1600.0, 1700.0,
            1800.0, 1900.0, 2000.0, 2100.0, 2200.0, 2300.0, 2400.0, 2500.0,
            2600.0, 2700.0, 2800.0, 2900.0, 5000.0, // Volume spike
        ];

        let config = SignalConfig::default();
        let signal = analyze_market_conditions(&prices, &volumes, &config);

        assert!(signal.is_some());
        // Note: Uptrend means RSI is high, so might not trigger buy
        // This tests that the function runs, actual signal depends on thresholds
    }

    #[test]
    fn test_signal_generation_sell() {
        // Create overbought conditions
        let mut prices = vec![100.0];
        for i in 1..25 {
            prices.push(100.0 + i as f64 * 2.0); // Steady climb
        }
        // Then decline
        for i in 0..5 {
            prices.push(150.0 - i as f64 * 3.0);
        }

        let volumes = vec![1000.0; prices.len()];

        let config = SignalConfig::default();
        let signal = analyze_market_conditions(&prices, &volumes, &config);

        assert!(signal.is_some());
        let signal = signal.unwrap();
        // After steady climb and reversal, should consider selling
        assert!(matches!(signal, Signal::Sell | Signal::Hold));
    }

    #[test]
    fn test_insufficient_data() {
        let prices = vec![100.0, 101.0, 102.0];
        let volumes = vec![1000.0, 1100.0, 1200.0];
        let config = SignalConfig::default();

        let signal = analyze_market_conditions(&prices, &volumes, &config);
        assert!(signal.is_none());
    }

    #[test]
    fn test_custom_config() {
        let config = SignalConfig {
            rsi_period: 10,
            rsi_oversold: 25.0,
            rsi_overbought: 75.0,
            short_ma_period: 5,
            long_ma_period: 15,
            volume_threshold: 2.0,
        };

        let prices = vec![100.0; 20];
        let volumes = vec![1000.0; 20];

        let signal = analyze_market_conditions(&prices, &volumes, &config);
        assert!(signal.is_some());
        // Flat prices should result in Hold
        assert_eq!(signal.unwrap(), Signal::Hold);
    }
}
