-- Birdeye Discovery System Schema (CLEANED UP)
-- Migration 003: Token discovery (SYSTEM-LEVEL) + performance tracking (PER-USER)
--
-- Architecture:
-- - Token discovery is SYSTEM-LEVEL: all users share the same discovered tokens
-- - Each token has ONE strategy type (system-level, not per-user)
-- - Performance tracking is PER-USER: each user has their own positions and P&L

-- Drop old bloated tables
DROP TABLE IF EXISTS token_scores CASCADE;
DROP TABLE IF EXISTS discovery_snapshots CASCADE;
DROP TABLE IF EXISTS strategy_performance CASCADE;
DROP TABLE IF EXISTS tracked_tokens CASCADE;
DROP TRIGGER IF EXISTS update_tracked_tokens_updated_at ON tracked_tokens;
DROP FUNCTION IF EXISTS update_updated_at_column();

-- ============================================
-- Table: tracked_tokens (SYSTEM-LEVEL)
-- Purpose: Tokens the system is actively monitoring (shared across all users)
-- Each token has ONE strategy assigned at system level
-- ============================================
CREATE TABLE IF NOT EXISTS tracked_tokens (
    id SERIAL PRIMARY KEY,
    symbol VARCHAR(20) NOT NULL,
    address VARCHAR(44) NOT NULL UNIQUE,  -- Solana address (globally unique)
    name VARCHAR(100),

    -- Status
    status VARCHAR(20) NOT NULL DEFAULT 'active',  -- 'active', 'paused', 'removed'

    -- Strategy assignment (SYSTEM-LEVEL: one strategy per token for all users)
    strategy_type VARCHAR(50) NOT NULL DEFAULT 'momentum',  -- 'momentum', 'aggressive_momentum', etc.
    strategy_config JSONB,  -- Optional: store SignalConfig as JSON for per-token tuning

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_tracked_tokens_status ON tracked_tokens(status);
CREATE INDEX idx_tracked_tokens_address ON tracked_tokens(address);

-- ============================================
-- Table: strategy_performance (PER USER)
-- Purpose: Track how well each strategy works per token/category per user
-- This is the ONLY per-user analytics table we actually need
-- ============================================
CREATE TABLE IF NOT EXISTS strategy_performance (
    id SERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_address VARCHAR(44) NOT NULL,
    strategy_type VARCHAR(50) NOT NULL,

    -- Performance window
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL,

    -- Metrics
    trades_count INTEGER DEFAULT 0,
    win_count INTEGER DEFAULT 0,
    loss_count INTEGER DEFAULT 0,
    win_rate DECIMAL(5, 4),  -- 0-1

    avg_gain_pct DECIMAL(10, 4),
    avg_loss_pct DECIMAL(10, 4),
    max_gain_pct DECIMAL(10, 4),
    max_loss_pct DECIMAL(10, 4),

    total_pnl_usd DECIMAL(20, 2),
    sharpe_ratio DECIMAL(10, 4),
    max_drawdown_pct DECIMAL(10, 4),

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_strategy_performance_user ON strategy_performance(user_id);
CREATE INDEX idx_strategy_performance_token ON strategy_performance(user_id, token_address);
CREATE INDEX idx_strategy_performance_window ON strategy_performance(window_start, window_end);

-- ============================================
-- Update Trigger: Auto-update updated_at timestamp
-- ============================================
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

CREATE TRIGGER update_tracked_tokens_updated_at BEFORE UPDATE
    ON tracked_tokens FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- Note: Removed bloat tables
-- - discovery_snapshots (0 rows, never used, not multi-user relevant)
-- - token_scores (0 rows, never used, not multi-user relevant)
--
-- Simplified tracked_tokens to only what's actually used:
-- - Removed: category, fdv_usd, liquidity_usd, discovery_score, discovery_rank
-- - Removed: discovered_at, status_reason, activated_at, deactivated_at, last_scored_at
-- - Kept: Core identity + status + strategy assignment
-- ============================================
