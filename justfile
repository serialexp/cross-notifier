# Default recipe - show available commands
default:
    @just --list

# Set up development environment
setup:
    git config core.hooksPath .githooks
    @echo "Development environment configured!"

# Build the desktop daemon (Rust)
build:
    cargo build --release -p cross-notifier-daemon

# Run the desktop daemon locally (Rust)
dev:
    cargo run -p cross-notifier-daemon

# Run all workspace tests (Rust)
test:
    cargo test --workspace

# Run Rust lints across the workspace
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Format all Rust code
fmt:
    cargo fmt --all

# Build server-only binary (Rust, no GUI dependencies)
server:
    cargo build --release -p cross-notifier-server

# Build macOS app bundle and DMG
macos:
    ./build-macos.sh --dmg

# Build and push Docker image via Depot
docker:
    depot build --platform linux/amd64,linux/arm64 -t aeolun/cross-notifier-server:latest ${IMAGE_TAG:+-t aeolun/cross-notifier-server:$IMAGE_TAG} --provenance=true --sbom=true --push .

# Build Docker image locally (loads into local Docker daemon)
docker-local:
    depot build --platform linux/amd64 -t cross-notifier-server --load .

# Send test notifications locally (default 10, ignores .env)
stress count="10":
    CROSS_NOTIFIER_SERVER=http://localhost:9876 CROSS_NOTIFIER_SECRET= ./test-notify.sh {{count}}

# Send test notifications to remote server (uses .env for CROSS_NOTIFIER_SERVER and CROSS_NOTIFIER_SECRET)
stress-remote count="10":
    ./test-notify.sh {{count}}

# Clean build artifacts
clean:
    rm -rf target
    rm -f cross-notifier
    rm -rf CrossNotifier.app dmg-temp
