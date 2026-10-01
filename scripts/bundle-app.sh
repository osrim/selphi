#!/usr/bin/env bash
# Builds selphi.app, ad-hoc signed, and zips it for the Homebrew cask.
# The bundle runs `selphi-gui` and carries the `selphi` command line and its
# shell completions, which the cask links into the Homebrew prefix.
#
# Usage: scripts/bundle-app.sh <out-dir>
# Writes <out-dir>/selphi.app and <out-dir>/selphi-darwin-arm64.zip.
set -euo pipefail

out=${1:?usage: scripts/bundle-app.sh <out-dir>}
mkdir -p "$out"
out=$(cd "$out" && pwd)
app="$out/selphi.app"
zip="$out/selphi-darwin-arm64.zip"

if [ "$(uname -s)-$(uname -m)" != "Darwin-arm64" ]; then
  echo "bundle-app.sh builds on Apple Silicon macOS only" >&2
  exit 1
fi
for path in "$app" "$zip"; do
  if [ -e "$path" ]; then
    echo "$path exists; remove it first" >&2
    exit 1
  fi
done

# The plist's minimum must match the one the binaries are linked for.
export MACOSX_DEPLOYMENT_TARGET=11.0

cd "$(dirname "$0")/.."
# `path+file:///…/crates/selphi-gui#0.1.0`
id=$(cargo pkgid -p selphi-gui)
version=${id##*[#@]}

cargo build --release --locked -p selphi-cli -p selphi-gui

mkdir -p "$app/Contents/MacOS"
cp target/release/selphi-gui target/release/selphi "$app/Contents/MacOS/"
# Generated here: Gatekeeper kills the quarantined CLI when brew runs it.
completions="$app/Contents/Resources/completions"
mkdir -p "$completions"
target/release/selphi completions bash > "$completions/selphi.bash"
target/release/selphi completions zsh > "$completions/_selphi"
target/release/selphi completions fish > "$completions/selphi.fish"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>selphi</string>
  <key>CFBundleExecutable</key>
  <string>selphi-gui</string>
  <key>CFBundleIdentifier</key>
  <string>io.github.osrim.selphi</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>selphi</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$version</string>
  <key>CFBundleVersion</key>
  <string>$version</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.photography</string>
  <key>LSMinimumSystemVersion</key>
  <string>$MACOSX_DEPLOYMENT_TARGET</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist"

# Nested code first: signing the bundle signs only its main executable.
codesign --force --sign - "$app/Contents/MacOS/selphi"
codesign --force --sign - "$app"
codesign --verify --strict --deep "$app"

ditto -c -k --keepParent "$app" "$zip"
echo "$zip"
