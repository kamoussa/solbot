-- Token Rotation: Track token freshness and lifecycle
-- Migration 005: Add last_seen_trending for automatic token rotation

-- Add last_seen_trending column
-- Tracks when this token was last observed in Birdeye trending list
-- Used to automatically remove stale tokens that haven't been trending recently
ALTER TABLE tracked_tokens
ADD COLUMN IF NOT EXISTS last_seen_trending TIMESTAMPTZ DEFAULT NOW();

-- Set existing tokens to NOW() (assume they're fresh)
UPDATE tracked_tokens
SET last_seen_trending = NOW()
WHERE last_seen_trending IS NULL;

-- Create index for efficient rotation queries
CREATE INDEX IF NOT EXISTS idx_tracked_tokens_last_seen
ON tracked_tokens(last_seen_trending)
WHERE status = 'active';

-- Extended status enum (no ALTER TYPE needed, just documentation):
-- 'active'  - Token is fresh (seen in trending recently or has open positions)
-- 'stale'   - Not seen in 24h, no open positions (stops price fetching)
-- 'removed' - Not seen in 7 days, no open positions (archived)
-- 'paused'  - Manually paused by user

-- Note: We don't use ALTER TYPE to add 'stale' because status is VARCHAR(20),
-- not an ENUM. The application logic will enforce the valid values.
