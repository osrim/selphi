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
cargo install --path crates/selphy-cli   # the `selphy` command line
cargo install --path crates/selphy-gui   # the `selphy-gui` window
```

Each command puts one binary in `~/.cargo/bin`. The command line builds
without GPUI, so it installs fast. The binaries have no other dependencies:
ImageMagick and exiftool are not needed.

## Commands

```
selphy [--config <FILE>] [--paper postcard|l|card] <command>
```

`--config` names the printer config file for any command. `--paper` names the
paper: `prepare` and `config` use that paper's profile, and `calibrate`
measures it. The default is the config's `paper`, else postcard. Both options
can go before or after the command. See [Configuration](#configuration).

```sh
selphy prepare --paper l trip/       # prepare for L paper
```

Postcard works with no config. L and card must be calibrated once with
`selphy calibrate --paper <paper>`; until then, `prepare` stops with an error
that names that command.

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
4. Sharpens, and writes a 300 dpi baseline JPEG at the paper's canvas size
   (150x100 mm for postcard), with 4:2:0 chroma, to `out/<name>-selphy.jpg`. Only the last extension is replaced:
   `photo.v2.png` becomes `photo.v2-selphy.jpg`.
5. Moves the source to the archive folder. An existing file is never
   replaced: `photo.jpg` becomes `photo-2.jpg`, then `photo-3.jpg`.

Each output holds a placement record: a private APP15 segment with the
paper, the fit, the orientation, the canvas size and the distance from each
canvas edge to the picture. `selphy adjust` reads it.

The output has no Exif unless `--camera-ref` is given. Hidden files, such as
the `._name.jpg` files macOS writes on external drives, are skipped.

A photo that fails is reported on stderr and left where it was; the other
photos are still prepared. The output reports each photo, prepared ones on
stdout and failed ones on stderr:

```
✓ src/a.jpg  portrait, stretched 1.9%, edge to edge
✓ src/b.jpg  landscape, stretched 2.5%, white top 7.2, bottom 7.2 mm
✗ src/c.jpg  reading src/c.jpg: Format error decoding Jpeg: ...

2 prepared → out, sources → originals
1 failed and left in place
```

The exit code is 1 when any photo failed. When no images are found, it says
"No images in …" on stderr and exits 0. The progress bar is shown only when
stderr is a terminal.

### `selphy-gui`

A window for `prepare`. Drop photos or folders on it, or use Add…. Choose the
folder the prints go to (on macOS it starts at `~/Pictures`), then Prepare.
Each photo shows its result or its error. Cancel stops the run after the
photo in progress. Sources are not moved, and no camera reference is used. It
prepares for the paper that `selphy prepare` uses without `--paper`.

The gear button (Cmd-,) opens the Config dialog. It has two sections:

- Appearance: the theme, System, Light or Dark. It is saved to `gui.toml`
  next to the printer config. The command line does not read this file.
- Printer: the postcard profile's four trims, canvas and largest stretch.
  They are saved to the `[postcard]` table of the config file that the
  command line reads. The other papers' tables are kept.

Restore defaults puts the default values in every field. Nothing is written
until Save.

| Key | Command |
|---|---|
| Cmd-O | Add photos |
| Cmd-Shift-O | Choose the output folder |
| Enter | Prepare |
| Escape | Cancel the run |
| Cmd-, | Config |

### `selphy config`

Shows the config file, the paper and the papers that are calibrated, and for
the paper: the canvas, the trim on each edge in both orientations, and how
common photo shapes land on the card. `selphy config --paper l` shows the L
profile; an uncalibrated paper is an error. The values are the ones this run
uses:
when env vars override values, an `Env` line under the header lists them, and
each overridden value is marked with `(from SELPHY_…)`. In the trim
table, the mark also names the column, as in `(portrait from SELPHY_…)`.

```
$ selphy config
Config  ~/.config/selphy/printer.toml  (not found: using defaults)
Paper   postcard (calibrated: postcard)
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
| `--init` | Write the paper's values to the config file, for editing by hand. Env overrides are not written. Fails if the file exists. |

