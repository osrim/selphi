#!/usr/bin/env bash
# Prints the Homebrew cask for a release to stdout.
#
# Usage: scripts/brew-cask.sh <version> <checksums.txt>
set -euo pipefail

version=${1:?usage: scripts/brew-cask.sh <version> <checksums.txt>}
checksums=${2:?usage: scripts/brew-cask.sh <version> <checksums.txt>}
asset=selphi-darwin-arm64.zip

sha=$(awk -v name="$asset" '$2 == name { print $1 }' "$checksums")
if ! [[ $sha =~ ^[0-9a-f]{64}$ ]]; then
  echo "no sha256 for $asset in $checksums" >&2
  exit 1
fi

cat <<EOF
cask "selphi" do
  version "$version"
  sha256 "$sha"

  url "https://github.com/osrim/selphi/releases/download/v#{version}/$asset"
  name "selphi"
  desc "Prepare photos for borderless printing on a Canon SELPHY"
  homepage "https://github.com/osrim/selphi"

  depends_on arch: :arm64
  depends_on :macos

  app "selphi.app"
  binary "#{appdir}/selphi.app/Contents/MacOS/selphi"
  bash_completion "#{appdir}/selphi.app/Contents/Resources/completions/selphi.bash"
  zsh_completion "#{appdir}/selphi.app/Contents/Resources/completions/_selphi"
  fish_completion "#{appdir}/selphi.app/Contents/Resources/completions/selphi.fish"

  zap trash: [
    "~/.config/selphi",
    "~/Library/Saved Application State/io.github.osrim.selphi.savedState",
  ]

  caveats <<~EOS
    selphi.app is not notarized. macOS blocks the first launch.
    To allow it, open System Settings > Privacy & Security and click Open Anyway.
  EOS
end
EOF
