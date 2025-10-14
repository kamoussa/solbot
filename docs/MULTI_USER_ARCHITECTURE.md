# Multi-User Architecture Planning

**Status**: 🚧 Future Work (Tabled for after MVP)

**Date**: 2025-10-14

## Context

Currently the bot is single-user. We want to eventually support multiple users (initially just me + a few friends) for 24/7 automated trading.

## Key Question: Custodial vs Non-Custodial?

### Option 1: Non-Custodial Alerts (Signal Service) 🔔

**How it works:**
- Bot generates signals centrally
- Sends alerts to Telegram/Discord
- Users execute trades manually

**Pros:**
- Zero custody risk
- Simple to implement
- No regulatory concerns

**Cons:**
- ❌ Only works during daytime (users need to be awake)
- ❌ Slower execution = worse prices
- ❌ Users might miss signals
- ❌ Defeats purpose of 24/7 algo trading

**Verdict:** Not suitable for swing trading where timing matters

---

### Option 2: Fully Custodial (Traditional Exchange Model) 🏦

**How it works:**
- Users deposit to single bot wallet
- Bot tracks balances in database
- Bot trades on behalf of all users

**Pros:**
- Simple architecture
- Efficient (one wallet, less fees)

**Cons:**
- ❌ Highest trust requirement
- ❌ Single point of failure
- ❌ Regulatory issues (might be considered money transmission)
- ❌ If bot gets hacked, everyone loses everything

**Verdict:** Too risky for personal project

---

### Option 3: Dedicated Wallets (Recommended) 🔑

**How it works:**
- Each user gets their own Solana wallet
- Bot generates keypair and stores encrypted private key
- Bot trades 24/7 with stored keys
- User can export key and withdraw anytime

**Pros:**
- ✅ 24/7 automated trading
- ✅ User has ultimate control (can export key)
- ✅ Bot can only trade while user trusts it
- ✅ Per-user risk settings
- ✅ Good UX for friends

**Cons:**
- Semi-custodial (requires trust during operation)
- Need good key encryption
- More complex than single-user

**Verdict:** Best for personal project with friends!

**Reference**: Similar to how Trojan bot on Solana works - dedicated wallet per user with exportable keys.

---

## Recommended Implementation Path

### Phase 0: MVP (Current - Single User)

**Goal**: Get one user (me) trading successfully

**Implementation**:
```bash
# .env
WALLET_PRIVATE_KEY=5J7x...  # My trading wallet key
MAX_POSITION_SIZE_PCT=5.0
MAX_DAILY_LOSS_PCT=5.0
```

**Code**:
```rust
// Simple single-user setup
let keypair = Keypair::from_base58_string(&env::var("WALLET_PRIVATE_KEY")?);
let circuit_breakers = CircuitBreakers::from_env();
```

**Duration**: Current focus

---

### Phase 1: Multi-User Core (No UI)

**Goal**: Support 2-5 users with encrypted wallet storage

**Database Schema**:
```sql
CREATE TABLE users (
    id UUID PRIMARY KEY,
    username TEXT UNIQUE NOT NULL,  -- 'kareem', 'friend1', etc.
    wallet_pubkey TEXT NOT NULL,
    encrypted_private_key BYTEA NOT NULL,  -- Encrypted with master key
    portfolio_value NUMERIC NOT NULL DEFAULT 0,
    max_position_size_pct NUMERIC DEFAULT 5.0,
    max_daily_loss_pct NUMERIC DEFAULT 5.0,
    created_at TIMESTAMPTZ NOT NULL,
    is_active BOOLEAN DEFAULT true
);

CREATE TABLE positions (
    id UUID PRIMARY KEY,
    user_id UUID REFERENCES users(id) NOT NULL,
    token TEXT NOT NULL,
    entry_price NUMERIC NOT NULL,
    quantity NUMERIC NOT NULL,
    entry_time TIMESTAMPTZ NOT NULL,
    stop_loss NUMERIC NOT NULL,
    take_profit NUMERIC,
    status TEXT NOT NULL,  -- 'open', 'closed'
    UNIQUE(user_id, token, status) WHERE status = 'open'
);

CREATE TABLE trades (
    id UUID PRIMARY KEY,
    user_id UUID REFERENCES users(id) NOT NULL,
    position_id UUID REFERENCES positions(id),
    token TEXT NOT NULL,
    side TEXT NOT NULL,  -- 'buy', 'sell'
    price NUMERIC NOT NULL,
    quantity NUMERIC NOT NULL,
    pnl NUMERIC,
    timestamp TIMESTAMPTZ NOT NULL
);
```

