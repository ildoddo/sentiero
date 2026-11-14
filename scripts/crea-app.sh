#!/bin/sh
# Compila sentiero-tutor con la finestra nativa e lo assembla in target/release/Sentiero.app (solo macOS).
set -eu
cd "$(dirname "$0")/.."

cargo build --release --features finestra --bin sentiero-tutor
versione=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

app=target/release/Sentiero.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/sentiero-tutor "$app/Contents/MacOS/"
cp assets/Sentiero.icns "$app/Contents/Resources/"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Sentiero</string>
  <key>CFBundleDisplayName</key><string>Sentiero</string>
  <key>CFBundleIdentifier</key><string>io.github.ildoddo.sentiero</string>
  <key>CFBundleExecutable</key><string>sentiero-tutor</string>
  <key>CFBundleIconFile</key><string>Sentiero</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$versione</string>
  <key>CFBundleVersion</key><string>$versione</string>
  <key>CFBundleDevelopmentRegion</key><string>it</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <!-- L'interfaccia è servita in http dal programma stesso, su 127.0.0.1. -->
  <key>NSAppTransportSecurity</key>
  <dict><key>NSAllowsLocalNetworking</key><true/></dict>
</dict>
</plist>
PLIST
# Firma locale (ad hoc): copre anche Info.plist e icona, altrimenti macOS considera il pacchetto alterato.
codesign --force --sign - "$app"
echo "Creata $app"
