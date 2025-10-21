use crate::api::birdeye::TrendingToken;

/// Safety filter heuristic for token discovery
///
/// Applies multiple checks to identify potentially unsafe tokens:
/// 1. Input validation (negative values, NaN, infinity)
/// 2. Data sanity (liquidity vs FDV)
/// 3. Minimum volume threshold (applies to ALL tokens, even blue chips)
/// 4. Blue chip identification (high liquidity + market cap)
/// 5. Established tokens (good metrics + rank)
/// 6. Liquidity ratio health check
/// 7. Volume/liquidity ratio sanity check
/// 8. Minimum liquidity threshold
///
/// Returns (is_safe, reason) tuple
pub fn is_safe_token(token: &TrendingToken) -> (bool, String) {
    // 1. Input validation: Check for invalid numeric values
    if token.liquidity_usd < 0.0
        || token.liquidity_usd.is_nan()
        || token.liquidity_usd.is_infinite()
    {
        return (
            false,
            "InvalidData: Liquidity is negative, NaN, or infinite".to_string(),
        );
    }
    if token.volume_24h_usd < 0.0
        || token.volume_24h_usd.is_nan()
        || token.volume_24h_usd.is_infinite()
    {
        return (
            false,
            "InvalidData: Volume is negative, NaN, or infinite".to_string(),
        );
    }
    if token.fdv < 0.0 || token.fdv.is_nan() || token.fdv.is_infinite() {
        return (
            false,
            "InvalidData: FDV is negative, NaN, or infinite".to_string(),
        );
    }
    if token.price <= 0.0 || token.price.is_nan() || token.price.is_infinite() {
        return (false, "InvalidData: Price is invalid".to_string());
    }

    // 2. Sanity check: Liquidity should not exceed FDV
    // Liquidity is value locked in pools, FDV is total token value
    // Liquidity can be high (e.g. 50% of FDV for well-paired tokens), but > 100% is suspicious
    if token.fdv > 0.0 && token.liquidity_usd > token.fdv {
        let ratio = (token.liquidity_usd / token.fdv) * 100.0;
        return (
            false,
            format!(
                "SuspiciousData: Liq (${:.0}) exceeds FDV (${:.0}) - {:.0}% ratio",
                token.liquidity_usd, token.fdv, ratio
            ),
        );
    }

    // 3. CRITICAL: Minimum volume floor (applies to ALL tokens, even blue chips)
    // A token with zero volume indicates stale data or a dead market
    if token.volume_24h_usd < 10_000.0 {
        return (
            false,
            format!("LowVolume: ${:.0}/24h", token.volume_24h_usd),
        );
    }

    // 3b. Extreme volatility check - reject pump-and-dumps
    // Tokens with >200% 24h price change are likely being manipulated
    if token.price_24h_change_percent.abs() > 200.0 {
        return (
            false,
            format!(
                "ExtremeVolatility: {:.1}% price change in 24h (likely pump-and-dump)",
                token.price_24h_change_percent
            ),
        );
    }

    // 3c. Volume drop during price spike - pump losing steam
    // If price is up >100% but volume is dropping >30%, avoid it
    if token.price_24h_change_percent > 100.0 && token.volume_24h_change_percent < -30.0 {
        return (
            false,
            format!(
                "PumpLosingSteam: Price +{:.1}%, Volume {:.1}% (pump exhausted)",
                token.price_24h_change_percent, token.volume_24h_change_percent
            ),
        );
    }

    // 4. Blue chip: Very high liquidity + market cap
    if token.liquidity_usd > 10_000_000.0 && token.fdv > 100_000_000.0 {
        return (true, "BlueChip: $10M+ liq, $100M+ FDV".to_string());
    }

    // 5. Established: Good liquidity + decent market cap + good rank
    // Rank must be valid (1-499, not 0 or max value)
    if token.liquidity_usd > 1_000_000.0
        && token.fdv > 10_000_000.0
        && token.rank > 0
        && token.rank < 500
    {
        return (
            true,
            "Established: $1M+ liq, $10M+ FDV, rank < 500".to_string(),
        );
    }

    // 6. Check liquidity ratio
    // Healthy tokens have liquidity > 0.5% of FDV
    if token.fdv > 0.0 {
        let liquidity_ratio = token.liquidity_usd / token.fdv;
        if liquidity_ratio < 0.005 {
            // Less than 0.5%
            return (
                false,
                format!(
                    "IlliquidToken: Liq ${:.0} is only {:.3}% of ${:.0} FDV",
                    token.liquidity_usd,
                    liquidity_ratio * 100.0,
                    token.fdv
                ),
            );
        }
    }

    // 7. Volume sanity check - if volume is too high relative to liquidity, suspicious
    let volume_liquidity_ratio = token.volume_24h_usd / token.liquidity_usd.max(1.0);
    if volume_liquidity_ratio > 50.0 {
        return (
            false,
            format!(
                "Suspicious: volume {}x liquidity",
                volume_liquidity_ratio as u32
            ),
        );
    }

    // 8. Minimum liquidity floor
    if token.liquidity_usd < 100_000.0 {
        return (false, format!("LowLiquidity: ${:.0}", token.liquidity_usd));
    }

    // Passed basic checks but not enough signals for "safe"
    (false, "Insufficient safety signals".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blue_chip_token() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "SOL".to_string(),
            name: "Solana".to_string(),
            decimals: 9,
            liquidity_usd: 50_000_000.0,
            volume_24h_usd: 100_000_000.0,
            volume_24h_change_percent: 5.0,
            fdv: 500_000_000.0,
            marketcap: 500_000_000.0,
            rank: 5,
            price: 100.0,
            price_24h_change_percent: 2.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(is_safe);
        assert!(reason.contains("BlueChip"));
    }

    #[test]
    fn test_low_liquidity_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "SCAM".to_string(),
            name: "Scam Token".to_string(),
            decimals: 9,
            liquidity_usd: 50_000.0, // Below $100k threshold
            volume_24h_usd: 10_000.0,
            volume_24h_change_percent: 5.0,
            fdv: 1_000_000.0,
            marketcap: 1_000_000.0,
            rank: 500,
            price: 1.0,
            price_24h_change_percent: 2.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(!is_safe);
        assert!(reason.contains("LowLiquidity"));
    }

    #[test]
    fn test_liquidity_exceeds_fdv_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "WEIRD".to_string(),
            name: "Weird Token".to_string(),
            decimals: 9,
            liquidity_usd: 10_000_000.0,
            volume_24h_usd: 100_000.0,
            volume_24h_change_percent: 5.0,
            fdv: 5_000_000.0, // Liquidity > FDV (suspicious!)
            marketcap: 5_000_000.0,
            rank: 100,
            price: 1.0,
            price_24h_change_percent: 2.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(!is_safe);
        assert!(reason.contains("SuspiciousData"));
    }

    #[test]
    fn test_low_volume_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "DEAD".to_string(),
            name: "Dead Token".to_string(),
            decimals: 9,
            liquidity_usd: 500_000.0,
            volume_24h_usd: 5_000.0, // Below $10k threshold
            volume_24h_change_percent: 5.0,
            fdv: 10_000_000.0,
            marketcap: 10_000_000.0,
            rank: 200,
            price: 1.0,
            price_24h_change_percent: 2.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(!is_safe);
        assert!(reason.contains("LowVolume"));
    }

    #[test]
    fn test_illiquid_ratio_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "THIN".to_string(),
            name: "Thin Token".to_string(),
            decimals: 9,
            liquidity_usd: 100_000.0,
            volume_24h_usd: 50_000.0,
            volume_24h_change_percent: 5.0,
            fdv: 100_000_000.0, // Only 0.1% liquidity ratio
            marketcap: 100_000_000.0,
            rank: 300,
            price: 1.0,
            price_24h_change_percent: 2.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(!is_safe);
        assert!(reason.contains("IlliquidToken"));
    }

    #[test]
    fn test_established_token_passes() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "JUP".to_string(),
            name: "Jupiter".to_string(),
            decimals: 6,
            liquidity_usd: 5_000_000.0,
            volume_24h_usd: 10_000_000.0,
            volume_24h_change_percent: 5.0,
            fdv: 500_000_000.0, // 1% liquidity ratio (healthy)
            marketcap: 500_000_000.0,
            rank: 10,
            price: 0.5,
            price_24h_change_percent: 2.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(is_safe);
        assert!(reason.contains("Established"));
    }

    #[test]
    fn test_blue_chip_with_zero_volume_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "STALE".to_string(),
            name: "Stale Blue Chip".to_string(),
            decimals: 9,
            liquidity_usd: 50_000_000.0, // Blue chip liquidity
            volume_24h_usd: 0.0,         // But zero volume!
            volume_24h_change_percent: 0.0,
            fdv: 500_000_000.0,
            marketcap: 500_000_000.0,
            rank: 5,
            price: 100.0,
            price_24h_change_percent: 0.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(!is_safe, "Blue chip with zero volume should be rejected");
        assert!(reason.contains("LowVolume"));
    }

    #[test]
    fn test_negative_liquidity_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "BAD".to_string(),
            name: "Bad Data".to_string(),
            decimals: 9,
            liquidity_usd: -1000.0, // Negative!
            volume_24h_usd: 100_000.0,
            volume_24h_change_percent: 5.0,
            fdv: 1_000_000.0,
            marketcap: 1_000_000.0,
            rank: 100,
            price: 1.0,
            price_24h_change_percent: 2.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(!is_safe);
        assert!(reason.contains("InvalidData"));
    }

    #[test]
    fn test_nan_volume_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "NAN".to_string(),
            name: "NaN Token".to_string(),
            decimals: 9,
            liquidity_usd: 1_000_000.0,
            volume_24h_usd: f64::NAN, // NaN!
            volume_24h_change_percent: 5.0,
            fdv: 10_000_000.0,
            marketcap: 10_000_000.0,
            rank: 100,
            price: 1.0,
            price_24h_change_percent: 2.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(!is_safe);
        assert!(reason.contains("InvalidData"));
    }

    #[test]
    fn test_infinite_fdv_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "INF".to_string(),
            name: "Infinite Token".to_string(),
            decimals: 9,
            liquidity_usd: 1_000_000.0,
            volume_24h_usd: 100_000.0,
            volume_24h_change_percent: 5.0,
            fdv: f64::INFINITY, // Infinite!
            marketcap: 10_000_000.0,
            rank: 100,
            price: 1.0,
            price_24h_change_percent: 2.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(!is_safe);
        assert!(reason.contains("InvalidData"));
    }

    #[test]
    fn test_zero_price_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "ZERO".to_string(),
            name: "Zero Price".to_string(),
            decimals: 9,
            liquidity_usd: 1_000_000.0,
            volume_24h_usd: 100_000.0,
            volume_24h_change_percent: 5.0,
            fdv: 10_000_000.0,
            marketcap: 10_000_000.0,
            rank: 100,
            price: 0.0, // Zero price!
            price_24h_change_percent: 2.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(!is_safe);
        assert!(reason.contains("InvalidData"));
    }

    #[test]
    fn test_extreme_volatility_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "PUMP".to_string(),
            name: "Pump Token".to_string(),
            decimals: 9,
            liquidity_usd: 5_000_000.0,
            volume_24h_usd: 10_000_000.0,
            volume_24h_change_percent: 50.0,
            fdv: 50_000_000.0,
            marketcap: 50_000_000.0,
            rank: 10,
            price: 1.0,
            price_24h_change_percent: 624.0, // 624% in 24h = pump
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(!is_safe);
        assert!(reason.contains("ExtremeVolatility"));
    }

    #[test]
    fn test_pump_losing_steam_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "DUMP".to_string(),
            name: "Dump Token".to_string(),
            decimals: 9,
            liquidity_usd: 2_000_000.0,
            volume_24h_usd: 5_000_000.0,
            volume_24h_change_percent: -47.5, // Volume dropping
            fdv: 20_000_000.0,
            marketcap: 20_000_000.0,
            rank: 50,
            price: 1.0,
            price_24h_change_percent: 150.0, // But price still up
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(!is_safe);
        assert!(reason.contains("PumpLosingSteam"));
    }

    #[test]
    fn test_moderate_growth_accepted() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "GOOD".to_string(),
            name: "Good Growth".to_string(),
            decimals: 9,
            liquidity_usd: 5_000_000.0,
            volume_24h_usd: 10_000_000.0,
            volume_24h_change_percent: 50.0,
            fdv: 50_000_000.0,
            marketcap: 50_000_000.0,
            rank: 10,
            price: 1.0,
            price_24h_change_percent: 25.0, // Moderate 25% growth is fine
        };

        let (is_safe, reason) = is_safe_token(&token);
        assert!(is_safe);
        assert!(reason.contains("Established"));
    }

    #[test]
    fn test_rank_zero_rejected() {
        let token = TrendingToken {
            address: "test".to_string(),
            symbol: "RANK0".to_string(),
            name: "Rank Zero".to_string(),
            decimals: 9,
            liquidity_usd: 5_000_000.0,
            volume_24h_usd: 10_000_000.0,
            volume_24h_change_percent: 5.0,
            fdv: 100_000_000.0,
            marketcap: 100_000_000.0,
            rank: 0, // Invalid rank!
            price: 1.0,
            price_24h_change_percent: 2.0,
        };

        let (is_safe, reason) = is_safe_token(&token);
        // Should not pass as "Established" due to rank = 0
        // But might still pass as blue chip if it meets those criteria
        if is_safe {
            // If it passed, it must be due to blue chip status, not established
            assert!(reason.contains("BlueChip") || reason.contains("Established"));
            // Actually, it shouldn't pass as established because rank is 0
            assert!(!reason.contains("Established"));
        }
    }
}
