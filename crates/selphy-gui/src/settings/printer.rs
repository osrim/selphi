//! The Printer pane: one paper's profile, its four trims by edge of a
//! landscape print, its canvas and max stretch. Save writes the paper's
//! table on top of the loaded file, so that the other tables and the
//! top-level keys are kept. Field errors name the edge or field that failed.

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    form::{Field, Form},
    h_flex,
    input::{InputState, NumberInput},
    menu::{DropdownMenu as _, PopupMenuItem},
    setting::{SettingGroup, SettingItem, SettingPage},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use selphy::calibrate;
use selphy::config::fields::{CANVAS_LONG, CANVAS_SHORT, Field as ConfigField, MAX_STRETCH};
use selphy::config::{Config, Invalid, Loaded, Profile, Side};
use selphy::geometry::{Edge, Fit, Orientation, Trim};
use selphy::paper::Paper;
use selphy::report::sentence;

use crate::state::Prefs;
use crate::text::{display_path, error_sentence, paper_label};

/// The field that holds the trim on `edge` of a landscape print.
pub fn trim_field(edge: Edge) -> &'static ConfigField {
    ConfigField::for_trim(Trim::at(Orientation::Landscape, edge))
}

/// A value the pane refused, the fields it is about, and why, in the pane's
/// words.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldError {
    pub fields: Vec<&'static ConfigField>,
    pub message: String,
}

