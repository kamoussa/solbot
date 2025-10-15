-- Create users table for multi-user support
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username VARCHAR(255) UNIQUE NOT NULL,
    initial_portfolio_value DECIMAL(20, 2) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create default user for single-user operation
INSERT INTO users (id, username, initial_portfolio_value)
VALUES ('00000000-0000-0000-0000-000000000001', 'default', 10000.00);