**Key Management**:
```rust
// src/wallet/encryption.rs
use aes_gcm::{Aes256Gcm, Key, Nonce};
use solana_sdk::signer::keypair::Keypair;

pub struct WalletManager {
    master_key: Key<Aes256Gcm>,  // From MASTER_ENCRYPTION_KEY env var
}

impl WalletManager {
    pub fn create_user_wallet(&self, username: &str) -> Result<(Pubkey, Vec<u8>)> {
        let keypair = Keypair::new();
        let encrypted_key = self.encrypt(&keypair.to_bytes());

        // Store in database
        db.insert_user(User {
            username: username.to_string(),
            wallet_pubkey: keypair.pubkey().to_string(),
            encrypted_private_key: encrypted_key,
            ...
        }).await?;

        Ok((keypair.pubkey(), keypair.to_bytes()))
    }

    pub fn decrypt_for_trading(&self, encrypted: &[u8]) -> Result<Keypair> {
        let decrypted = self.decrypt(encrypted)?;
        Ok(Keypair::from_bytes(&decrypted)?)
    }

    fn encrypt(&self, data: &[u8]) -> Vec<u8> {
        // AES-256-GCM encryption
    }

    fn decrypt(&self, encrypted: &[u8]) -> Result<Vec<u8>> {
        // AES-256-GCM decryption
    }
}
```

**CLI Tools**:
```bash
# Add new user
cargo run --bin add-user -- --username kareem

# Output:
# ✅ Created wallet for kareem
# Address: CxH7...9Qw3
# 🔑 Private Key: 5J7x...  (SAVE THIS!)
# Encrypted and stored in database.

# List users
cargo run --bin list-users

# Export user key (for withdrawal)
cargo run --bin export-key -- --username kareem
```

**Bot Changes**:
```rust
// src/main.rs - Modified main loop
loop {
    ticker.tick().await;

    let results = price_manager.fetch_all().await;

    // Get ALL active users from database
    let users = db.get_active_users().await?;

    for user in users {
        for token in &tokens {
            let candles = price_manager.buffer().get_candles(&token.symbol)?;

            if candles.len() >= samples_needed {
                let signal = strategy.generate_signal(&candles)?;

                // Execute signal for this specific user
                executor.execute_for_user(&user, &signal, &token).await?;
            }
        }

        // Check user's positions for exits
        position_manager.check_exits_for_user(&user).await?;
    }
}
```

**Security**:
```bash
# .env
MASTER_ENCRYPTION_KEY=<32-byte hex string>  # Generate once, never change!
DATABASE_URL=postgresql://...

# Generate master key:
openssl rand -hex 32
```

**Duration**: 2-3 weeks after executor layer is done

---

### Phase 2: Telegram Bot (Optional)

**Goal**: User-friendly interface for friends

**Commands**:
- `/start` - Create wallet
- `/balance` - Show portfolio
- `/positions` - Show open trades
- `/withdraw <address> <amount>` - Withdraw funds
- `/export` - Get private key
- `/risk <level>` - Adjust risk (conservative/moderate/aggressive)
- `/pause` - Pause trading
- `/resume` - Resume trading

**Implementation**:
```rust
// src/telegram/bot.rs
use teloxide::prelude::*;

async fn cmd_start(bot: Bot, msg: Message) -> ResponseResult<()> {
    let telegram_id = msg.from().id.0 as i64;

    // Check if user exists
    if let Some(user) = db.get_user_by_telegram_id(telegram_id).await? {
        bot.send_message(msg.chat.id, format!(
            "Welcome back! Your wallet: {}",
            user.wallet_pubkey
        )).await?;
        return Ok(());
    }

    // Create new wallet
    let (pubkey, private_key) = wallet_manager.create_user_wallet()?;

    db.create_user(User {
        telegram_id,
        wallet_pubkey: pubkey.to_string(),
        encrypted_private_key: wallet_manager.encrypt(&private_key),
        ...
    }).await?;

    bot.send_message(msg.chat.id, format!(
        "🎉 Wallet created!\n\n\
         Address: {}\n\n\
         Deposit SOL to start trading.\n\n\
         ⚠️ Use /export to get your private key.",
        pubkey
    )).await?;

    Ok(())
}

// Similar handlers for other commands...
```

