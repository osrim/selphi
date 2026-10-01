# selphi command line

## Commands

```
selphi [--config <FILE>] <command>
```

selphi prepares for postcard paper (100 x 148 mm). With no config file it
uses the values measured on a SELPHY CP1500. `--config` goes before or after
the command; see [Configuration](#configuration).

### `selphi prepare`

Turns photos into print-ready JPEGs.

```sh
selphi prepare                       # src/ -> out/, sources moved to originals/
selphi prepare photo.jpg trip/       # named files and folders
selphi prepare -o prints/ trip/      # write somewhere else than out/
selphi prepare --dry-run             # show the plan; write and move nothing
```

| Option | Meaning |
|---|---|
| `[PATHS]...` | Photos or folders to prepare. Default: `src`. Folders are not searched recursively. |
| `-o, --out <DIR>` | Where the print-ready JPEGs go. Default: `out`. |
| `--archive <DIR>` | Move each finished source here. Default: `originals` when reading `src`; no archiving when paths are named. |
| `--no-archive` | Leave finished sources where they are. |
| `--camera-ref <FILE>` | An unedited camera JPEG. Its Exif is written into every output, for printers that reject edited files. Also read from `$CAMERA_REF`. |
| `--fit <contain\|cover>` | How a photo fills the card: `contain` (the whole photo, stretched up to 2.5%) or `cover` (fills the card, cropped, never stretched). Default: the config's `fit`, else `contain`. Also read from `$SELPHI_FIT`. |
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
   chroma, to `out/<name>-selphi.jpg`. Only the last extension is replaced:
   `photo.v2.png` becomes `photo.v2-selphi.jpg`.
5. Moves the source to the archive folder. An existing file is never
   replaced: `photo.jpg` becomes `photo-2.jpg`, then `photo-3.jpg`.

Each output holds a placement record: a private APP15 segment with the
paper, the fit, the orientation, the canvas size and the distance from each
canvas edge to the picture. `selphi adjust` reads it.

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
→ src/a.jpg  → out/a-selphi.jpg  portrait, stretched 1.9%, edge to edge  (archive: originals/a.jpg)

Dry run: nothing written.
```

### `selphi config`

Shows the values this run uses: the config file, the fit, the output
settings, the canvas, the trim on each edge in both orientations, and how
common photo shapes land on the card. An `Env` line lists the env overrides,
and each overridden value is marked `(from SELPHI_…)`, or
`(portrait from SELPHI_…)` in the trim table.

```
$ selphi config
Config  ~/.config/selphi/printer.toml  (not found: using defaults)
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
| `--fit <contain\|cover>` | Show the photo shapes with this fit. Default: the config's `fit`, else `contain`. Also read from `$SELPHI_FIT`. |

### `selphi calibrate`

Measures the printer's trim. It writes a sheet with nine lines on each
edge, 1.5 to 5.5 mm in, each labelled with its distance. Print it Borderless
and tear the tabs. On each edge, the smallest number whose line still shows
is the trim.

```sh
selphi calibrate                          # write the sheet, then enter the readings
selphi calibrate --sheet-only             # only write calibration-landscape.jpg
selphi calibrate --read                   # enter readings from a sheet printed earlier
selphi calibrate --read --left 2.5 --top 2.0 --right 5.5 --bottom 3.0 --yes
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
| `--font <FILE>` | The TrueType font for the labels. Default: Arial from macOS. Also read from `$SELPHI_FONT`. |
| `--left`, `--top`, `--right`, `--bottom <MM>` | The reading on that edge. Needs `--read`. With any of them, no edge is asked for, and the edges not given keep their trim. Each must be a line on the sheet: 1.5, 2.0, … 5.5. |
| `--yes` | Save without asking. |

Without a terminal, `calibrate` writes the sheet and prints the command to
enter the readings later. To save without one, give the edge flags and
`--yes`.

### `selphi adjust`

Corrects the trims from measurements of a printed photo. This is finer than
`calibrate`, which measures in 0.5 mm steps.

```sh
selphi adjust out/photo-selphi.jpg                              # asks for each edge
selphi adjust out/photo-selphi.jpg --left 1.0 --bottom -0.5 --yes
```

Print a JPEG from `selphi prepare` Borderless, and hold the card in the
orientation it was printed. For each edge, `adjust` asks for a number in mm:

- Positive: white showed between the picture and the card edge.
- Negative: the picture was cut by that amount.
- 0 (the default): the picture reached the edge.

The new trim on each edge is the recorded margin minus the number entered,
so edges with planned white, such as the top and bottom of a 16:9 photo,
work too. `adjust` then shows the trims before and after, and saves them
when you confirm. A trim below 0 or over half the canvas side is an error.

`adjust` refuses a file without a placement record, one made by an older
selphi, and one whose canvas differs from the current canvas: prepare and
print it again. It also refuses a cover print, because its margins do not
show the trim.

| Option | Meaning |
|---|---|
| `--left`, `--top`, `--right`, `--bottom <MM>` | The number for that edge, as above. With any of them, no edge is asked for, and the edges not given keep their trim. |
| `--yes` | Save without asking. |

Without a terminal, give the edge flags and `--yes`.

`calibrate` and `adjust` save the file's values with the new trims, never the
env overrides, and warn when a saved trim is overridden.

### `selphi completions`

Prints the completion script for a shell: `bash`, `elvish`, `fish`,
`powershell` or `zsh`.

```sh
selphi completions zsh > ~/.zfunc/_selphi
```

For zsh, `~/.zfunc` must be in `fpath` before `compinit` runs.

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

The config file is `~/.config/selphi/printer.toml`, or under
`$XDG_CONFIG_HOME` when it is set. `--config <FILE>` or `$SELPHI_CONFIG`
names another file; the flag wins. Without a file, selphi uses the defaults
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
photo so that its top lands on long A and its left on short B; `selphi config`
shows the result for both orientations.

Each profile value has an env var that overrides it for one run, for
example `SELPHI_MAX_STRETCH_PCT=0 selphi prepare`. An override is never
written to the file, and an empty value counts as not set.

| Env var | Overrides |
|---|---|
| `SELPHI_FIT` | `fit` |
| `SELPHI_CANVAS_LONG_MM` | `canvas_long_mm` |
| `SELPHI_CANVAS_SHORT_MM` | `canvas_short_mm` |
| `SELPHI_TRIM_LONG_A_MM` | `trim_long_a_mm` |
| `SELPHI_TRIM_LONG_B_MM` | `trim_long_b_mm` |
| `SELPHI_TRIM_SHORT_A_MM` | `trim_short_a_mm` |
| `SELPHI_TRIM_SHORT_B_MM` | `trim_short_b_mm` |
| `SELPHI_MAX_STRETCH_PCT` | `max_stretch_pct` |

`selphi-gui` uses the same file and overrides, but its own fit from the
toolbar. Its Settings window saves the file's values, never the overrides.

To edit the file by hand, run `selphi config --init`, then
`$EDITOR "$(selphi config --path)"`. `selphi` rewrites the file, so comments
added by hand are not kept.
