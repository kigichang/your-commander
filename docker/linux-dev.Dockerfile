# Linux build/test environment for Your-Commander (Ubuntu 24.04).
#
# Build:  docker build -t yc-linux-dev -f docker/linux-dev.Dockerfile .
# Test:   docker run --rm -v "$PWD":/app -v yc-target:/build/target \
#           -e CARGO_TARGET_DIR=/build/target -w /app/src-tauri \
#           yc-linux-dev cargo test
# Bundle: docker run --rm -v "$PWD":/app -v yc-target:/build/target \
#           -e CARGO_TARGET_DIR=/build/target -w /app \
#           yc-linux-dev bash -c "npm ci && npm run tauri build -- -b deb"
#
# Keep CARGO_TARGET_DIR on a container volume: bind-mounted target/ on an
# external disk is slow and would pollute the macOS build cache.
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    curl \
    ca-certificates \
    git \
    libwebkit2gtk-4.1-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    libssl-dev \
    libdbus-1-dev \
    && rm -rf /var/lib/apt/lists/*

# Node 20 (NodeSource)
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# Rust (stable)
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /app
