-- Create positions table
CREATE TABLE positions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token VARCHAR(50) NOT NULL,
    entry_price DECIMAL(20, 8) NOT NULL,
    quantity DECIMAL(20, 8) NOT NULL,
    entry_time TIMESTAMPTZ NOT NULL,
    stop_loss DECIMAL(20, 8) NOT NULL,
    take_profit DECIMAL(20, 8),
    trailing_high DECIMAL(20, 8) NOT NULL,
    status VARCHAR(20) NOT NULL CHECK (status IN ('Open', 'Closed')),
    realized_pnl DECIMAL(20, 8),
    exit_price DECIMAL(20, 8),
    exit_time TIMESTAMPTZ,
    exit_reason VARCHAR(20) CHECK (exit_reason IN ('StopLoss', 'TakeProfit', 'TimeStop', 'Manual')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes for efficient queries
CREATE INDEX idx_positions_user_id ON positions(user_id);
CREATE INDEX idx_positions_status ON positions(status);
CREATE INDEX idx_positions_entry_time ON positions(entry_time DESC);
CREATE INDEX idx_positions_user_status ON positions(user_id, status);