**Duration**: 1-2 weeks

---

## Security Checklist

### Phase 0 (MVP):
- [x] Store private key in environment variable only
- [x] Never commit .env to git
- [x] Implement circuit breakers

### Phase 1 (Multi-User):
- [ ] Encrypt private keys with AES-256-GCM
- [ ] Store master encryption key in environment variable only
- [ ] Never log private keys or decrypted data
- [ ] Audit log all wallet operations
- [ ] Regular backups of encrypted keys
- [ ] Rate limit key decryption operations

### Phase 2 (Telegram Bot):
- [ ] Implement 2FA for sensitive operations
- [ ] Rate limit withdrawals
- [ ] Add withdrawal limits (e.g., max 50% per day)
- [ ] Whitelist withdrawal addresses
- [ ] Alert on suspicious activity

---

## Data Model Evolution

### Phase 0 (MVP):
```rust
// Single user in memory
let circuit_breakers = CircuitBreakers::from_env();
let trading_state = TradingState::new(10000.0);
```

### Phase 1 (Multi-User):
```rust
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub wallet: Pubkey,
    // Private key stored encrypted in DB, decrypted only for trading
    pub portfolio_value: f64,
    pub circuit_breakers: CircuitBreakers,  // Per-user limits
    pub is_active: bool,
}

pub struct Position {
    pub id: Uuid,
    pub user_id: Uuid,  // NEW!
    pub token: String,
    pub entry_price: f64,
    pub quantity: f64,
    // ...
}
```

### Phase 2 (Telegram):
```rust
pub struct User {
    pub id: Uuid,
    pub telegram_id: Option<i64>,  // NEW!
    pub username: String,
    pub wallet: Pubkey,
    pub portfolio_value: f64,
    pub circuit_breakers: CircuitBreakers,
    pub notification_settings: NotificationSettings,  // NEW!
    pub is_active: bool,
}

pub struct NotificationSettings {
    pub notify_on_trade: bool,
    pub notify_on_exit: bool,
    pub notify_on_circuit_breaker: bool,
    pub notify_daily_summary: bool,
}
```

---

## Comparison: Custodial Models

| Feature | Alerts Only | Full Custody | Dedicated Wallets (Recommended) |
|---------|-------------|--------------|--------------------------------|
| 24/7 Trading | ❌ No | ✅ Yes | ✅ Yes |
| User Control | ✅ Full | ❌ None | ⚠️ Can export key |
| Trust Required | Low | High | Medium |
| Implementation | Easy | Easy | Medium |
| Scalability | High | High | High |
| Regulatory Risk | None | High | Low-Medium |
| **Best For** | Day traders | Not recommended | **Friends group** ✅ |

---

## Open Questions

1. **Profit Sharing**: Should bot take a performance fee? (e.g., 10% of profits)
2. **Minimum Deposit**: What's the minimum to trade effectively? (Suggest: 100 SOL)
3. **Risk Tiers**: Should we offer preset risk levels? (Conservative/Moderate/Aggressive)
4. **Withdrawal Limits**: Should we limit withdrawals to prevent panic? (e.g., max 50%/day)
5. **Onboarding**: How do we explain risks to friends?

---

## References

- **Trojan Bot**: Uses dedicated wallets with exportable keys (semi-custodial)
- **GMX Trading Bots**: Use dedicated wallets per user
- **Solana Best Practices**: [Solana Security Best Practices](https://docs.solana.com/developing/programming-model/overview#security)

---

## Decision Log

**2025-10-14**: Decided on dedicated wallets (Option 3) for multi-user support. Rationale:
- 24/7 trading is core value proposition
- User maintains ultimate control via exportable key
- Good balance of trust and convenience for friends
- Scalable architecture

**2025-10-14**: MVP will use single user with private key in environment variable. Rationale:
- Keep it simple for initial development
- Validate core trading logic first
- Add multi-user complexity later
- Faster iteration

---

## Next Steps (When Ready)

1. **Implement executor layer** (single-user first)
2. **Implement position manager** (single-user first)
3. **Test with real money** (my wallet only)
4. **Add database schema** for multi-user
5. **Implement wallet encryption**
6. **Build CLI tools** for user management
7. **Test with 2-3 friends**
8. **(Optional) Build Telegram bot**

**Estimated Timeline**: 4-6 weeks after MVP is stable
