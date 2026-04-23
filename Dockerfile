# ── Stage 1: Builder ─────────────────────────────────────────────────────
FROM rust:1.82-bookworm AS builder

RUN apt-get update && apt-get install -y \
    clang llvm libclang-dev cmake pkg-config librocksdb-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
COPY wallet/ wallet/
COPY prototypes/ prototypes/
COPY tests/ tests/

RUN cargo build --release -p evaporchain-node -p evaporchain-cli

# ── Stage 2: Runtime ─────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates librocksdb7.8 curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash evaporchain

COPY --from=builder /build/target/release/evaporchain-node /usr/local/bin/
COPY --from=builder /build/target/release/evaporchain-cli /usr/local/bin/
COPY genesis-mainnet.json /etc/evaporchain/genesis.json

RUN mkdir -p /data/evaporchain && chown evaporchain:evaporchain /data/evaporchain

USER evaporchain
WORKDIR /home/evaporchain

ENV DATA_DIR=/data/evaporchain
ENV RUST_LOG=info

EXPOSE 8080 9090 26656

HEALTHCHECK --interval=10s --timeout=3s --retries=3 \
    CMD curl -sf http://localhost:8080/api/status || exit 1

ENTRYPOINT ["evaporchain-node"]
CMD ["--api", "--api-port", "8080", "--network", "--tendermint", \
     "--data-dir", "/data/evaporchain", \
     "--genesis-config", "/etc/evaporchain/genesis.json"]
