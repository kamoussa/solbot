# Deployment Guide

## Prerequisites

- Git
- Docker & Docker Compose
- SSH access to your server (for VPS deployment)

## Option 1: VPS Deployment (Recommended for 24/7 Dry Run)

### Why VPS?
- **Full control**: Root access, can monitor everything
- **Cheap**: $5-12/month
- **Simple**: Just Docker + SSH
- **Persistent**: Data survives across deployments

### 1.1 Choose a Provider

**DigitalOcean** (Easiest, $6/month):
```bash
# Create droplet via UI
- Choose: Ubuntu 24.04 LTS
- Plan: Basic $6/month (1 GB RAM, 1 CPU)
- Region: Closest to you
- Add SSH key
```

**Hetzner** (Cheapest, ~€4/month):
```bash
# Similar process via Hetzner Cloud Console
- Choose: Ubuntu 24.04
- Plan: CX11 (2 GB RAM)
```

**Linode/Akamai** ($5/month):
```bash
# Via Linode Cloud Manager
- Choose: Ubuntu 24.04 LTS
- Plan: Nanode 1GB ($5/month)
```

### 1.2 Initial Server Setup

SSH into your server:
```bash
ssh root@your_server_ip
```

Install Docker:
```bash
# Update packages
apt update && apt upgrade -y

# Install Docker
curl -fsSL https://get.docker.com -o get-docker.sh
sh get-docker.sh

# Install Docker Compose
apt install docker-compose-plugin -y

# Verify installation
docker --version
docker compose version
```

### 1.3 Deploy the Bot

Clone the repo:
```bash
cd ~
git clone <your-repo-url> cryptobot
cd cryptobot
```

**IMPORTANT: Clear Redis First** (to avoid gap issues):
```bash
docker compose up -d redis
docker exec cryptobot-redis redis-cli FLUSHDB
docker compose down
```

Configure environment:
```bash
# Copy example env
cp .env.example .env

# Edit with your values
nano .env
```

Set these values in `.env`:
```bash
REDIS_URL=redis://redis:6379
INITIAL_PORTFOLIO_VALUE=10000.0  # Your starting capital
RUST_LOG=info
# WALLET_PRIVATE_KEY=...  # Add later for live trading
```

Build and start:
```bash
# Build the Docker image (first time only, takes ~5-10 min)
docker compose build

# Start services
docker compose up -d

# Check logs
docker compose logs -f bot
```

### 1.4 Monitor the Bot

View logs in real-time:
```bash
docker compose logs -f bot
```

Check Redis data:
```bash
docker exec cryptobot-redis redis-cli
> KEYS snapshots:*
> ZCARD snapshots:SOL
> ZRANGE snapshots:SOL -10 -1
> exit
```

View resource usage:
```bash
docker stats
```

### 1.5 Update the Bot

```bash
cd ~/cryptobot
git pull
docker compose build
docker compose up -d
```

### 1.6 Backup Redis Data

**Automated backups** (recommended):
```bash
# Add to crontab
crontab -e

# Backup Redis every 6 hours
0 */6 * * * docker exec cryptobot-redis redis-cli SAVE && cp /var/lib/docker/volumes/cryptobot_redis-data/_data/dump.rdb ~/backups/redis-$(date +\%Y\%m\%d-\%H\%M).rdb
```

**Manual backup**:
```bash
docker exec cryptobot-redis redis-cli SAVE
docker cp cryptobot-redis:/data/dump.rdb ./redis-backup-$(date +%Y%m%d).rdb
```

---

## Option 2: Railway/Fly.io (Simpler, $5-10/month)

### Railway

1. **Install Railway CLI**:
```bash
npm install -g @railway/cli
railway login
```

2. **Initialize project**:
```bash
cd cryptobot
railway init
```

3. **Add Redis**:
```bash
railway add redis
```

4. **Configure environment variables**:
```bash
railway variables set RUST_LOG=info
railway variables set INITIAL_PORTFOLIO_VALUE=10000.0
# Add more as needed
```

5. **Deploy**:
```bash
railway up
```

6. **View logs**:
```bash
railway logs
```

**Note**: Railway auto-sleeps after inactivity on free tier. Upgrade to Hobby ($5/month) for 24/7.

### Fly.io

