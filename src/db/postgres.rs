use crate::execution::{ExitReason, Position, PositionStatus};
use crate::Result;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use uuid::Uuid;

/// Default user ID for single-user mode
pub const DEFAULT_USER_ID: Uuid = Uuid::from_u128(1);

/// Postgres persistence for positions
pub struct PostgresPersistence {
    pool: PgPool,
    user_id: Uuid,
}

/// Data for saving a tracked token (SYSTEM-LEVEL)
pub struct TrackedTokenData<'a> {
    pub symbol: &'a str,
    pub address: &'a str,
    pub name: &'a str,
    pub decimals: u8,
    pub strategy_type: &'a str,
}

impl PostgresPersistence {
    /// Connect to Postgres
    ///
    /// # Arguments
    /// * `database_url` - Postgres connection URL
    /// * `user_id` - Optional user ID (defaults to single-user mode)
    pub async fn new(database_url: &str, user_id: Option<Uuid>) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        // Run migrations
        sqlx::migrate!("./migrations").run(&pool).await?;

        tracing::info!("Connected to Postgres at {}", database_url);

        Ok(Self {
            pool,
            user_id: user_id.unwrap_or(DEFAULT_USER_ID),
        })
    }

    /// Save position to Postgres
    pub async fn save_position(&self, position: &Position) -> Result<()> {
        let status_str = match position.status {
            PositionStatus::Open => "Open",
            PositionStatus::Closed => "Closed",
        };

        let exit_reason_str = position.exit_reason.as_ref().map(|r| match r {
            ExitReason::StopLoss => "StopLoss",
            ExitReason::TakeProfit => "TakeProfit",
            ExitReason::TimeStop => "TimeStop",
            ExitReason::Manual => "Manual",
        });

        sqlx::query(
            r#"
            INSERT INTO positions (
                id, user_id, token, entry_price, quantity, entry_time,
                stop_loss, take_profit, trailing_high, status,
                realized_pnl, exit_price, exit_time, exit_reason
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT (id) DO UPDATE SET
                status = EXCLUDED.status,
                take_profit = EXCLUDED.take_profit,
                trailing_high = EXCLUDED.trailing_high,
                realized_pnl = EXCLUDED.realized_pnl,
                exit_price = EXCLUDED.exit_price,
                exit_time = EXCLUDED.exit_time,
                exit_reason = EXCLUDED.exit_reason,
                updated_at = NOW()
            "#,
        )
        .bind(position.id)
        .bind(self.user_id)
        .bind(&position.token)
        .bind(position.entry_price)
        .bind(position.quantity)
        .bind(position.entry_time)
        .bind(position.stop_loss)
        .bind(position.take_profit)
        .bind(position.trailing_high)
        .bind(status_str)
        .bind(position.realized_pnl)
        .bind(position.exit_price)
        .bind(position.exit_time)
        .bind(exit_reason_str)
        .execute(&self.pool)
        .await?;

        tracing::debug!(
            "Saved position {} for {} to Postgres",
            position.id,
            position.token
        );

        Ok(())
    }

    /// Load all positions for the user
    pub async fn load_positions(&self) -> Result<Vec<Position>> {
        let rows = sqlx::query(
            r#"
            SELECT id, token, entry_price, quantity, entry_time,
                   stop_loss, take_profit, trailing_high, status,
                   realized_pnl, exit_price, exit_time, exit_reason
            FROM positions
            WHERE user_id = $1
            ORDER BY entry_time ASC
            "#,
        )
        .bind(self.user_id)
        .fetch_all(&self.pool)
        .await?;

        let mut positions = Vec::new();

        for row in rows {
            let id: Uuid = row.get("id");
            let token: String = row.get("token");
            let entry_price: rust_decimal::Decimal = row.get("entry_price");
            let quantity: rust_decimal::Decimal = row.get("quantity");
            let entry_time: DateTime<Utc> = row.get("entry_time");
            let stop_loss: rust_decimal::Decimal = row.get("stop_loss");
            let take_profit: Option<rust_decimal::Decimal> = row.get("take_profit");
            let trailing_high: rust_decimal::Decimal = row.get("trailing_high");
            let status_str: String = row.get("status");
            let realized_pnl: Option<rust_decimal::Decimal> = row.get("realized_pnl");
            let exit_price: Option<rust_decimal::Decimal> = row.get("exit_price");
            let exit_time: Option<DateTime<Utc>> = row.get("exit_time");
            let exit_reason_str: Option<String> = row.get("exit_reason");

            let status = match status_str.as_str() {
                "Open" => PositionStatus::Open,
                "Closed" => PositionStatus::Closed,
                _ => return Err("Invalid position status".into()),
            };

            let exit_reason = match exit_reason_str.as_deref() {
                Some("StopLoss") => Some(ExitReason::StopLoss),
                Some("TakeProfit") => Some(ExitReason::TakeProfit),
                Some("TimeStop") => Some(ExitReason::TimeStop),
                Some("Manual") => Some(ExitReason::Manual),
                None => None,
                _ => return Err("Invalid exit reason".into()),
            };

            positions.push(Position {
                id,
                token,
                entry_price: entry_price.to_string().parse()?,
                quantity: quantity.to_string().parse()?,
                entry_time,
                stop_loss: stop_loss.to_string().parse()?,
                take_profit: take_profit.map(|v| v.to_string().parse()).transpose()?,
                trailing_high: trailing_high.to_string().parse()?,
                status,
                realized_pnl: realized_pnl.map(|v| v.to_string().parse()).transpose()?,
                exit_price: exit_price.map(|v| v.to_string().parse()).transpose()?,
                exit_time,
                exit_reason,
            });
        }

        tracing::info!("Loaded {} positions from Postgres", positions.len());

        Ok(positions)
    }

    /// Load positions from last N days
    pub async fn load_recent_positions(&self, days: i64) -> Result<Vec<Position>> {
        let cutoff = Utc::now() - chrono::Duration::days(days);

        let rows = sqlx::query(
            r#"
            SELECT id, token, entry_price, quantity, entry_time,
                   stop_loss, take_profit, trailing_high, status,
                   realized_pnl, exit_price, exit_time, exit_reason
            FROM positions
            WHERE user_id = $1 AND entry_time >= $2
            ORDER BY entry_time ASC
            "#,
        )
        .bind(self.user_id)
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await?;

        let mut positions = Vec::new();

        for row in rows {
            let id: Uuid = row.get("id");
            let token: String = row.get("token");
            let entry_price: rust_decimal::Decimal = row.get("entry_price");
            let quantity: rust_decimal::Decimal = row.get("quantity");
            let entry_time: DateTime<Utc> = row.get("entry_time");
            let stop_loss: rust_decimal::Decimal = row.get("stop_loss");
            let take_profit: Option<rust_decimal::Decimal> = row.get("take_profit");
            let trailing_high: rust_decimal::Decimal = row.get("trailing_high");
            let status_str: String = row.get("status");
            let realized_pnl: Option<rust_decimal::Decimal> = row.get("realized_pnl");
            let exit_price: Option<rust_decimal::Decimal> = row.get("exit_price");
            let exit_time: Option<DateTime<Utc>> = row.get("exit_time");
            let exit_reason_str: Option<String> = row.get("exit_reason");

            let status = match status_str.as_str() {
                "Open" => PositionStatus::Open,
                "Closed" => PositionStatus::Closed,
                _ => return Err("Invalid position status".into()),
            };

            let exit_reason = match exit_reason_str.as_deref() {
                Some("StopLoss") => Some(ExitReason::StopLoss),
                Some("TakeProfit") => Some(ExitReason::TakeProfit),
                Some("TimeStop") => Some(ExitReason::TimeStop),
                Some("Manual") => Some(ExitReason::Manual),
                None => None,
                _ => return Err("Invalid exit reason".into()),
            };

            positions.push(Position {
                id,
                token,
                entry_price: entry_price.to_string().parse()?,
                quantity: quantity.to_string().parse()?,
                entry_time,
                stop_loss: stop_loss.to_string().parse()?,
                take_profit: take_profit.map(|v| v.to_string().parse()).transpose()?,
                trailing_high: trailing_high.to_string().parse()?,
                status,
                realized_pnl: realized_pnl.map(|v| v.to_string().parse()).transpose()?,
                exit_price: exit_price.map(|v| v.to_string().parse()).transpose()?,
                exit_time,
                exit_reason,
            });
        }

        tracing::info!(
            "Loaded {} positions from last {} days",
            positions.len(),
            days
        );

        Ok(positions)
    }

    /// Get total realized P&L for user
    pub async fn get_total_pnl(&self) -> Result<f64> {
        let row = sqlx::query(
            r#"
            SELECT COALESCE(SUM(realized_pnl), 0) as total_pnl
            FROM positions
            WHERE user_id = $1 AND status = 'Closed'
            "#,
        )
        .bind(self.user_id)
        .fetch_one(&self.pool)
        .await?;

        let total_pnl: rust_decimal::Decimal = row.get("total_pnl");
        Ok(total_pnl.to_string().parse()?)
    }

    /// Delete all positions for user (testing only)
    #[cfg(test)]
    pub async fn clear_all_positions(&self) -> Result<()> {
        sqlx::query("DELETE FROM positions WHERE user_id = $1")
            .bind(self.user_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Save tracked token to database (SYSTEM-LEVEL)
    /// If token already exists (same address), updates it
    pub async fn save_tracked_token(&self, data: TrackedTokenData<'_>) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO tracked_tokens (symbol, address, name, decimals, strategy_type, status)
            VALUES ($1, $2, $3, $4, $5, 'active')
            ON CONFLICT (address) DO UPDATE SET
                symbol = EXCLUDED.symbol,
                name = EXCLUDED.name,
                decimals = EXCLUDED.decimals,
                strategy_type = EXCLUDED.strategy_type,
                status = 'active',
                updated_at = NOW()
            "#,
        )
        .bind(data.symbol)
        .bind(data.address)
        .bind(data.name)
        .bind(data.decimals as i16)
        .bind(data.strategy_type)
        .execute(&self.pool)
        .await?;

        tracing::debug!(
            "Saved tracked token {} ({}) to Postgres",
            data.symbol,
            data.address
        );

        Ok(())
    }

    /// Load all active tracked tokens (SYSTEM-LEVEL)
    /// Returns: (symbol, address, name, decimals)
    pub async fn load_tracked_tokens(&self) -> Result<Vec<(String, String, String, u8)>> {
        let rows = sqlx::query(
            r#"
            SELECT symbol, address, name, decimals
            FROM tracked_tokens
            WHERE status = 'active'
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut tokens = Vec::new();

        for row in rows {
            let symbol: String = row.get("symbol");
            let address: String = row.get("address");
            let name: String = row.get("name");
            let decimals: i16 = row.get("decimals");
            tokens.push((symbol, address, name, decimals as u8));
        }

        tracing::info!("Loaded {} tracked tokens from Postgres", tokens.len());

        Ok(tokens)
    }

    /// Delete all tracked tokens (testing only)
    #[cfg(test)]
    pub async fn clear_all_tracked_tokens(&self) -> Result<()> {
        sqlx::query("DELETE FROM tracked_tokens")
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get_test_db() -> PostgresPersistence {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://localhost/cryptobot_test".to_string());

        PostgresPersistence::new(&database_url, None)
            .await
            .expect("Failed to connect to test database")
    }

    #[tokio::test]
    #[ignore] // Requires Postgres running
    async fn test_save_and_load_position() {
        let db = get_test_db().await;
        db.clear_all_positions().await.unwrap();

        let position = Position {
            id: Uuid::new_v4(),
            token: "SOL".to_string(),
            entry_price: 100.0,
            quantity: 2.0,
            entry_time: Utc::now(),
            stop_loss: 92.0,
            take_profit: None,
            trailing_high: 100.0,
            status: PositionStatus::Open,
            realized_pnl: None,
            exit_price: None,
            exit_time: None,
            exit_reason: None,
        };

        db.save_position(&position).await.unwrap();

        let positions = db.load_positions().await.unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].id, position.id);
        assert_eq!(positions[0].token, "SOL");
        assert_eq!(positions[0].entry_price, 100.0);

        db.clear_all_positions().await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Postgres running
    async fn test_save_multiple_positions() {
        let db = get_test_db().await;
        db.clear_all_positions().await.unwrap();

        let pos1 = Position {
            id: Uuid::new_v4(),
            token: "SOL".to_string(),
            entry_price: 100.0,
            quantity: 2.0,
            entry_time: Utc::now() - chrono::Duration::hours(2),
            stop_loss: 92.0,
            take_profit: None,
            trailing_high: 100.0,
            status: PositionStatus::Open,
            realized_pnl: None,
            exit_price: None,
            exit_time: None,
            exit_reason: None,
        };

        let pos2 = Position {
            id: Uuid::new_v4(),
            token: "JUP".to_string(),
            entry_price: 1.0,
            quantity: 100.0,
            entry_time: Utc::now() - chrono::Duration::hours(1),
            stop_loss: 0.92,
            take_profit: Some(1.14),
            trailing_high: 1.20,
            status: PositionStatus::Closed,
            realized_pnl: Some(20.0),
            exit_price: Some(1.20),
            exit_time: Some(Utc::now()),
            exit_reason: Some(ExitReason::TakeProfit),
        };

        db.save_position(&pos1).await.unwrap();
        db.save_position(&pos2).await.unwrap();

        let positions = db.load_positions().await.unwrap();
        assert_eq!(positions.len(), 2);
        assert_eq!(positions[0].token, "SOL");
        assert_eq!(positions[1].token, "JUP");
        assert_eq!(positions[1].status, PositionStatus::Closed);
        assert_eq!(positions[1].realized_pnl, Some(20.0));

        db.clear_all_positions().await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Postgres running
    async fn test_update_position() {
        let db = get_test_db().await;
        db.clear_all_positions().await.unwrap();

        let mut position = Position {
            id: Uuid::new_v4(),
            token: "SOL".to_string(),
            entry_price: 100.0,
            quantity: 2.0,
            entry_time: Utc::now(),
            stop_loss: 92.0,
            take_profit: None,
            trailing_high: 100.0,
            status: PositionStatus::Open,
            realized_pnl: None,
            exit_price: None,
            exit_time: None,
            exit_reason: None,
        };

        db.save_position(&position).await.unwrap();

        // Close the position
        position.status = PositionStatus::Closed;
        position.exit_price = Some(110.0);
        position.exit_time = Some(Utc::now());
        position.exit_reason = Some(ExitReason::TakeProfit);
        position.realized_pnl = Some(20.0);

        db.save_position(&position).await.unwrap();

        let positions = db.load_positions().await.unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].status, PositionStatus::Closed);
        assert_eq!(positions[0].realized_pnl, Some(20.0));

        db.clear_all_positions().await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Postgres running
    async fn test_load_recent_positions() {
        let db = get_test_db().await;
        db.clear_all_positions().await.unwrap();

        let old_position = Position {
            id: Uuid::new_v4(),
            token: "OLD".to_string(),
            entry_price: 100.0,
            quantity: 1.0,
            entry_time: Utc::now() - chrono::Duration::days(10),
            stop_loss: 92.0,
            take_profit: None,
            trailing_high: 100.0,
            status: PositionStatus::Closed,
            realized_pnl: Some(10.0),
            exit_price: Some(110.0),
            exit_time: Some(Utc::now() - chrono::Duration::days(9)),
            exit_reason: Some(ExitReason::TakeProfit),
        };

        let recent_position = Position {
            id: Uuid::new_v4(),
            token: "NEW".to_string(),
            entry_price: 100.0,
            quantity: 1.0,
            entry_time: Utc::now() - chrono::Duration::days(1),
            stop_loss: 92.0,
            take_profit: None,
            trailing_high: 100.0,
            status: PositionStatus::Open,
            realized_pnl: None,
            exit_price: None,
            exit_time: None,
            exit_reason: None,
        };

        db.save_position(&old_position).await.unwrap();
        db.save_position(&recent_position).await.unwrap();

        let positions = db.load_recent_positions(7).await.unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].token, "NEW");

        let all_positions = db.load_recent_positions(30).await.unwrap();
        assert_eq!(all_positions.len(), 2);

        db.clear_all_positions().await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Postgres running
    async fn test_get_total_pnl() {
        let db = get_test_db().await;
        db.clear_all_positions().await.unwrap();

        let pos1 = Position {
            id: Uuid::new_v4(),
            token: "SOL".to_string(),
            entry_price: 100.0,
            quantity: 2.0,
            entry_time: Utc::now() - chrono::Duration::hours(2),
            stop_loss: 92.0,
            take_profit: None,
            trailing_high: 110.0,
            status: PositionStatus::Closed,
            realized_pnl: Some(20.0),
            exit_price: Some(110.0),
            exit_time: Some(Utc::now()),
            exit_reason: Some(ExitReason::TakeProfit),
        };

        let pos2 = Position {
            id: Uuid::new_v4(),
            token: "JUP".to_string(),
            entry_price: 1.0,
            quantity: 100.0,
            entry_time: Utc::now() - chrono::Duration::hours(1),
            stop_loss: 0.92,
            take_profit: None,
            trailing_high: 1.0,
            status: PositionStatus::Closed,
            realized_pnl: Some(-8.0),
            exit_price: Some(0.92),
            exit_time: Some(Utc::now()),
            exit_reason: Some(ExitReason::StopLoss),
        };

        db.save_position(&pos1).await.unwrap();
        db.save_position(&pos2).await.unwrap();

        let total_pnl = db.get_total_pnl().await.unwrap();
        assert_eq!(total_pnl, 12.0); // 20 + (-8)

        db.clear_all_positions().await.unwrap();
    }
}