### `selphy calibrate`

Measures the printer's trim on one paper. It writes a bracket sheet at the
paper's canvas: on each edge, nine lines at 1.5 to 5.5 mm from the edge, each
labelled with its distance. The sheet names the paper. Print the sheet
Borderless and tear the tabs. On each edge, the smallest number whose line
still shows is the trim on that edge.

```sh
selphy calibrate                          # write the sheet, then enter the readings
selphy calibrate --sheet-only             # only write calibration-landscape.jpg
selphy calibrate --read                   # enter readings from a sheet printed earlier
selphy calibrate --paper card             # measure card paper
```

The readings are saved to the paper's table. The other papers' tables stay
as they are. A paper that has no table starts from a canvas the size of the
paper, with no trims.

For each edge, `calibrate` asks for the smallest visible number. The cursor
starts at the current trim. "no line visible" means the trim is more than
5.5 mm, which the sheet cannot measure; "skip" keeps the edge's trim. It then
shows the trims before and after, and saves them to the config file when you
confirm.

| Option | Meaning |
|---|---|
| `--orientation <landscape\|portrait>` | The orientation of the sheet. Either one measures all four trims. Default: `landscape`. Use the same value, and the same `--paper`, with `--read`. |
| `--sheet-only` | Only write the sheet. |
| `--read` | Do not write the sheet; ask for the readings. |
| `-o, --out <FILE>` | Where to write the sheet. Default: `calibration-<orientation>.jpg`. |
| `--font <FILE>` | The TrueType font for the labels. Default: Arial from macOS. Also read from `$SELPHY_FONT`. |

The readings need a terminal. Without one, `calibrate` writes the sheet and
tells you, on stderr, the command to enter the readings later.

### `selphy adjust`

Corrects the trims from measurements of a printed photo. This is finer than
`calibrate`, which measures in 0.5 mm steps.

```sh
selphy adjust out/photo-selphy.jpg
```

Print a JPEG from `selphy prepare` Borderless, and hold the card in the
orientation it was printed. For each edge, `adjust` asks for a number in mm:

- Positive: white showed between the picture and the card edge.
- Negative: the picture was cut by that amount.
- 0 (the default): the picture reached the edge.

The new trim on each edge is the recorded margin minus the number entered, so
edges with planned white, such as the top and bottom of a 16:9 photo, give
correct results too. `adjust` then shows the trims before and after, and saves
them to the profile of the paper the file was prepared for when you confirm.
`--paper` and `$SELPHY_PAPER` are ignored, with a warning on stderr when they
name another paper.
It stops with an error when a trim would be below 0 or more than half the
canvas side.

The file must hold a placement record, which only `selphy prepare` writes.
`adjust` refuses a file made by an older selphy, and a file whose canvas
differs from the paper's current canvas: prepare and print it again. The
prompts need a terminal.

`calibrate` and `adjust` save the file's values with the new trims, never the
env overrides. For each trim they save while an env var overrides it, they
warn on stderr that the saved value is not used while the var is set.

## Output and exit codes

stdout gets results and tables: the prepared photos, the summary, the
`config` output, the change tables, "Saved.", "Not saved." and "Wrote …".
stderr gets problems and hints: failed photos, "No images in …", the "Not a
terminal" hint, override warnings and `error: …`.

| Code | Meaning |
|---|---|
| 0 | Done as asked. This includes "No images" and answering No to "Save?". |
| 1 | An error, or at least one photo failed. |
| 2 | A usage error, such as an unknown option. |
| 130 | Cancelled at a prompt with Esc or Ctrl-C. Nothing is saved. |

## Configuration

Each value is resolved in this order: command line, then env, then the config
file, then the defaults. On the command line, `--config` chooses the file and
`--paper` the paper; env vars set single values.

