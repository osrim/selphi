//! The Config dialog body: the window's Appearance, and the Printer values that
//! `selphy` reads. Each section is a heading over groups of fields. The
//! Printer values are the postcard profile; the other papers' tables are
//! kept as they are.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    form::{Field, Form},
    h_flex,
    input::{InputState, NumberInput},
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, Hsla, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _,
};

use selphy::config::fields::{
    CANVAS_LONG, CANVAS_SHORT, Field as ConfigField, MAX_STRETCH, TRIM_LONG_A, TRIM_LONG_B,
    TRIM_SHORT_A, TRIM_SHORT_B,
};
use selphy::config::{Config, ConfigFile, Profile};
use selphy::geometry::Fit;
use selphy::paper::Paper;
use selphy::toml_file;

use crate::appearance::{self, Appearance, ThemeChoice};
use crate::batch_view::error_sentence;

/// One editable value: its label, unit, and field in `Profile`.
struct Setting {
    label: &'static str,
    unit: &'static str,
    field: &'static ConfigField,
}

const TRIMS: [Setting; 4] = [
    Setting {
        label: "Left",
        unit: "mm",
        field: &TRIM_LONG_A,
    },
    Setting {
        label: "Right",
        unit: "mm",
        field: &TRIM_LONG_B,
    },
    Setting {
        label: "Top",
        unit: "mm",
        field: &TRIM_SHORT_A,
    },
    Setting {
        label: "Bottom",
        unit: "mm",
        field: &TRIM_SHORT_B,
    },
];

const CANVAS: [Setting; 3] = [
    Setting {
        label: "Long side",
        unit: "mm",
        field: &CANVAS_LONG,
    },
    Setting {
        label: "Short side",
        unit: "mm",
        field: &CANVAS_SHORT,
    },
    Setting {
        label: "Largest stretch",
        unit: "%",
        field: &MAX_STRETCH,
    },
];

/// The body of the Config dialog; owns its inputs while it is open.
pub struct ConfigPanel {
    config: ConfigFile,
    /// The file as it was read. Save replaces its postcard table.
    saved: Config,
    theme: ThemeChoice,
    trims: Vec<Entity<InputState>>,
    canvas: Vec<Entity<InputState>>,
    error: Option<SharedString>,
}

