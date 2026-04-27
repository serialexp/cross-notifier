# Dockerfile for the headless notification server.
# Builds the cross-notifier-server Rust binary on a distroless-ish runtime.

FROM rust:1.94-slim AS builder

WORKDIR /build

# Workspace manifests first for better layer caching. Copy every crate's
# Cargo.toml so `cargo fetch` sees the full dependency graph.
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml core/Cargo.toml
COPY server/Cargo.toml server/Cargo.toml
COPY daemon/Cargo.toml daemon/Cargo.toml
COPY calendar/Cargo.toml calendar/Cargo.toml

# Create empty src trees so cargo can resolve targets without the real code.
RUN mkdir -p core/src server/src daemon/src calendar/src \
    && echo "fn main() {}" > server/src/main.rs \
    && echo "" > core/src/lib.rs \
    && echo "fn main() {}" > daemon/src/main.rs \
    && echo "" > calendar/src/lib.rs \
    && mkdir -p calendar/examples \
    && echo "fn main() {}" > calendar/examples/fetch_caldav.rs \
    && echo "fn main() {}" > calendar/examples/dump_caldav.rs \
    && cargo fetch --locked

# Real sources.
COPY core ./core
COPY server ./server
COPY calendar ./calendar

# Build only the server crate — skip the daemon (needs graphics libs).
RUN cargo build --release -p cross-notifier-server --locked

# Runtime
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 1000 -m notifier

WORKDIR /app
COPY --from=builder /build/target/release/cross-notifier-server /app/cross-notifier-server

USER notifier

EXPOSE 9876
ENV CROSS_NOTIFIER_PORT=9876

ENTRYPOINT ["/app/cross-notifier-server"]
