# ==========================================
# WirePlant Multi-Stage Container Build
# ==========================================

# Stage 1: Build the release binary
FROM rust:slim-bookworm AS builder

WORKDIR /usr/src/wireplant

# Install compilation prerequisites
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace configuration, sources, and scenario definitions
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY scenarios ./scenarios

# Compile optimized release binary using locked dependencies
RUN cargo build --release --bin wireplant --locked

# Stage 2: Minimal runtime image
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create unprivileged runtime user
RUN useradd -u 10001 -m -s /bin/bash wireplant

WORKDIR /home/wireplant

# Copy compiled binary and bundled scenario definitions
COPY --from=builder /usr/src/wireplant/target/release/wireplant /usr/local/bin/wireplant
COPY scenarios /home/wireplant/scenarios

USER wireplant

# Industrial protocol ports:
# 5020: Modbus TCP
# 4840: OPC-UA
# 8080: HTTP Inspection REST API
EXPOSE 5020 4840 8080

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/wireplant", "--headless"]
