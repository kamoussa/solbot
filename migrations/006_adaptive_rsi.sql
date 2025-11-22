-- Add RSI threshold column for adaptive per-token configuration
-- Default to 45.0 (median optimal value from backtests)
-- Values can be updated dynamically via tune_rsi command
ALTER TABLE tracked_tokens ADD COLUMN IF NOT EXISTS rsi_threshold REAL DEFAULT 45.0;

-- Add index for faster lookups
CREATE INDEX IF NOT EXISTS idx_tracked_tokens_symbol_rsi ON tracked_tokens(symbol, rsi_threshold);
