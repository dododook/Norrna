# syntax=docker/dockerfile:1
FROM rust:1-bookworm AS builder
WORKDIR /src
COPY Cargo.toml ./
COPY crates ./crates
RUN cargo build --release --workspace \
    && strip target/release/norrna target/release/norrna-manager

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /opt/norrna
COPY --from=builder /src/target/release/norrna-manager /opt/norrna/norrna-manager
COPY --from=builder /src/target/release/norrna /opt/norrna/norrna
RUN mkdir -p /etc/norrna-manager /data \
    && cp /opt/norrna/norrna /etc/norrna-manager/norrna \
    && chmod +x /opt/norrna/norrna-manager /opt/norrna/norrna /etc/norrna-manager/norrna

ENV WEBPORT=3000
ENV AGENTPORT=3001
ENV DATA_DIR=/data

EXPOSE 3000 3001
VOLUME ["/data"]

ENTRYPOINT ["/opt/norrna/norrna-manager"]
