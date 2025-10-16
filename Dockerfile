# Multi-stage build for smaller image
FROM rust:1.90 AS builder

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Copy source and migrations
COPY src ./src
COPY migrations ./migrations

# Build release binary
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/cryptobot /app/cryptobot

# Set environment variables
ENV RUST_LOG=info

# Run the bot
CMD ["/app/cryptobot"]