impl ConfigPanel {
    /// Starts from the window's current theme and the postcard values in the
    /// printer config file, without the env overrides. A file that cannot be
    /// read shows its error and starts from the defaults; saving then
    /// replaces it.
    pub fn new(
        theme: ThemeChoice,
        config: ConfigFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (saved, error) = match config.load(None, Some(Fit::Contain)) {
            Ok(loaded) => (loaded.saved, None),
            Err(err) => (
                Config::default(),
                Some(format!("{} Saving replaces the file.", error_sentence(&err)).into()),
            ),
        };
        let postcard = saved
            .profile(Paper::Postcard)
            .expect("postcard has a built-in profile");
        let mut input = |setting: &Setting| {
            let value = setting.field.get(&postcard);
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value.to_string())
                    .step(0.1)
                    .min(0.0)
            })
        };
        let trims = TRIMS.iter().map(&mut input).collect();
        let canvas = CANVAS.iter().map(&mut input).collect();
        Self {
            config,
            saved,
            theme,
            trims,
            canvas,
            error,
        }
    }

    /// Writes both files and returns the saved theme. Returns `None`, and
    /// shows why, when a value is not a number or a file cannot be written.
    pub fn save(&mut self, cx: &mut Context<Self>) -> Option<ThemeChoice> {
        let appearance = Appearance { theme: self.theme };
        let result = self.read(cx).and_then(|postcard| {
            let gui_path = self.config.sibling(appearance::FILE_NAME);
            self.config
                .save(&self.saved.with_profile(Paper::Postcard, postcard))
                .and_then(|()| toml_file::save(&gui_path, "", &appearance))
                .map_err(|err| format!("Couldn't save. {}", error_sentence(&err)))
        });
        match result {
            Ok(()) => Some(self.theme),
            Err(message) => {
                self.error = Some(message.into());
                cx.notify();
                None
            }
        }
    }

    /// Puts the postcard defaults in every field. Nothing is written until
    /// Save.
    pub fn restore_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let postcard = Paper::Postcard.starting_profile();
        let fields = TRIMS.iter().zip(&self.trims);
        for (setting, input) in fields.chain(CANVAS.iter().zip(&self.canvas)) {
            let value = setting.field.get(&postcard).to_string();
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.theme = ThemeChoice::default();
        self.error = None;
        cx.notify();
    }

    /// The postcard profile in the fields.
    fn read(&self, cx: &Context<Self>) -> Result<Profile, String> {
        let mut postcard = Paper::Postcard.starting_profile();
        let fields = TRIMS.iter().zip(&self.trims);
        for (setting, input) in fields.chain(CANVAS.iter().zip(&self.canvas)) {
            let text = input.read(cx).value();
            *setting.field.get_mut(&mut postcard) = text
                .trim()
                .parse()
                .map_err(|_| format!("{} must be a number.", setting.label))?;
        }
        Ok(postcard)
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = ThemeChoice::ALL.iter().position(|&t| t == self.theme);
        let theme = Field::new().label("Theme").child(
            RadioGroup::horizontal("theme")
                .children(ThemeChoice::ALL.map(ThemeChoice::label))
                .selected_index(selected)
                .on_change(cx.listener(|this, ix: &usize, _, cx| {
                    this.theme = ThemeChoice::ALL[*ix];
                    cx.notify();
                })),
        );
        section(
            "Appearance",
            cx.theme().border,
            fields_form(1, [theme]).into_any_element(),
        )
    }

    fn render_printer(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let groups = v_flex()
            .gap_6()
            .child(group(
                "Trim",
                "Card lost at each edge of a landscape print.",
                number_fields(&TRIMS, &self.trims),
                muted,
            ))
            .child(group(
                "Canvas",
                "The page sent to the printer.",
                number_fields(&CANVAS, &self.canvas),
                muted,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(self.config.path().display().to_string()),
            );
        section("Printer", cx.theme().border, groups.into_any_element())
    }
}

/// A top-level section: a heading above a hairline, then its content.
fn section(title: &'static str, border: Hsla, content: AnyElement) -> AnyElement {
    v_flex()
        .gap_4()
        .child(
            div()
                .pb_2()
                .border_b_1()
                .border_color(border)
                .text_base()
                .font_semibold()
                .child(title),
        )
        .child(content)
        .into_any_element()
}

/// A group of fields within a section: a smaller heading and a description.
fn group(
    title: &'static str,
    description: &'static str,
    fields: Vec<Field>,
    muted: Hsla,
) -> impl IntoElement {
    let columns = fields.len();
    v_flex()
        .gap_2()
        .child(
            v_flex()
                .gap_1()
                .child(div().text_sm().font_medium().child(title))
                .child(div().text_xs().text_color(muted).child(description)),
        )
        .child(fields_form(columns, fields))
}

/// One row of fields: `columns` is how many sit side by side.
fn fields_form(columns: usize, fields: impl IntoIterator<Item = Field>) -> Form {
    Form::new().small().columns(columns).children(fields)
}

fn number_fields(settings: &[Setting], inputs: &[Entity<InputState>]) -> Vec<Field> {
    settings
        .iter()
        .zip(inputs)
        .map(|(setting, input)| {
            Field::new().label(setting.label).child(
                NumberInput::new(input)
                    .small()
                    .suffix(div().text_xs().child(setting.unit)),
            )
        })
        .collect()
}

impl Render for ConfigPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_8()
            .pt_4()
            .child(self.render_appearance(cx))
            .child(self.render_printer(cx))
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(Icon::new(IconName::CircleX).small())
                        .child(div().flex_1().child(error)),
                )
            })
    }
}
