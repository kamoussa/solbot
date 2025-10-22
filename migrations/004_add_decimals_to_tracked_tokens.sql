-- Add decimals column to tracked_tokens
-- This allows us to fully restore token identity from DB without needing fresh API data

ALTER TABLE tracked_tokens ADD COLUMN IF NOT EXISTS decimals SMALLINT NOT NULL DEFAULT 9;

-- Most Solana tokens use 9 decimals (including SOL)
-- We default to 9 for any existing tokens, which is a safe assumption
-- New tokens will have the correct decimals from Birdeye API