/// `start` with `values` typed into its fields, checked with
/// `Profile::with_trims`. A value that is not a number, or a profile that
/// cannot print, is an error on the fields it is about.
pub fn read_profile(
    start: &Profile,
    values: &[(&'static ConfigField, &str)],
) -> Result<Profile, FieldError> {
    let mut unchecked = start.clone();
    for &(field, text) in values {
        *field.get_mut(&mut unchecked) = text.trim().parse().map_err(|_| FieldError {
            fields: vec![field],
            message: "Must be a number.".to_string(),
        })?;
    }
    let trims = Edge::ALL.map(|edge| (edge, trim_field(edge).get(&unchecked)));
    unchecked
        .with_trims(Orientation::Landscape, &trims)
        .map_err(|err| FieldError {
            fields: unchecked
                .validate()
                .err()
                .map_or_else(Vec::new, |invalid| invalid_fields(&invalid)),
            message: sentence(&err.to_string()),
        })
}

/// The fields a failed check is about.
fn invalid_fields(invalid: &Invalid) -> Vec<&'static ConfigField> {
    let side = |side: &Side| match side {
        Side::Long => [Trim::LongA, Trim::LongB],
        Side::Short => [Trim::ShortA, Trim::ShortB],
    };
    match invalid {
        Invalid::Canvas {
            side: Side::Long, ..
        } => vec![&CANVAS_LONG],
        Invalid::Canvas {
            side: Side::Short, ..
        } => vec![&CANVAS_SHORT],
        Invalid::NegativeTrim { trim, .. } => vec![ConfigField::for_trim(*trim)],
        Invalid::Stretch { .. } => vec![&MAX_STRETCH],
        Invalid::NothingLeft { side: s, .. } => {
            side(s).map(ConfigField::for_trim).into_iter().collect()
        }
    }
}

/// One number field of the pane.
struct Row {
    label: &'static str,
    unit: &'static str,
    field: &'static ConfigField,
    input: Entity<InputState>,
}

/// What the last Save or Save Calibration Sheet did.
enum Notice {
    Done(SharedString),
    Failed(SharedString),
}

pub struct PrinterPane {
    prefs: Entity<Prefs>,
    paper: Paper,
    /// The file as it was read, and the overrides. `None` when the file
    /// cannot be read; Save then replaces it.
    loaded: Option<Loaded>,
    load_error: Option<SharedString>,
    trims: Vec<Row>,
    canvas: Vec<Row>,
    error: Option<FieldError>,
    notice: Option<Notice>,
}

impl PrinterPane {
    /// Starts at the main window's paper, with the file's values.
    pub fn new(prefs: Entity<Prefs>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let paper = prefs.read(cx).settings().paper;
        let mut input = |label, unit, field| Row {
            label,
            unit,
            field,
            input: cx.new(|cx| InputState::new(window, cx).step(0.1).min(0.0)),
        };
        let trims = Edge::ALL
            .into_iter()
            .map(|edge| input(edge_label(edge), "mm", trim_field(edge)))
            .collect();
        let canvas = vec![
            input("Long side", "mm", &CANVAS_LONG),
            input("Short side", "mm", &CANVAS_SHORT),
            input("Max stretch", "%", &MAX_STRETCH),
        ];
        let mut pane = Self {
            prefs,
            paper,
            loaded: None,
            load_error: None,
            trims,
            canvas,
            error: None,
            notice: None,
        };
        pane.load(paper, window, cx);
        pane
    }

    fn rows(&self) -> impl Iterator<Item = &Row> {
        self.trims.iter().chain(&self.canvas)
    }

    /// Reads the file for `paper` and puts its profile in the fields: its
    /// table, else its starting profile.
    fn load(&mut self, paper: Paper, window: &mut Window, cx: &mut Context<Self>) {
        self.paper = paper;
        let file = self.prefs.read(cx).config().clone();
        // The fit does not matter here; naming it keeps a bad SELPHY_FIT
        // from hiding the profile.
        match file.load(Some(paper), Some(Fit::Contain)) {
            Ok(loaded) => {
                self.loaded = Some(loaded);
                self.load_error = None;
            }
            Err(err) => {
                self.loaded = None;
                let message = format!("{} Saving replaces the file.", error_sentence(&err));
                self.load_error = Some(message.into());
            }
        }
        let profile = self
            .saved()
            .profile(paper)
            .unwrap_or_else(|| paper.starting_profile());
        self.fill(&profile, window, cx);
    }

    fn saved(&self) -> Config {
        self.loaded
            .as_ref()
            .map_or_else(Config::default, |loaded| loaded.saved.clone())
    }

    fn is_calibrated(&self) -> bool {
        self.saved().profile(self.paper).is_some()
    }

    fn fill(&mut self, profile: &Profile, window: &mut Window, cx: &mut Context<Self>) {
        for row in self.trims.iter().chain(&self.canvas) {
            let value = row.field.get(profile).to_string();
            row.input
                .update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.error = None;
        cx.notify();
    }

    /// The profile in the fields. Shows the error next to its field.
    fn read(&mut self, cx: &mut Context<Self>) -> Option<Profile> {
        let texts: Vec<(&'static ConfigField, SharedString)> = self
            .rows()
            .map(|row| (row.field, row.input.read(cx).value()))
            .collect();
        let values: Vec<_> = texts.iter().map(|(f, t)| (*f, t.as_ref())).collect();
        let start = self.paper.starting_profile();
        let result = read_profile(&start, &values);
        self.error = result.as_ref().err().cloned();
        cx.notify();
        result.ok()
    }

    /// Writes the paper's profile into `printer.toml`, then makes it the
    /// main window's paper in `gui.toml`. Says which file was saved when
    /// only the first could be.
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = self.read(cx) else {
            return;
        };
        let file = self.prefs.read(cx).config().clone();
        let updated = self.saved().with_profile(self.paper, profile);
        if let Err(err) = file.save(&updated) {
            let message = format!(
                "Couldn't save {}. {}",
                display_path(file.path()),
                error_sentence(&err)
            );
            self.notice = Some(Notice::Failed(message.into()));
            return cx.notify();
        }
        let paper = self.paper;
        let gui = self.prefs.update(cx, |prefs, cx| {
            prefs.printer_saved();
            let result = prefs.change(|settings| settings.paper = paper);
            cx.notify();
            result.map_err(|err| (prefs.path(), err))
        });
        let saved = display_path(file.path());
        self.notice = Some(match gui {
            Ok(()) => Notice::Done(format!("Saved to {saved}.").into()),
            Err((path, err)) => Notice::Failed(
                format!(
                    "Saved to {saved}, but couldn't save {}. {}",
                    display_path(&path),
                    error_sentence(&err)
                )
                .into(),
            ),
        });
        self.load(paper, window, cx);
    }

    /// Puts the paper's starting profile in the fields. Nothing is written
    /// until Save.
    fn restore_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = self.paper.starting_profile();
        self.notice = None;
        self.fill(&profile, window, cx);
    }

    /// Writes the calibration sheet for the fields' profile, with the env
    /// overrides, as `selphy calibrate` does.
    fn save_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = self.read(cx) else {
            return;
        };
        let profile = match &self.loaded {
            Some(loaded) => match loaded.apply_overrides(profile) {
                Ok(profile) => profile,
                Err(err) => {
                    self.notice = Some(Notice::Failed(error_sentence(&err)));
                    return cx.notify();
                }
            },
            None => profile,
        };
        let paper = self.paper;
        let out_dir = self.prefs.read(cx).settings().out_dir.clone();
        let dir = if out_dir.is_dir() {
            out_dir
        } else {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
        };
        let name = format!("calibration-{paper}-landscape.jpg");
        let picked = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = picked.await else {
                return;
            };
            let written = path.clone();
            let result = cx
                .background_spawn(async move {
                    let font = calibrate::load_font(calibrate::DEFAULT_FONT.as_ref())?;
                    calibrate::write_sheet(paper, &profile, Orientation::Landscape, &font, &path)
                })
                .await;
            this.update(cx, |this, cx| {
                this.notice = Some(match result {
                    Ok(()) => Notice::Done(
                        format!(
                            "Wrote {}. Print it Borderless; on each edge, the smallest number \
                             whose line still shows is the trim.",
                            display_path(&written)
                        )
                        .into(),
                    ),
                    Err(err) => Notice::Failed(error_sentence(&err)),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_paper(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.paper;
        let saved = self.saved();
        let view = cx.weak_entity();
        let menu = Button::new("printer-paper")
            .small()
            .outline()
            .label(paper_label(current))
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for paper in Paper::ALL {
                    let label = if saved.profile(paper).is_some() {
                        paper_label(paper).to_string()
                    } else {
                        format!("{} (not calibrated)", paper_label(paper))
                    };
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(paper == current)
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.notice = None;
                                    this.load(paper, window, cx)
                                })
                                .ok();
                            }),
                    );
                }
                menu
            });
        let muted = cx.theme().muted_foreground;
        let note = (!self.is_calibrated()).then(|| {
            format!(
                "{} paper is not calibrated. Save a calibration sheet, print it, and enter the \
                 trims; or enter measured trims, then Save.",
                paper_label(current)
            )
        });
        v_flex()
            .gap_2()
            .child(menu)
            .when_some(note, |this, note| {
                this.child(div().text_sm().text_color(muted).child(note))
            })
            .into_any_element()
    }

    fn render_rows(&self, rows: &[Row], cx: &mut Context<Self>) -> AnyElement {
        let danger = cx.theme().danger;
        let fields = rows.iter().map(|row| {
            let error = self
                .error
                .as_ref()
                .filter(|error| error.fields.contains(&row.field))
                .map(|error| error.message.clone());
            let overridden = self
                .loaded
                .as_ref()
                .and_then(|loaded| loaded.override_of(row.field))
                .map(|o| format!("Prepare uses {} from {}.", o.value, o.field.env));
            Field::new()
                .label(row.label)
                .child(
                    NumberInput::new(&row.input)
                        .small()
                        .suffix(div().text_xs().child(row.unit)),
                )
                .when_some(
                    error
                        .map(|text| (text, true))
                        .or(overridden.map(|text| (text, false))),
                    |field, (text, is_error)| {
                        field.description_fn(move |_, cx| {
                            let color = if is_error {
                                danger
                            } else {
                                cx.theme().muted_foreground
                            };
                            div().text_xs().text_color(color).child(text.clone())
                        })
                    },
                )
        });
        Form::new()
            .small()
            .columns(rows.len())
            .children(fields)
            .into_any_element()
    }

    fn render_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let notice = self.notice.as_ref().map(|notice| match notice {
            Notice::Done(text) => (text.clone(), theme.muted_foreground),
            Notice::Failed(text) => (text.clone(), theme.danger),
        });
        let unplaced = self
            .error
            .as_ref()
            .filter(|error| error.fields.is_empty())
            .map(|error| error.message.clone());
        let overrides = self.overrides_note();
        let path = self.prefs.read(cx).config().path().to_path_buf();
        v_flex()
            .gap_3()
            .when_some(self.load_error.clone(), |this, error| {
                this.child(div().text_sm().text_color(theme.danger).child(error))
            })
            .when_some(overrides, |this, note| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(note),
                )
            })
            .when_some(unplaced, |this, error| {
                this.child(div().text_sm().text_color(theme.danger).child(error))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("save-sheet")
                            .small()
                            .label("Save Calibration Sheet…")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_sheet(window, cx)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("restore-defaults")
                            .small()
                            .ghost()
                            .label("Restore Defaults")
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.restore_defaults(window, cx)
                                }),
                            ),
                    )
                    .child(
                        Button::new("save-printer")
                            .small()
                            .primary()
                            .label("Save")
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
            .when_some(notice, |this, (text, color)| {
                this.child(div().text_sm().text_color(color).child(text))
            })
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(display_path(&path)),
            )
            .into_any_element()
    }

    /// "SELPHY_TRIM_LONG_A_MM is set. Prepare uses its value, not the one
    /// saved here.", naming every override that is set.
    fn overrides_note(&self) -> Option<String> {
        let overrides = &self.loaded.as_ref()?.overrides;
        if overrides.is_empty() {
            return None;
        }
        let names: Vec<&str> = overrides.iter().map(|o| o.field.env).collect();
        let verb = if names.len() == 1 { "is" } else { "are" };
        Some(format!(
            "{} {verb} set. Prepare uses the values under the fields, not the ones saved here.",
            names.join(", ")
        ))
    }
}

