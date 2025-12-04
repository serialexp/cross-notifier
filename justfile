# Default recipe - show available commands
default:
    @just --list

# Set up development environment
setup:
    git config core.hooksPath .githooks
    go install github.com/golangci/golangci-lint/cmd/golangci-lint
    @echo "Development environment configured!"

# Build the desktop daemon
build:
    go build -o cross-notifier .

# Run tests
test:
    go test ./...

# Run linters (same as CI)
lint:
    @echo "Checking formatting..."
    @gofmt -s -l . | grep -v '^vendor/' | (! grep .) || (echo "Run 'just fmt' to fix" && exit 1)
    @echo "Running go vet..."
    go vet ./...
    @echo "Running golangci-lint..."
    golangci-lint run --timeout=5m

# Format code
fmt:
    gofmt -s -w .

# Build server-only binary (no GUI dependencies)
server:
    CGO_ENABLED=0 go build -o server ./cmd/server

# Build macOS app bundle and DMG
macos:
    ./build-macos.sh --dmg

# Build and push Docker image via Depot
docker:
    depot build --platform linux/amd64,linux/arm64 -t aeolun/cross-notifier-server --provenance=true --sbom=true --push .

# Build Docker image locally
docker-local:
    docker build -t cross-notifier-server .

# Send test notifications in parallel (default 10)
stress count="10":
    ./test-notify.sh "Stress Test" "Message" "" {{count}}

# Clean build artifacts
clean:
    rm -f cross-notifier server
    rm -rf CrossNotifier.app dmg-temp
