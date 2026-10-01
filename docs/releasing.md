# Releasing

A release is a tag, a GitHub Release with `selphi-darwin-arm64.zip`, and the cask `Casks/selphi.rb` in `osrim/homebrew-tap`.

## Version scheme

selphi is 0.x. A breaking change bumps the minor version. Anything else bumps the patch version. The version is `version` in `[workspace.package]` in `Cargo.toml`. The tag is the version with a `v` prefix. `release.yml` fails when they differ.

## Release steps

On `main`, with a clean tree:

1. Set the new version in `Cargo.toml`.
2. Run `cargo check --workspace` to update `Cargo.lock`.
3. Commit both files as `chore: release vX.Y.Z`.
4. Tag `vX.Y.Z` and push the commit and the tag. The tag push starts `release.yml`.

## What release.yml does

The `build` job runs on `macos-latest` (arm64) and holds no secrets.

1. Installs the Rust version from `mise.toml`.
2. Checks that `Cargo.toml` matches the tag.
3. Runs `cargo test --workspace --locked`.
4. Runs `scripts/bundle-app.sh dist`, which builds `selphi.app`, ad-hoc signs it, and zips it.
5. Checks the version of the CLI and the bundle, and that the completions exist.

The `release` job runs on `ubuntu-latest`. It holds the release token and the tap deploy key, so it runs only the scripts in `scripts/`.

1. Writes `checksums.txt`.
2. Creates the GitHub Release with generated notes. A tag with a `-`, such as `v0.2.0-rc.1`, is a prerelease.
3. Renders the cask with `scripts/brew-cask.sh` and pushes it to `osrim/homebrew-tap`. Prereleases skip this step.

Step 3 runs after the Release, because the asset must be public before the cask points at it.

To run a release again for an existing tag, start `release.yml` by hand with the tag as input. It uploads the assets again with `--clobber`.

## The tap deploy key

`TAP_DEPLOY_KEY` is a repository secret that holds the private half of a deploy key with write access on `osrim/homebrew-tap`.

## The bundle

`selphi.app/Contents/MacOS` holds `selphi-gui`, which is the bundle executable, and `selphi`, the command line. `Contents/Resources/completions` holds the bash, zsh and fish completions. The bundle generates them at build time because Gatekeeper kills the quarantined CLI when `brew` runs it during the install.

The bundle is ad-hoc signed, not notarized. After a cask install, macOS blocks the first launch with "Not Opened". The user must click Open Anyway in System Settings > Privacy & Security once.

## Checking the bundle and the cask locally

`brew style` lints a cask only inside a tap. Render into the tapped clone, lint, then remove the file:

```sh
scripts/bundle-app.sh dist
(cd dist && shasum -a 256 selphi-darwin-arm64.zip > checksums.txt)
tap="$(brew --repo osrim/tap)"
mkdir -p "$tap/Casks"
scripts/brew-cask.sh 0.1.0 dist/checksums.txt > "$tap/Casks/selphi.rb"
brew style osrim/tap/selphi && brew audit --cask --strict osrim/tap/selphi
trash "$tap/Casks/selphi.rb"
```
