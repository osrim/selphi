# selphy

Prepares photos for borderless printing on a Canon SELPHY, so that the
printer's edge trim never crops the picture.

In Borderless mode the SELPHY enlarges the image and cuts 2–6 mm off each
edge. `selphy` measures that loss once, then places every photo inside the
part of the page that reaches the card. Nothing of the picture is lost; the
leftover space is white.

## Install

Requires Rust. The version is pinned in `mise.toml`.

```sh
cd ~/repositories/selphy
mise install
cargo install --path .
```

This puts `selphy` in `~/.cargo/bin`. The binary has no other dependencies:
ImageMagick and exiftool are not needed.

## Commands

### `selphy prepare`

Turns photos into print-ready JPEGs.

```sh
selphy prepare                       # src/ -> out/, sources moved to originals/
selphy prepare photo.jpg trip/       # named files and folders
selphy prepare -o prints/ trip/      # write somewhere else than out/
```

| Option | Meaning |
|---|---|
| `[PATHS]...` | Photos or folders to prepare. Default: `src`. Folders are not searched recursively. |
| `-o, --out <DIR>` | Where the print-ready JPEGs go. Default: `out`. |
| `--archive <DIR>` | Move each finished source here. Default: `originals` when reading `src`; no archiving when paths are named. |
| `--no-archive` | Leave finished sources where they are. |
| `--camera-ref <FILE>` | An unedited camera JPEG. Its Exif is written into every output, for printers that reject edited files. Also read from `$CAMERA_REF`. |

Paths are relative to the current directory.

For each photo, `prepare`:

1. Reads the photo (JPEG, PNG or TIFF) and applies its Exif rotation.
2. Converts an embedded colour profile to sRGB, and flattens transparency
   onto white.
3. Scales the photo to fit the part of the canvas that survives the trim.
   The side that falls short is stretched by up to 2.5%, because the card is
   not 2:3. A 2:3 photo needs 1.9% and fills the card edge to edge.
4. Sharpens, and writes a 150x100 mm, 300 dpi, baseline JPEG with 4:2:0
   chroma to `out/<name>.jpg`. Only the last extension is replaced:
   `photo.v2.png` becomes `photo.v2.jpg`.
5. Moves the source to the archive folder. An existing file is never
   replaced: `photo.jpg` becomes `photo-2.jpg`, then `photo-3.jpg`.

Each output holds a placement record: a private APP15 segment with the
orientation and the distance from each canvas edge to the picture.
`selphy adjust` reads it.

The output has no Exif unless `--camera-ref` is given. Hidden files, such as
the `._name.jpg` files macOS writes on external drives, are skipped.

A photo that fails is reported and left where it was; the other photos are
still prepared. The output reports each photo:

```
✓ src/a.jpg  portrait, stretched 1.9%, edge to edge
✓ src/b.jpg  landscape, stretched 2.5%, white top 7.2, bottom 7.2 mm
✗ src/c.jpg  reading src/c.jpg: Format error decoding Jpeg: ...

2 prepared → out, sources → originals
1 failed and left in place
```

The exit code is 1 when any photo failed.

### `selphy config`

Shows the config file, the trim on each edge in both orientations, and how
common photo shapes land on the card.

```
$ selphy config
Config  ~/.config/selphy/printer.toml  (not found: using defaults)
Canvas  150 x 100 mm, stretch up to 2.5%

Trim, mm    landscape  portrait
  left            4.5       2.7
  top             2.1       4.5
  right           5.5       2.1
  bottom          2.7       5.5

How a photo lands
  3:2   stretched 1.9%, edge to edge
  4:3   stretched 2.5%, white left 5.0, right 5.0 mm
  16:9  stretched 2.5%, white top 7.2, bottom 7.2 mm
  1:1   stretched 2.5%, white left 21.3, right 21.3 mm
```

| Option | Meaning |
|---|---|
| `--path` | Print only the config file's path. |
| `--init` | Write the current values to the config file, for editing by hand. Fails if the file exists. |

### `selphy calibrate`