1. **Install flyctl**:
```bash
curl -L https://fly.io/install.sh | sh
flyctl auth login
```

2. **Create fly.toml**:
```toml
app = "cryptobot-[your-name]"

[build]
  dockerfile = "Dockerfile"

[[services]]
  internal_port = 8080
  protocol = "tcp"

[env]
  RUST_LOG = "info"
  INITIAL_PORTFOLIO_VALUE = "10000.0"
```

3. **Create Redis**:
```bash
flyctl redis create
```

4. **Deploy**:
```bash
flyctl deploy
```

5. **View logs**:
```bash
flyctl logs
```

**Cost**: ~$5-10/month for small instance + Redis.

---

## Option 3: AWS EC2 (Most Complex, ~$10/month)

### 3.1 Launch EC2 Instance

1. Go to AWS Console → EC2 → Launch Instance
2. Choose: Ubuntu Server 24.04 LTS
3. Instance type: t2.micro (free tier eligible)
4. Create/select key pair
5. Security group: Allow SSH (port 22) from your IP
6. Launch

### 3.2 Setup (Same as VPS)

Follow steps 1.2-1.6 from VPS deployment above.

---

## Monitoring & Alerts

### View Logs

```bash
# Last 100 lines
docker compose logs --tail=100 bot

# Follow logs
docker compose logs -f bot

# Filter for errors
docker compose logs bot | grep ERROR
```

### Check Portfolio Status

```bash
# View recent logs for portfolio summary
docker compose logs bot | grep "Portfolio Summary" | tail -20
```

### Disk Space

```bash
# Check disk usage
df -h

# Check Docker volumes
docker system df
```

### Set up Alerts (Optional)

Create a simple health check script:
```bash
#!/bin/bash
# health_check.sh

if ! docker ps | grep -q cryptobot; then
    echo "Bot is not running!" | mail -s "CryptoBot Down" your@email.com
    docker compose up -d
fi
```

Add to crontab:
```bash
crontab -e
# Check every 5 minutes
*/5 * * * * /root/health_check.sh
```

---

## Troubleshooting

### Bot Crashes

```bash
# Check logs
docker compose logs --tail=50 bot

# Restart
docker compose restart bot
```

### Gap Detection Errors

If you see "Data gap detected", clear Redis:
```bash
docker exec cryptobot-redis redis-cli FLUSHDB
docker compose restart bot
```

### Out of Memory

```bash
# Check memory
free -h

# If low, add swap
fallocate -l 2G /swapfile
chmod 600 /swapfile
mkswap /swapfile
swapon /swapfile
echo '/swapfile none swap sw 0 0' >> /etc/fstab
```

### Can't Connect to Redis

```bash
# Check Redis is running
docker ps | grep redis

# Restart Redis
docker compose restart redis

# Check Redis health
docker exec cryptobot-redis redis-cli ping
```

---

## Costs Summary

| Option | Monthly Cost | Pros | Cons |
|--------|--------------|------|------|
| **VPS (DigitalOcean)** | $6 | Full control, simple, persistent | Manual setup |
| **VPS (Hetzner)** | €4 (~$4.30) | Cheapest | EU-based |
| **Railway** | $5 | Easiest, auto-deploy | Limited control |
| **Fly.io** | $5-10 | Good CLI, simple | Learning curve |
| **AWS EC2** | $10+ | Scalable, powerful | Most complex |

---

## Recommendation for Dry Run

**VPS (DigitalOcean $6/month)** because:
1. ✅ You're ops-savvy - no problem with SSH/Docker
2. ✅ Full control to monitor everything
3. ✅ Cheap - $6/month for 24/7
4. ✅ Redis data persists across restarts
5. ✅ Easy to scale up later

**Steps**:
1. Create DigitalOcean droplet ($6/month)
2. SSH in, install Docker
3. Clone repo, clear Redis
4. `docker compose up -d`
5. Let it run for 3-7 days to collect uniform data
6. Monitor with `docker compose logs -f bot`

After dry run looks good → Add `WALLET_PRIVATE_KEY` for live trading!

---

## Next Steps After Deployment

1. **Day 1-2**: Monitor logs, verify data collection
2. **Day 3-7**: Review signals, check for gaps
3. **After 7 days**: Analyze performance, tune strategy
4. **When ready**: Add wallet key for live trading with small amounts
