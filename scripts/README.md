# Scripts

Helper scripts for managing the CryptoBot application.

## backfill_all_tokens.sh

Backfills historical price data for all active tokens in the database using CoinGecko API.

### Prerequisites

1. **CoinGecko API Key**: Required for fetching historical data
2. **PostgreSQL**: Database must be running with tracked tokens
3. **Redis**: Must be running to store candle data

### Usage

```bash
# Backfill 90 days (default)
./scripts/backfill_all_tokens.sh

# Backfill custom number of days
./scripts/backfill_all_tokens.sh 180

# With explicit environment variables
COINGECKO_API_KEY=your_key_here \
DATABASE_URL="postgres://user:pass@localhost:5432/cryptobot" \
REDIS_URL="redis://127.0.0.1:6379" \
./scripts/backfill_all_tokens.sh 90
```

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `COINGECKO_API_KEY` | Yes | - | Your CoinGecko API key |
| `DATABASE_URL` | No | `postgres://cryptobot:cryptobot_dev_password@localhost:5432/cryptobot` | PostgreSQL connection string |
| `REDIS_URL` | No | `redis://127.0.0.1:6379` | Redis connection string |

### Features

- ✅ Automatically fetches all active tokens from database
- ✅ Configurable backfill period (default 90 days)
- ✅ Progress tracking with colored output
- ✅ Error handling and failure reporting
- ✅ Rate limiting (2 second delay between tokens)
- ✅ Summary report at end
- ✅ Non-zero exit code on failures

### Output

The script provides:
- Real-time progress for each token
- Success/failure status with colored indicators
- Final summary showing successful and failed tokens
- List of any failed tokens for retry

### Example

```bash
$ COINGECKO_API_KEY=your_key ./scripts/backfill_all_tokens.sh 90

╔════════════════════════════════════════════════════════════════╗
║           BACKFILL HISTORICAL DATA FOR ALL TOKENS              ║
╚════════════════════════════════════════════════════════════════╝

Configuration:
  Days to backfill: 90
  Database: postgres://cryptobot:***@localhost:5432/cryptobot
  Redis: redis://127.0.0.1:6379

📊 Fetching active tokens from database...
Found 10 active tokens

════════════════════════════════════════════════════════════════
[1/10] Processing: Solana (SOL)
════════════════════════════════════════════════════════════════
  Address: So11111111111111111111111111111111111111112
  Backfilling 90 days of data...

[... backfill output ...]

✓ Successfully backfilled SOL

⏳ Waiting 2 seconds before next token...

[... continues for all tokens ...]

════════════════════════════════════════════════════════════════
BACKFILL SUMMARY
════════════════════════════════════════════════════════════════
✓ Successful: 10
✗ Failed: 0

════════════════════════════════════════════════════════════════
```

### Notes

- **CoinGecko Rate Limits**: Free tier has strict rate limits. The script includes 2-second delays between tokens.
- **Data Granularity**: CoinGecko provides hourly data for 90-day periods
- **Existing Data**: The backfill command will overwrite existing data in Redis
- **API Key**: Get a free API key at https://www.coingecko.com/en/api/pricing

### Troubleshooting

**Error: COINGECKO_API_KEY not set**
```bash
export COINGECKO_API_KEY="your_key_here"
./scripts/backfill_all_tokens.sh
```

**Error: No active tokens found**
- Ensure your database has tokens with `status = 'active'`
- Check your DATABASE_URL is correct

**Individual token failures**
- Check CoinGecko API limits (free tier: 30 calls/minute)
- Verify token addresses are correct in database
- Check CoinGecko supports the token (not all Solana tokens have historical data)

### Related Commands

After backfilling, you may want to:

```bash
# Run RSI parameter tuning
DATABASE_URL="..." cargo run --bin tune_rsi

# Run comprehensive backtests
DATABASE_URL="..." cargo run --bin backtest_real
```