The config file is `~/.config/selphy/printer.toml`. `$XDG_CONFIG_HOME` moves
it. `--config <FILE>` names the file directly, and so does `$SELPHY_CONFIG`;
the flag wins over the env var.

The file holds the default paper and one table per calibrated paper:
`[postcard]`, `[l]` and `[card]`. Each table is a profile: the canvas, the
four trims and the max stretch. Without a file, postcard uses the defaults
below, and L and card are not calibrated.

```toml
paper = "postcard"       # the default paper: postcard, l or card

[postcard]
canvas_long_mm = 150.0   # the canvas sent to the printer, not 4x6 inch
canvas_short_mm = 100.0
trim_long_a_mm = 4.5     # mm lost at each edge, landscape canvas: left
trim_long_b_mm = 5.5     # right
trim_short_a_mm = 2.1    # top
trim_short_b_mm = 2.7    # bottom
max_stretch_pct = 2.5    # largest one-axis stretch
```

The paper is resolved as `--paper`, then `$SELPHY_PAPER`, then `paper` in the
file, then postcard. Keys left out of a `[postcard]` table take the defaults
above. An `[l]` or `[card]` table must have all four trims, because those
papers have no built-in trims; a canvas or stretch left out takes the size of
the paper (L 119 x 89 mm, card 86 x 54 mm) or 2.5 %. Unknown keys are an
error. So is a file with the profile keys at the top
level, as older versions wrote it: move them into a `[postcard]` table. So
are values that leave nothing to print on: a canvas side of 0 or less, a
negative trim or stretch, or two trims that together cover a canvas side.
The error names the table, as in `[l] trim_long_a_mm = -1: must be 0 or
more`.

The trims are named for the landscape canvas. The printer rotates a portrait
photo so that its top lands on long A and its left on short B; `selphy config`
shows the result for both orientations.

Each profile value has an env var that overrides it for one run, in the
profile of the paper this run uses, for example
`SELPHY_MAX_STRETCH_PCT=0 selphy prepare --paper l`. An override is never
written to the file. An empty value counts as not set. A value that is not a number is
an error that names the var, and so is an override that leaves nothing to
print on.

| Env var | Overrides |
|---|---|
| `SELPHY_PAPER` | `paper` |
| `SELPHY_CANVAS_LONG_MM` | `canvas_long_mm` |
| `SELPHY_CANVAS_SHORT_MM` | `canvas_short_mm` |
| `SELPHY_TRIM_LONG_A_MM` | `trim_long_a_mm` |
| `SELPHY_TRIM_LONG_B_MM` | `trim_long_b_mm` |
| `SELPHY_TRIM_SHORT_A_MM` | `trim_short_a_mm` |
| `SELPHY_TRIM_SHORT_B_MM` | `trim_short_b_mm` |
| `SELPHY_MAX_STRETCH_PCT` | `max_stretch_pct` |

`selphy-gui` uses the same file (`$SELPHY_CONFIG` if set), the same paper
(`$SELPHY_PAPER`, else the file's `paper`) and the same overrides when it
prepares photos. Its Config dialog shows and saves the file's postcard
values, without the overrides, and keeps the other tables.

To edit the values: run `selphy config --init`, then open the file with
`$EDITOR "$(selphy config --path)"`. `selphy` rewrites the file, so comments
added by hand are not kept.

## Development

The repo is a Cargo workspace with three crates:

- `crates/selphy`: the library. It does all the work.
- `crates/selphy-cli`: the `selphy` command line.
- `crates/selphy-gui`: the `selphy-gui` window.

```sh
cargo test                               # the library and the command line
cargo test --workspace                   # also the window (builds GPUI)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

The library has one module per concept in [`CONTEXT.md`](CONTEXT.md). The
module layout is in
[`docs/specs/05-workspace-and-module-layout.md`](docs/specs/05-workspace-and-module-layout.md).
