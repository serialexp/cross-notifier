#!/bin/bash
# ABOUTME: Universal installer script for CrossNotifier
# ABOUTME: Downloads and installs the appropriate release for the current platform

set -euo pipefail

REPO="serialexp/cross-notifier"
APP_NAME="cross-notifier"
DISPLAY_NAME="CrossNotifier"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

info() { echo -e "${GREEN}==>${NC} $1"; }
warn() { echo -e "${YELLOW}warning:${NC} $1"; }
error() { echo -e "${RED}error:${NC} $1" >&2; exit 1; }

# Detect OS and architecture
detect_platform() {
    local os arch

    case "$(uname -s)" in
        Linux*)  os="linux" ;;
        Darwin*) os="macos" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *) error "Unsupported operating system: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64) arch="amd64" ;;
        arm64|aarch64) arch="arm64" ;;
        *) error "Unsupported architecture: $(uname -m)" ;;
    esac

    echo "${os}-${arch}"
}

# Get the latest release version from GitHub
get_latest_version() {
    local version
    version=$(curl -sL "https://api.github.com/repos/${REPO}/releases/latest" |
              grep '"tag_name":' |
              sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')

    if [[ -z "$version" ]]; then
        error "Failed to fetch latest version from GitHub"
    fi

    echo "$version"
}

# Install on Linux
install_linux() {
    local version="$1"
    local url="https://github.com/${REPO}/releases/download/${version}/${APP_NAME}-${version}-linux-amd64.tar.gz"
    local tmp_dir
    tmp_dir=$(mktemp -d)

    info "Downloading ${APP_NAME} ${version}..."
    curl -sL "$url" -o "${tmp_dir}/archive.tar.gz"

    info "Extracting..."
    tar -xzf "${tmp_dir}/archive.tar.gz" -C "${tmp_dir}"

    # Find the binary
    local binary_path
    binary_path=$(find "${tmp_dir}" -name "${APP_NAME}" -type f 2>/dev/null | head -1)
    if [[ -z "$binary_path" ]]; then
        error "Could not find ${APP_NAME} binary in archive"
    fi

    # Determine install location
    local bin_dir
    if [[ -w "/usr/local/bin" ]]; then
        bin_dir="/usr/local/bin"
    else
        bin_dir="${HOME}/.local/bin"
        mkdir -p "$bin_dir"
    fi

    info "Installing binary to ${bin_dir}..."
    cp "$binary_path" "${bin_dir}/${APP_NAME}"
    chmod +x "${bin_dir}/${APP_NAME}"

    # Install desktop entry
    local desktop_dir="${HOME}/.local/share/applications"
    local icon_dir="${HOME}/.local/share/icons/hicolor/256x256/apps"
    mkdir -p "$desktop_dir" "$icon_dir"

    info "Installing desktop entry..."
    # Download icon
    curl -sL "https://raw.githubusercontent.com/${REPO}/main/logo.png" -o "${icon_dir}/${APP_NAME}.png" || true

    # Create desktop entry
    cat > "${desktop_dir}/${APP_NAME}.desktop" << EOF
[Desktop Entry]
Name=${DISPLAY_NAME}
Comment=Cross-platform notification daemon
Exec=${bin_dir}/${APP_NAME}
Icon=${APP_NAME}
Terminal=false
Type=Application
Categories=Utility;
Keywords=notifications;daemon;
StartupNotify=false
EOF

    # Update desktop database if available
    if command -v update-desktop-database &> /dev/null; then
        update-desktop-database "$desktop_dir" 2>/dev/null || true
    fi

    # Ask about systemd user service
    echo ""
    read -p "Would you like to enable ${DISPLAY_NAME} to start automatically on login? (y/N) " -n 1 -r
    echo ""
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        setup_systemd_service "$bin_dir"
    fi

    rm -rf "$tmp_dir"

    info "Installation complete!"
    echo ""
    echo "  Binary installed to: ${bin_dir}/${APP_NAME}"
    echo "  Desktop entry: ${desktop_dir}/${APP_NAME}.desktop"
    echo ""
    echo "  Usage:"
    echo "    ${APP_NAME}              # Start daemon (displays notifications)"
    echo "    ${APP_NAME} -server      # Run as server (forwards to clients)"
    echo "    ${APP_NAME} -setup       # Configure settings"
    echo ""
    echo "  You can now launch ${DISPLAY_NAME} from your application menu!"
    echo ""

    # Check if bin_dir is in PATH
    if [[ ":$PATH:" != *":${bin_dir}:"* ]]; then
        warn "${bin_dir} is not in your PATH"
        echo "  Add it with: export PATH=\"\$PATH:${bin_dir}\""
    fi
}

# Setup systemd user service
setup_systemd_service() {
    local bin_dir="$1"
    local service_dir="${HOME}/.config/systemd/user"
    mkdir -p "$service_dir"

    info "Creating systemd user service..."
    cat > "${service_dir}/${APP_NAME}.service" << EOF
[Unit]
Description=CrossNotifier - Desktop notification daemon
After=graphical-session.target

[Service]
Type=simple
ExecStart=${bin_dir}/${APP_NAME}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF

    # Reload systemd and enable service
    info "Enabling systemd service..."
    systemctl --user daemon-reload
    systemctl --user enable ${APP_NAME}.service
    systemctl --user start ${APP_NAME}.service

    if systemctl --user is-active --quiet ${APP_NAME}.service; then
        info "Service started successfully!"
        echo ""
        echo "  Manage the service with:"
        echo "    systemctl --user status ${APP_NAME}   # Check status"
        echo "    systemctl --user stop ${APP_NAME}     # Stop service"
        echo "    systemctl --user restart ${APP_NAME}  # Restart service"
        echo "    systemctl --user disable ${APP_NAME}  # Disable auto-start"
    else
        warn "Service failed to start. Check status with: systemctl --user status ${APP_NAME}"
    fi
}

# Install on macOS
install_macos() {
    local version="$1"
    local arch="$2"
    local url="https://github.com/${REPO}/releases/download/${version}/${DISPLAY_NAME}-${version}-macos-${arch}.dmg"
    local tmp_dir
    tmp_dir=$(mktemp -d)
    local dmg_path="${tmp_dir}/${APP_NAME}.dmg"

    info "Downloading ${APP_NAME} ${version}..."
    curl -sL "$url" -o "$dmg_path"

    info "Mounting DMG..."
    local mount_point
    mount_point=$(hdiutil attach -nobrowse -readonly "$dmg_path" 2>/dev/null | grep "/Volumes" | cut -f3)

    if [[ -z "$mount_point" ]]; then
        error "Failed to mount DMG"
    fi

    info "Installing to /Applications..."
    local app_path="/Applications/${DISPLAY_NAME}.app"

    # Remove existing installation
    if [[ -d "$app_path" ]]; then
        rm -rf "$app_path"
    fi

    cp -R "${mount_point}/${DISPLAY_NAME}.app" "/Applications/"

    info "Unmounting DMG..."
    hdiutil detach "$mount_point" -quiet

    rm -rf "$tmp_dir"

    info "Installation complete!"
    echo ""
    echo "  App installed to: ${app_path}"
    echo "  Launch from Applications or Spotlight"
    echo ""
    echo "  Or run from terminal:"
    echo "    /Applications/${DISPLAY_NAME}.app/Contents/MacOS/${APP_NAME}"
}

# Install on Windows (basic support)
install_windows() {
    local version="$1"
    local url="https://github.com/${REPO}/releases/download/${version}/${APP_NAME}-${version}-windows-amd64.zip"

    echo ""
    echo "Windows installation via this script is not fully supported."
    echo "Please download manually from:"
    echo "  $url"
    echo ""
    echo "Or use PowerShell:"
    echo "  Invoke-WebRequest -Uri '$url' -OutFile '${APP_NAME}.zip'"
    echo "  Expand-Archive -Path '${APP_NAME}.zip' -DestinationPath ."
}

main() {
    echo ""
    echo "  ╔═══════════════════════════════════════╗"
    echo "  ║      CrossNotifier Installer          ║"
    echo "  ╚═══════════════════════════════════════╝"
    echo ""

    local platform
    platform=$(detect_platform)
    info "Detected platform: ${platform}"

    local version
    version=$(get_latest_version)
    info "Latest version: ${version}"

    case "$platform" in
        linux-amd64)
            install_linux "$version"
            ;;
        linux-arm64)
            error "Linux ARM64 builds are not yet available"
            ;;
        macos-amd64)
            install_macos "$version" "amd64"
            ;;
        macos-arm64)
            install_macos "$version" "arm64"
            ;;
        windows-amd64)
            install_windows "$version"
            ;;
        *)
            error "No installation method for platform: ${platform}"
            ;;
    esac
}

main "$@"
