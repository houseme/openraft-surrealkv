# syntax=docker/dockerfile:1
FROM rust:1.80-slim-bookworm AS builder

WORKDIR /build
COPY .. .

# Cache dependencies
RUN cargo fetch

# Build release binary
RUN cargo build --release --bin openraft-surrealkv

# Final minimal image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/openraft-surrealkv /usr/local/bin/
COPY --from=builder /build/docker/entrypoint.sh /entrypoint.sh

RUN chmod +x /entrypoint.sh

# Default data & snapshot directories (bind-mounted in production)
VOLUME ["/data", "/snapshots"]

EXPOSE 50051  # gRPC port
EXPOSE 8080   # optional HTTP client API

ENTRYPOINT ["/entrypoint.sh"]
CMD ["openraft-surrealkv"]