/// The edge's name as the pane labels it.
fn edge_label(edge: Edge) -> &'static str {
    match edge {
        Edge::Left => "Left",
        Edge::Top => "Top",
        Edge::Right => "Right",
        Edge::Bottom => "Bottom",
    }
}

/// The Printer page of the Settings window.
pub fn page(pane: &Entity<PrinterPane>) -> SettingPage {
    let part = |render: fn(&PrinterPane, &mut Context<PrinterPane>) -> AnyElement| {
        let pane = pane.clone();
        SettingItem::render(move |_, _, cx| pane.update(cx, |pane, cx| render(pane, cx)))
    };
    SettingPage::new("Printer")
        .resettable(false)
        .group(
            SettingGroup::new()
                .title("Paper")
                .item(part(PrinterPane::render_paper).keywords(["paper", "calibrated"])),
        )
        .group(
            SettingGroup::new()
                .title("Trims")
                .description("Card lost at each edge of a landscape print.")
                .item(
                    part(|pane, cx| pane.render_rows(&pane.trims, cx)).keywords(["trim", "edge"]),
                ),
        )
        .group(
            SettingGroup::new()
                .title("Canvas")
                .description("The image sent to the printer.")
                .item(
                    part(|pane, cx| pane.render_rows(&pane.canvas, cx))
                        .keywords(["canvas", "stretch"]),
                ),
        )
        .group(
            SettingGroup::new().item(part(PrinterPane::render_actions).keywords([
                "save",
                "calibration",
                "sheet",
                "defaults",
            ])),
        )
}

