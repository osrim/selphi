# selphi

> [!WARNING]
> I built selphi using AI, for my own printer. It may not work on
> yours, and I don't offer support.

selphi prepares photos for borderless printing on a Canon SELPHY. In
Borderless mode the printer enlarges the image and cuts 2-6 mm off each
edge. selphi measures that loss once, then fits every photo inside the part
of the page that reaches the card.

It is a side project for trying out [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui), through
[gpui-kit](https://crates.io/crates/gpui-kit).

<img width="1000" height="713" alt="Screenshot 2026-10-01 at 21 42 28" src="https://github.com/user-attachments/assets/0f64ad5e-0a33-4339-b51b-023ccaa1b998" />

## Install

On an Apple Silicon Mac:

```sh
brew install --cask osrim/tap/selphi
```

This installs `selphi.app` in `/Applications` and the `selphi` command line.
The app is not notarized, so macOS blocks the first launch. Open System
Settings > Privacy & Security and click Open Anyway.

To build from source, install the Rust version in `mise.toml`, then run
`cargo install --path crates/selphi-cli` and
`cargo install --path crates/selphi-gui`.

## Use

In the app, add photos, check each card in the preview, and click Prepare.
The prints go to `~/Pictures/SELPHY`. Settings (Cmd-,) holds the printer's
trims and the output options.

The command line prepares photos too, and measures the trims with
`selphi calibrate` and `selphi adjust`. See [docs/cli.md](docs/cli.md).

## Printer settings

On the SELPHY, under Setup > Print settings:

| Setting | Use | Why |
|---|---|---|
| Borders | Borderless | The trims are measured in Borderless mode. Bordered prints the whole canvas smaller. |
| Page Layout | 1-up | Other layouts shrink the canvas. |
| Image Optimize | Off | It is on by default and corrects brightness and contrast again. |
| Date, File Number | Off | They print over the picture. |

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

[docs/releasing.md](docs/releasing.md) covers the app bundle and releases.

## Licence

MIT. See [LICENCE](LICENCE).
