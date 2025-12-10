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

echo "Generating Icon.icns from logo.png..."
if [ -f "logo.png" ]; then
    rm -rf Icon.iconset
    mkdir -p Icon.iconset
    sips -z 16 16 logo.png --out Icon.iconset/icon_16x16.png > /dev/null
    sips -z 32 32 logo.png --out Icon.iconset/icon_16x16@2x.png > /dev/null
    sips -z 32 32 logo.png --out Icon.iconset/icon_32x32.png > /dev/null
    sips -z 64 64 logo.png --out Icon.iconset/icon_32x32@2x.png > /dev/null
    sips -z 128 128 logo.png --out Icon.iconset/icon_128x128.png > /dev/null
    sips -z 256 256 logo.png --out Icon.iconset/icon_128x128@2x.png > /dev/null
    sips -z 256 256 logo.png --out Icon.iconset/icon_256x256.png > /dev/null
    sips -z 512 512 logo.png --out Icon.iconset/icon_256x256@2x.png > /dev/null
    sips -z 512 512 logo.png --out Icon.iconset/icon_512x512.png > /dev/null
    sips -z 1024 1024 logo.png --out Icon.iconset/icon_512x512@2x.png > /dev/null
    iconutil -c icns Icon.iconset -o Icon.icns
    rm -rf Icon.iconset
fi

echo "Building Go binary..."
MACOSX_DEPLOYMENT_TARGET=11.0 go build -ldflags "-X main.Version=${VERSION}" -o cross-notifier .

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
    <string>11.0</string>
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

    echo "Creating DMG..."
    rm -f "${DMG_NAME}"

    # Use create-dmg for a nice DMG with icon positioning
    if command -v create-dmg &> /dev/null; then
        create-dmg \
            --volname "${APP_NAME}" \
            --volicon "${RESOURCES_DIR}/Icon.icns" \
            --window-pos 200 120 \
            --window-size 600 400 \
            --icon-size 100 \
            --icon "${APP_NAME}.app" 150 185 \
            --hide-extension "${APP_NAME}.app" \
            --app-drop-link 450 185 \
            "${DMG_NAME}" \
            "${APP_DIR}"
    else
        echo "Note: Install create-dmg (brew install create-dmg) for a nicer DMG"
        DMG_TEMP="dmg-temp"
        rm -rf "${DMG_TEMP}"
        mkdir -p "${DMG_TEMP}"
        cp -R "${APP_DIR}" "${DMG_TEMP}/"
        ln -s /Applications "${DMG_TEMP}/Applications"
        hdiutil create -volname "${APP_NAME}" -srcfolder "${DMG_TEMP}" -ov -format UDZO "${DMG_NAME}"
        rm -rf "${DMG_TEMP}"
    fi

    echo "DMG created: ${DMG_NAME}"
fi

echo "Done!"
