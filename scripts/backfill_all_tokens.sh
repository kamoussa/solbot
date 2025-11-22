#!/bin/bash
#
# Backfill historical price data for all tracked tokens
#
# Usage:
#   ./scripts/backfill_all_tokens.sh [days]
#
# Examples:
#   ./scripts/backfill_all_tokens.sh           # Backfill 90 days (default)
#   ./scripts/backfill_all_tokens.sh 180       # Backfill 180 days
#
# Environment variables:
#   DATABASE_URL         - PostgreSQL connection string (required)
#   COINGECKO_API_KEY    - CoinGecko API key (required)
#   REDIS_URL            - Redis connection string (optional, default: redis://127.0.0.1:6379)

set -e  # Exit on error

# Configuration
DAYS=${1:-90}
DATABASE_URL=${DATABASE_URL:-"postgres://cryptobot:cryptobot_dev_password@localhost:5432/cryptobot"}
REDIS_URL=${REDIS_URL:-"redis://127.0.0.1:6379"}
COINGECKO_API_KEY=${COINGECKO_API_KEY:-""}

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Header
echo ""
echo "╔════════════════════════════════════════════════════════════════╗"
echo "║           BACKFILL HISTORICAL DATA FOR ALL TOKENS              ║"
echo "╚════════════════════════════════════════════════════════════════╝"
echo ""

# Check prerequisites
if [ -z "$COINGECKO_API_KEY" ]; then
    echo -e "${RED}Error: COINGECKO_API_KEY environment variable not set${NC}"
    echo "Please export COINGECKO_API_KEY before running this script"
    exit 1
fi

echo -e "${BLUE}Configuration:${NC}"
echo "  Days to backfill: $DAYS"
echo "  Database: ${DATABASE_URL%%@*}@***"
echo "  Redis: $REDIS_URL"
echo ""

# Get list of active tokens from database
echo -e "${BLUE}📊 Fetching active tokens from database...${NC}"
TOKENS=$(psql "$DATABASE_URL" -t -A -F'|' -c "
    SELECT symbol, name, address
    FROM tracked_tokens
    WHERE status = 'active'
    ORDER BY symbol;
")

if [ -z "$TOKENS" ]; then
    echo -e "${RED}Error: No active tokens found in database${NC}"
    exit 1
fi

# Count tokens
TOKEN_COUNT=$(echo "$TOKENS" | wc -l | tr -d ' ')
echo -e "${GREEN}Found $TOKEN_COUNT active tokens${NC}"
echo ""

# Track success/failure
SUCCESS_COUNT=0
FAILURE_COUNT=0
FAILED_TOKENS=()

# Process each token
CURRENT=0
echo "$TOKENS" | while IFS='|' read -r SYMBOL NAME ADDRESS; do
    CURRENT=$((CURRENT + 1))

    echo "════════════════════════════════════════════════════════════════"
    echo -e "${BLUE}[$CURRENT/$TOKEN_COUNT] Processing: $NAME ($SYMBOL)${NC}"
    echo "════════════════════════════════════════════════════════════════"
    echo "  Address: $ADDRESS"
    echo "  Backfilling $DAYS days of data..."
    echo ""

    # Run backfill command
    if COINGECKO_API_KEY="$COINGECKO_API_KEY" \
       DATABASE_URL="$DATABASE_URL" \
       REDIS_URL="$REDIS_URL" \
       cargo run --bin cryptobot backfill "$SYMBOL" "$ADDRESS" --days "$DAYS"; then
        echo ""
        echo -e "${GREEN}✓ Successfully backfilled $SYMBOL${NC}"
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    else
        echo ""
        echo -e "${RED}✗ Failed to backfill $SYMBOL${NC}"
        FAILURE_COUNT=$((FAILURE_COUNT + 1))
        FAILED_TOKENS+=("$SYMBOL")
    fi

    echo ""

    # Rate limiting: Wait 2 seconds between tokens to avoid API throttling
    if [ "$CURRENT" -lt "$TOKEN_COUNT" ]; then
        echo -e "${YELLOW}⏳ Waiting 2 seconds before next token...${NC}"
        sleep 2
        echo ""
    fi
done

# Summary
echo "════════════════════════════════════════════════════════════════"
echo -e "${BLUE}BACKFILL SUMMARY${NC}"
echo "════════════════════════════════════════════════════════════════"
echo -e "${GREEN}✓ Successful: $SUCCESS_COUNT${NC}"
echo -e "${RED}✗ Failed: $FAILURE_COUNT${NC}"

if [ "$FAILURE_COUNT" -gt 0 ]; then
    echo ""
    echo "Failed tokens:"
    for token in "${FAILED_TOKENS[@]}"; do
        echo "  - $token"
    done
fi

echo ""
echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}"
echo ""

# Exit with error if any failures
if [ "$FAILURE_COUNT" -gt 0 ]; then
    exit 1
fi
