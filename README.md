# selphy

Prepares photos for borderless printing on a Canon SELPHY, so that the
printer's edge trim never crops the picture.

In Borderless mode the SELPHY enlarges the image and cuts 2-6 mm off each
edge. `selphy` measures that loss once, then places every photo inside the
part of the page that reaches the card, with white around it. With
`--fit cover`, the photo covers the card instead and the overflow is cut.

## Install

Requires Rust. The version is pinned in `mise.toml`.

```sh
cd ~/repositories/selphy
mise install
cargo install --path crates/selphy-cli   # the `selphy` command line
cargo install --path crates/selphy-gui   # the `selphy-gui` window
```

The command line builds without GPUI, so it installs fast. Neither binary
needs ImageMagick or exiftool.

## Commands

```
selphy [--config <FILE>] <command>
```

selphy prepares for postcard paper (100 x 148 mm). With no config file it
uses the values measured on a SELPHY CP1500. `--config` goes before or after
the command; see [Configuration](#configuration).

### `selphy prepare`

Turns photos into print-ready JPEGs.

```sh
selphy prepare                       # src/ -> out/, sources moved to originals/
selphy prepare photo.jpg trip/       # named files and folders
selphy prepare -o prints/ trip/      # write somewhere else than out/
selphy prepare --dry-run             # show the plan; write and move nothing
```

| Option | Meaning |
|---|---|
| `[PATHS]...` | Photos or folders to prepare. Default: `src`. Folders are not searched recursively. |
| `-o, --out <DIR>` | Where the print-ready JPEGs go. Default: `out`. |
| `--archive <DIR>` | Move each finished source here. Default: `originals` when reading `src`; no archiving when paths are named. |
| `--no-archive` | Leave finished sources where they are. |
| `--camera-ref <FILE>` | An unedited camera JPEG. Its Exif is written into every output, for printers that reject edited files. Also read from `$CAMERA_REF`. |
| `--fit <contain\|cover>` | How a photo fills the card: `contain` (the whole photo, stretched up to 2.5%) or `cover` (fills the card, cropped, never stretched). Default: the config's `fit`, else `contain`. Also read from `$SELPHY_FIT`. |
| `--sharpening <off\|standard\|strong>` | How much each photo is sharpened after resizing. Default: the config's `sharpening`, else `standard`. |
| `--background <white\|black>` | The colour around a `contain` photo. Default: the config's `background`, else `white`. |
| `-j, --jobs <N>` | How many photos to prepare at once. Default: the number of cores, at most 4, because each holds a decoded photo. |
| `--dry-run` | Print each photo's output, placement and archive path. Nothing is written or moved. |

Two sources that would write the same output, such as `a/x.jpg` and
`b/x.jpg`, or `x.jpg` and `x.png`, are an error before any photo is
prepared. Names that differ only in case count as the same output.

For each photo, `prepare`:

1. Reads the photo (JPEG, PNG or TIFF) and applies its Exif rotation.
2. Converts an embedded colour profile to sRGB, and flattens transparency
   onto white.
3. Scales the photo to fit the part of the canvas that survives the trim.
   The side that falls short is stretched by up to 2.5%, because the card is
   not 2:3. A 2:3 photo needs 1.9% and fills the card edge to edge.
   With `--fit cover`, the photo instead covers that part plus 1 mm into the
   trim on each edge, so that a trim that is a little off shows picture, not
   white. The photo keeps its shape and is centred, so both ends of the side
   that overflows lose the same amount.
4. Sharpens, fills the rest of the canvas with the background, and writes a
   300 dpi baseline sRGB JPEG at the canvas size (150x100 mm), with 4:2:0
   chroma, to `out/<name>-selphy.jpg`. Only the last extension is replaced:
   `photo.v2.png` becomes `photo.v2-selphy.jpg`.
5. Moves the source to the archive folder. An existing file is never
   replaced: `photo.jpg` becomes `photo-2.jpg`, then `photo-3.jpg`.

Each output holds a placement record: a private APP15 segment with the
paper, the fit, the orientation, the canvas size and the distance from each
canvas edge to the picture. `selphy adjust` reads it.

The output has no Exif unless `--camera-ref` is given. Hidden files, such as
the `._name.jpg` files macOS writes on external drives, are skipped.

A photo that fails is reported on stderr and left where it was; the other
photos are still prepared. A contain photo reports the white on each edge; a
cover photo reports what was cut off each edge, not counting the 1 mm into
the trim, as in `stretched 2.4%, cut left 13.3, right 13.3 mm`:

```
✓ src/a.jpg  portrait, stretched 1.9%, edge to edge
✓ src/b.jpg  landscape, stretched 2.5%, white top 7.2, bottom 7.2 mm
✗ src/c.jpg  reading src/c.jpg: Format error decoding Jpeg: ...

2 prepared → out, sources → originals
1 failed and left in place
```

The lines are in input order, whatever order the workers finish in. The
progress bar shows only when stderr is a terminal.

`--dry-run` reads only each photo's header, so it is fast on a large folder,
and a photo whose pixels are broken passes it and fails the real run.

```
→ src/a.jpg  → out/a-selphy.jpg  portrait, stretched 1.9%, edge to edge  (archive: originals/a.jpg)

Dry run: nothing written.
```

### `selphy-gui`

A window for `prepare`. Drop photos or folders on the photo list, or use
Add…. Select a photo to see its card as it will print, rendered by the same
code as Prepare, with the part the printer cuts off dimmed around it. The
toolbar holds the fit and the output folder (`~/Pictures/SELPHY` at first).
Prepare runs in parallel, and a failed row's tooltip says why it failed.
Cancel stops the run after the photos in progress. Sources are not moved,
and no camera reference is used.

Remove and Clear can be undone. The window keeps the fit, the output folder
and the theme in `gui.toml` next to the printer config.

Settings… (Cmd-,), or the gear button at the bottom left, opens the Settings
window:

- Printer: the trims, the canvas and the max stretch from the printer
  config. Save Calibration Sheet… writes the sheet for these values.
- Output: the sharpening and the background, also in the printer config, and
  the printer settings that the outputs need.
- Appearance: System, Light or Dark.

Save (Cmd-S) checks the values, writes both files, and closes the window; an
invalid value keeps it open with the error next to the field. Cancel
(Escape), or closing the window, writes nothing. Restore Defaults fills in
the built-in values; nothing is written until Save.

| Key | Command |
|---|---|
| Cmd-O | Add photos |
| Cmd-Shift-O | Choose the output folder |
| Cmd-Return | Prepare |
| Escape | Cancel the run |
| Up, Down | Move the selection |
| Delete, Backspace | Remove the selected photo |
| Cmd-Z, Cmd-Shift-Z | Undo, Redo |
| Cmd-, | Settings |
| Cmd-S, Escape | Save or Cancel in Settings |

### `selphy config`

Shows the values this run uses: the config file, the fit, the output
settings, the canvas, the trim on each edge in both orientations, and how
common photo shapes land on the card. An `Env` line lists the env overrides,
and each overridden value is marked `(from SELPHY_…)`, or
`(portrait from SELPHY_…)` in the trim table.

```
$ selphy config
Config  ~/.config/selphy/printer.toml  (not found: using defaults)
Fit     contain
Output  sRGB, standard sharpening, white background
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
| `--init` | Write the values to the config file, for editing by hand. Env overrides are not written. Fails if the file exists. |
| `--fit <contain\|cover>` | Show the photo shapes with this fit. Default: the config's `fit`, else `contain`. Also read from `$SELPHY_FIT`. |

### `selphy calibrate`

Measures the printer's trim. It writes a sheet with nine lines on each
edge, 1.5 to 5.5 mm in, each labelled with its distance. Print it Borderless
and tear the tabs. On each edge, the smallest number whose line still shows
is the trim.

```sh
selphy calibrate                          # write the sheet, then enter the readings
selphy calibrate --sheet-only             # only write calibration-landscape.jpg
selphy calibrate --read                   # enter readings from a sheet printed earlier
selphy calibrate --read --left 2.5 --top 2.0 --right 5.5 --bottom 3.0 --yes
```

For each edge, `calibrate` asks for the smallest visible number, starting
at the current trim. "no line visible" means the trim is more than 5.5 mm,
which the sheet cannot measure; "skip" keeps the edge's trim. It then shows
the trims before and after, and saves them when you confirm.

| Option | Meaning |
|---|---|
| `--orientation <landscape\|portrait>` | The orientation of the sheet. Either one measures all four trims. Default: `landscape`. Use the same value with `--read`. |
| `--sheet-only` | Only write the sheet. |
| `--read` | Do not write the sheet; ask for the readings. |
| `-o, --out <FILE>` | Where to write the sheet. Default: `calibration-<orientation>.jpg`. |
| `--font <FILE>` | The TrueType font for the labels. Default: Arial from macOS. Also read from `$SELPHY_FONT`. |
| `--left`, `--top`, `--right`, `--bottom <MM>` | The reading on that edge. Needs `--read`. With any of them, no edge is asked for, and the edges not given keep their trim. Each must be a line on the sheet: 1.5, 2.0, … 5.5. |
| `--yes` | Save without asking. |

Without a terminal, `calibrate` writes the sheet and prints the command to
enter the readings later. To save without one, give the edge flags and
`--yes`.

### `selphy adjust`

Corrects the trims from measurements of a printed photo. This is finer than
`calibrate`, which measures in 0.5 mm steps.

```sh
selphy adjust out/photo-selphy.jpg                              # asks for each edge
selphy adjust out/photo-selphy.jpg --left 1.0 --bottom -0.5 --yes
```

Print a JPEG from `selphy prepare` Borderless, and hold the card in the
orientation it was printed. For each edge, `adjust` asks for a number in mm:

- Positive: white showed between the picture and the card edge.
- Negative: the picture was cut by that amount.
- 0 (the default): the picture reached the edge.

The new trim on each edge is the recorded margin minus the number entered,
so edges with planned white, such as the top and bottom of a 16:9 photo,
work too. `adjust` then shows the trims before and after, and saves them
when you confirm. A trim below 0 or over half the canvas side is an error.

`adjust` refuses a file without a placement record, one made by an older
selphy, and one whose canvas differs from the current canvas: prepare and
print it again. It also refuses a cover print, because its margins do not
show the trim.

| Option | Meaning |
|---|---|
| `--left`, `--top`, `--right`, `--bottom <MM>` | The number for that edge, as above. With any of them, no edge is asked for, and the edges not given keep their trim. |
| `--yes` | Save without asking. |

Without a terminal, give the edge flags and `--yes`.

`calibrate` and `adjust` save the file's values with the new trims, never the
env overrides, and warn when a saved trim is overridden.

### `selphy completions`

Prints the completion script for a shell: `bash`, `elvish`, `fish`,
`powershell` or `zsh`.

```sh
selphy completions zsh > ~/.zfunc/_selphy
```

For zsh, `~/.zfunc` must be in `fpath` before `compinit` runs.

## Printer settings

The SELPHY's own Print Settings menu (Setup → Print settings, see the
[CP1500 manual](https://cam.start.canon/en/P001/manual/html/UG-06_Set-up_0020.html))
also changes the print. For selphy's outputs:

| Setting | Use | Why |
|---|---|---|
| Borders | Borderless | The trims are measured in Borderless mode. Bordered prints the whole canvas smaller. |
| Page Layout | 1-up | Other layouts shrink the canvas. |
| Image Optimize | Off | It is on by default and corrects brightness and contrast again. |
| Date, File Number | Off | They print over the picture. |
| Print Finish | Any | Glossy, Semi-gloss or Satin change only the surface. |

The outputs are always sRGB, because the SELPHY does no colour management.
Brightness, Color Adjustment and Filter stay in the printer.

## Output and exit codes

stdout gets results and tables. stderr gets problems and hints: failed
photos, "No images in …", the "Not a terminal" hint, override warnings and
`error: …`.

| Code | Meaning |
|---|---|
| 0 | Done as asked. This includes "No images" and answering No to "Save?". |
| 1 | An error, or at least one photo failed. |
| 2 | A usage error, such as an unknown option. |
| 130 | Cancelled at a prompt with Esc or Ctrl-C. Nothing is saved. |

## Configuration

The config file is `~/.config/selphy/printer.toml`, or under
`$XDG_CONFIG_HOME` when it is set. `--config <FILE>` or `$SELPHY_CONFIG`
names another file; the flag wins. Without a file, selphy uses the defaults
below.

```toml
fit = "contain"          # the default fit: contain or cover
sharpening = "standard"  # off, standard or strong
background = "white"     # around a contain photo: white or black

[postcard]
canvas_long_mm = 150.0   # the canvas sent to the printer, not 4x6 inch
canvas_short_mm = 100.0
trim_long_a_mm = 4.5     # mm lost at each edge, landscape canvas: left
trim_long_b_mm = 5.5     # right
trim_short_a_mm = 2.1    # top
trim_short_b_mm = 2.7    # bottom
max_stretch_pct = 2.5    # largest one-axis stretch
```

Each value resolves as the flag, then the env var, then the file, then the
default. `sharpening` and `background` have no env var. Keys left out of
`[postcard]` take the defaults above.

Unknown keys and tables are an error. So are profile keys at the top level,
as older versions wrote them, and values that leave nothing to print on: a
canvas side of 0 or less, a negative trim or stretch, or two trims that
cover a canvas side.

The trims are named for the landscape canvas. The printer rotates a portrait
photo so that its top lands on long A and its left on short B; `selphy config`
shows the result for both orientations.

Each profile value has an env var that overrides it for one run, for
example `SELPHY_MAX_STRETCH_PCT=0 selphy prepare`. An override is never
written to the file, and an empty value counts as not set.

| Env var | Overrides |
|---|---|
| `SELPHY_FIT` | `fit` |
| `SELPHY_CANVAS_LONG_MM` | `canvas_long_mm` |
| `SELPHY_CANVAS_SHORT_MM` | `canvas_short_mm` |
| `SELPHY_TRIM_LONG_A_MM` | `trim_long_a_mm` |
| `SELPHY_TRIM_LONG_B_MM` | `trim_long_b_mm` |
| `SELPHY_TRIM_SHORT_A_MM` | `trim_short_a_mm` |
| `SELPHY_TRIM_SHORT_B_MM` | `trim_short_b_mm` |
| `SELPHY_MAX_STRETCH_PCT` | `max_stretch_pct` |

`selphy-gui` uses the same file and overrides, but its own fit from the
toolbar. Its Settings window saves the file's values, never the overrides.

To edit the file by hand, run `selphy config --init`, then
`$EDITOR "$(selphy config --path)"`. `selphy` rewrites the file, so comments
added by hand are not kept.

## Development

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