Measures the printer's trim. It writes a bracket sheet: on each edge, nine
lines at 1.5 to 5.5 mm from the edge, each labelled with its distance. Print
the sheet Borderless and tear the tabs. On each edge, the smallest number
whose line still shows is the trim on that edge.

```sh
selphy calibrate                          # write the sheet, then enter the readings
selphy calibrate --sheet-only             # only write calibration-landscape.jpg
selphy calibrate --read                   # enter readings from a sheet printed earlier
```

For each edge, `calibrate` asks for the smallest visible number. The cursor
starts at the current trim. "no line visible" means the trim is more than
5.5 mm, which the sheet cannot measure; "skip" keeps the edge's trim. It then
shows the trims before and after, and saves them to the config file when you
confirm.

| Option | Meaning |
|---|---|
| `--orientation <landscape\|portrait>` | The orientation of the sheet. Either one measures all four trims. Default: `landscape`. Use the same value with `--read`. |
| `--sheet-only` | Only write the sheet. |
| `--read` | Do not write the sheet; ask for the readings. |
| `-o, --out <FILE>` | Where to write the sheet. Default: `calibration-<orientation>.jpg`. |
| `--font <FILE>` | The TrueType font for the labels. Default: Arial from macOS. Also read from `$SELPHY_FONT`. |

The readings need a terminal. Without one, `calibrate` writes the sheet and
tells you the command to enter the readings later.

### `selphy adjust`

Corrects the trims from measurements of a printed photo. This is finer than
`calibrate`, which measures in 0.5 mm steps.

```sh
selphy adjust out/photo.jpg
```

Print a JPEG from `selphy prepare` Borderless, and hold the card in the
orientation it was printed. For each edge, `adjust` asks for a number in mm:

- Positive: white showed between the picture and the card edge.
- Negative: the picture was cut by that amount.
- 0 (the default): the picture reached the edge.

The new trim on each edge is the recorded margin minus the number entered, so
edges with planned white, such as the top and bottom of a 16:9 photo, give
correct results too. `adjust` then shows the trims before and after, and saves
them to the config file when you confirm. It stops with an error when a trim
would be below 0 or more than half the canvas side.

The file must hold a placement record. JPEGs from the old bash scripts do not;
prepare the photo again. The prompts need a terminal.

## Configuration

The config file is `~/.config/selphy/printer.toml`. `$XDG_CONFIG_HOME` moves
it, and `$SELPHY_CONFIG` names the file directly. Without a file, the
defaults below apply. Keys left out of the file take their default; unknown
keys are an error.

```toml
canvas_long_mm = 150.0   # the canvas sent to the printer, not 4x6 inch
canvas_short_mm = 100.0
trim_long_a_mm = 4.5     # mm lost at each edge, landscape canvas: left
trim_long_b_mm = 5.5     # right
trim_short_a_mm = 2.1    # top
trim_short_b_mm = 2.7    # bottom
max_stretch_pct = 2.5    # largest one-axis stretch
```

The trims are named for the landscape canvas. The printer rotates a portrait
photo so that its top lands on long A and its left on short B; `selphy config`
shows the result for both orientations.

To edit the values: run `selphy config --init`, then open the file with
`$EDITOR "$(selphy config --path)"`. `selphy` rewrites the file, so comments
added by hand are not kept.

## Development

```sh
cargo test                               # unit and binary tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

| Module | Contents |
|---|---|
| `src/config.rs` | `Config`, loading and saving the TOML file |
| `src/geometry/canvas.rs` | Units, orientation, edges, which trim lands on which edge |
| `src/geometry/placement.rs` | `place()`: fit and capped stretch inside the safe box |
| `src/imaging.rs` | Decoding, sRGB conversion, resize and sharpen, JPEG encoding |
| `src/calibrate.rs` | The bracket sheet, and readings to trims |
| `src/record.rs` | The placement record: writing, finding and parsing it |
| `src/adjust.rs` | Measurements from a print to corrected trims |
| `src/prepare.rs` | Collecting inputs, preparing one photo, archiving, the batch |
| `src/main.rs`, `src/cli/` | The command line: one module per command |