#[cfg(test)]
mod tests {
    use selphy::config::fields::{TRIM_LONG_A, TRIM_LONG_B, TRIM_SHORT_B};

    use super::*;

    #[test]
    fn each_trim_field_is_the_trim_on_its_landscape_edge() {
        let profile = Profile {
            trim_long_a_mm: 1.0,
            trim_long_b_mm: 2.0,
            trim_short_a_mm: 3.0,
            trim_short_b_mm: 4.0,
            ..Paper::Postcard.starting_profile()
        };
        for edge in Edge::ALL {
            assert_eq!(
                trim_field(edge).get(&profile),
                Trim::at(Orientation::Landscape, edge).mm(&profile),
                "{edge:?}"
            );
        }
    }

    fn values(pairs: &[(&'static ConfigField, &'static str)]) -> Result<Profile, FieldError> {
        read_profile(&Paper::Postcard.starting_profile(), pairs)
    }

    #[test]
    fn typed_values_go_into_their_fields() {
        let profile = values(&[
            (trim_field(Edge::Left), "3.5"),
            (trim_field(Edge::Bottom), " 1 "),
            (&MAX_STRETCH, "0"),
        ])
        .unwrap();
        assert_eq!(profile.trim_long_a_mm, 3.5);
        assert_eq!(profile.trim_short_b_mm, 1.0);
        assert_eq!(profile.max_stretch_pct, 0.0);
    }

    #[test]
    fn a_negative_trim_is_an_error_on_its_edge_in_edge_words() {
        let err = values(&[(trim_field(Edge::Bottom), "-1")]).unwrap_err();
        assert_eq!(err.fields, [&TRIM_SHORT_B]);
        assert_eq!(
            err.message,
            "The bottom trim is -1 mm: it must be 0 or more."
        );
    }

    #[test]
    fn trims_that_leave_nothing_are_an_error_on_both_fields() {
        let err = values(&[
            (trim_field(Edge::Left), "80"),
            (trim_field(Edge::Right), "80"),
        ])
        .unwrap_err();
        assert_eq!(err.fields, [&TRIM_LONG_A, &TRIM_LONG_B]);
        assert!(
            err.message.starts_with("The left and right trims"),
            "{}",
            err.message
        );
    }

    #[test]
    fn a_bad_canvas_or_stretch_is_an_error_on_that_field() {
        let err = values(&[(&CANVAS_SHORT, "0")]).unwrap_err();
        assert_eq!(err.fields, [&CANVAS_SHORT]);
        let err = values(&[(&MAX_STRETCH, "-2")]).unwrap_err();
        assert_eq!(err.fields, [&MAX_STRETCH]);
    }

    #[test]
    fn text_that_is_not_a_number_is_an_error_on_its_field() {
        let err = values(&[(&CANVAS_LONG, "wide")]).unwrap_err();
        assert_eq!(err.fields, [&CANVAS_LONG]);
        assert_eq!(err.message, "Must be a number.");
    }
}
