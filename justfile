# Default recipe - show available commands
default:
    @just --list

# Build the desktop daemon
build:
    go build -o cross-notifier .

# Run tests
test:
    go test ./...

# Build server-only binary (no GUI dependencies)
server:
    CGO_ENABLED=0 go build -o server ./cmd/server

# Build macOS app bundle and DMG
macos:
    ./build-macos.sh --dmg

# Build and push Docker image via Depot
docker:
    depot build --platform linux/amd64,linux/arm64 -t aeolun/cross-notifier-server --push .

# Build Docker image locally
docker-local:
    docker build -t cross-notifier-server .

# Clean build artifacts
clean:
    rm -f cross-notifier server
    rm -rf CrossNotifier.app dmg-temp
