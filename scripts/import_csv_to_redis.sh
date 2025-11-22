#!/bin/bash
#
# Import SOL daily candle data from CSV into Redis for backtesting
#
# Usage:
#   ./scripts/import_csv_to_redis.sh [CSV_FILE] [SYMBOL] [REDIS_URL]
#
# Defaults:
#   CSV_FILE:  ./Solana_daily_data_2018_2024.csv
#   SYMBOL:    SOL
#   REDIS_URL: redis://127.0.0.1:6379 (uses redis-cli)
#

set -e  # Exit on error

# Parse arguments
CSV_FILE="${1:-./Solana_daily_data_2018_2024.csv}"
SYMBOL="${2:-SOL}"
REDIS_HOST="${3:-127.0.0.1}"
REDIS_PORT="${4:-6379}"

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo "======================================================================"
echo "CSV TO REDIS IMPORTER"
echo "======================================================================"
echo "CSV File:  $CSV_FILE"
echo "Symbol:    $SYMBOL"
echo "Redis:     $REDIS_HOST:$REDIS_PORT"
echo "======================================================================"
echo ""

# Check if CSV exists
if [ ! -f "$CSV_FILE" ]; then
    echo -e "${RED}❌ CSV file not found: $CSV_FILE${NC}"
    exit 1
fi

# Check if redis-cli is available
if ! command -v redis-cli &> /dev/null; then
    echo -e "${RED}❌ redis-cli not found. Please install redis-cli.${NC}"
    exit 1
fi

# Test Redis connection
if ! redis-cli -h "$REDIS_HOST" -p "$REDIS_PORT" PING > /dev/null 2>&1; then
    echo -e "${RED}❌ Cannot connect to Redis at $REDIS_HOST:$REDIS_PORT${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Connected to Redis${NC}"

# Clear existing data
KEY="snapshots:$SYMBOL"
EXISTING_COUNT=$(redis-cli -h "$REDIS_HOST" -p "$REDIS_PORT" ZCARD "$KEY")

if [ "$EXISTING_COUNT" -gt 0 ]; then
    echo -e "${YELLOW}Clearing $EXISTING_COUNT existing $SYMBOL candles from Redis...${NC}"
    redis-cli -h "$REDIS_HOST" -p "$REDIS_PORT" DEL "$KEY" > /dev/null
else
    echo "No existing $SYMBOL data found in Redis"
fi

# Import CSV data
echo ""
echo "Reading CSV from $CSV_FILE..."

# Count total lines (excluding header)
TOTAL_LINES=$(($(wc -l < "$CSV_FILE") - 1))
echo "Found $TOTAL_LINES data rows"
echo ""

# Process CSV and build Redis commands
IMPORTED=0
SKIPPED=0
BATCH_SIZE=1000
BATCH_COMMANDS=""

# Skip header line, then process each row
tail -n +2 "$CSV_FILE" | while IFS=',' read -r time open high low close volume; do
    # Validate data
    if [ -z "$time" ] || [ -z "$open" ] || [ -z "$high" ] || [ -z "$low" ] || [ -z "$close" ] || [ -z "$volume" ]; then
        ((SKIPPED++))
        continue
    fi

    # Convert date to Unix timestamp (assuming UTC)
    UNIX_TS=$(date -j -f "%Y-%m-%d" "$time" "+%s" 2>/dev/null || echo "")

    if [ -z "$UNIX_TS" ]; then
        # Try GNU date format (Linux)
        UNIX_TS=$(date -d "$time" "+%s" 2>/dev/null || echo "")
    fi

    if [ -z "$UNIX_TS" ]; then
        echo -e "${YELLOW}⚠️  Skipping invalid date: $time${NC}"
        ((SKIPPED++))
        continue
    fi

    # Build JSON candle object (matching our Candle struct)
    CANDLE_JSON=$(cat <<EOF
{"open":$open,"high":$high,"low":$low,"close":$close,"volume":$volume,"timestamp":"$time 00:00:00 UTC"}
EOF
)

    # Add to Redis sorted set
    redis-cli -h "$REDIS_HOST" -p "$REDIS_PORT" ZADD "$KEY" "$UNIX_TS" "$CANDLE_JSON" > /dev/null

    ((IMPORTED++))

    # Progress indicator
    if [ $((IMPORTED % 100)) -eq 0 ]; then
        echo -e "  Imported $IMPORTED candles..."
    fi
done

echo ""
echo -e "${GREEN}✅ Successfully imported $IMPORTED candles for $SYMBOL${NC}"

# Verification
echo ""
echo "📊 Verification:"
FINAL_COUNT=$(redis-cli -h "$REDIS_HOST" -p "$REDIS_PORT" ZCARD "$KEY")
echo "  Redis contains $FINAL_COUNT candles for $SYMBOL"

# Get date range
FIRST_DATA=$(redis-cli -h "$REDIS_HOST" -p "$REDIS_PORT" ZRANGE "$KEY" 0 0 WITHSCORES)
LAST_DATA=$(redis-cli -h "$REDIS_HOST" -p "$REDIS_PORT" ZRANGE "$KEY" -1 -1 WITHSCORES)

if [ -n "$FIRST_DATA" ] && [ -n "$LAST_DATA" ]; then
    # Extract timestamps from Redis output (score is on second line)
    FIRST_TS=$(echo "$FIRST_DATA" | tail -1)
    LAST_TS=$(echo "$LAST_DATA" | tail -1)

    # Convert to dates
    if command -v date &> /dev/null; then
        FIRST_DATE=$(date -r "$FIRST_TS" "+%Y-%m-%d" 2>/dev/null || date -d "@$FIRST_TS" "+%Y-%m-%d" 2>/dev/null)
        LAST_DATE=$(date -r "$LAST_TS" "+%Y-%m-%d" 2>/dev/null || date -d "@$LAST_TS" "+%Y-%m-%d" 2>/dev/null)

        TOTAL_DAYS=$(( (LAST_TS - FIRST_TS) / 86400 ))

        echo "  Date range: $FIRST_DATE to $LAST_DATE"
        echo "  Total days: $TOTAL_DAYS"
    fi
fi

echo ""
echo -e "${GREEN}✅ Import complete! Ready for backtesting.${NC}"
echo ""
echo "Usage examples:"
echo "  # Run backtest with imported data"
echo "  DATABASE_URL=\$DATABASE_URL cargo run --bin backtest_real"
echo ""
echo "  # Check Redis data"
echo "  redis-cli ZCARD snapshots:$SYMBOL"
echo "  redis-cli ZRANGE snapshots:$SYMBOL 0 5"
