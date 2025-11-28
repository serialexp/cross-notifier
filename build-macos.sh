#!/bin/bash
# ABOUTME: Builds cross-notifier as a macOS .app bundle and optional DMG.
# ABOUTME: Usage: ./build-macos.sh [--dmg]

set -e

APP_NAME="CrossNotifier"
BUNDLE_ID="com.crossnotifier.app"
VERSION="1.0.0"

APP_DIR="${APP_NAME}.app"
CONTENTS_DIR="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"

echo "Building Go binary..."
go build -o cross-notifier .

echo "Creating app bundle structure..."
rm -rf "${APP_DIR}"
mkdir -p "${MACOS_DIR}"
mkdir -p "${RESOURCES_DIR}"

echo "Copying binary..."
cp cross-notifier "${MACOS_DIR}/"

echo "Copying icon..."
if [ -f "Icon.icns" ]; then
    cp Icon.icns "${RESOURCES_DIR}/"
fi

echo "Creating Info.plist..."
cat > "${CONTENTS_DIR}/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>cross-notifier</string>
    <key>CFBundleIconFile</key>
    <string>Icon</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.15</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSUIElement</key>
    <false/>
</dict>
</plist>
EOF

echo "App bundle created: ${APP_DIR}"

# Create DMG if requested
if [ "$1" == "--dmg" ]; then
    DMG_NAME="${APP_NAME}-${VERSION}.dmg"
    DMG_TEMP="dmg-temp"

    echo "Creating DMG..."
    rm -rf "${DMG_TEMP}"
    mkdir -p "${DMG_TEMP}"
    cp -R "${APP_DIR}" "${DMG_TEMP}/"

    # Create symlink to Applications
    ln -s /Applications "${DMG_TEMP}/Applications"

    # Create DMG
    rm -f "${DMG_NAME}"
    hdiutil create -volname "${APP_NAME}" -srcfolder "${DMG_TEMP}" -ov -format UDZO "${DMG_NAME}"

    rm -rf "${DMG_TEMP}"
    echo "DMG created: ${DMG_NAME}"
fi

echo "Done!"
