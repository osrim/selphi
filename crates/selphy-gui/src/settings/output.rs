//! The Output section of the Settings window: the sharpening and the
//! background, which the window saves into `printer.toml` next to the
//! profile, and the printer settings that the outputs need.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    form::{Field, Form},
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window, div};
use selphy::imaging::{Background, Look, Sharpening};

/// The printer settings a prepared output needs, from the CP1500 manual's
/// Print Settings menu.
const PRINTER_NOTE: &str = "On the SELPHY, set Borders to Borderless and Page Layout to 1-up: \
                            the trims are measured for both. Turn Image Optimize off, so that \
                            the printer does not change the brightness and contrast again, and \
                            Date off.";

pub struct OutputForm {
    look: Look,
}

impl OutputForm {
    /// The form with `look`, the output settings of the loaded file.
    pub fn new(look: Look) -> Self {
        Self { look }
    }

    /// The output settings chosen in the form.
    pub fn look(&self) -> Look {
        self.look
    }

    /// Puts the default output settings in the form. Nothing is written
    /// until Save.
    pub fn restore_defaults(&mut self, cx: &mut Context<Self>) {
        self.look = Look::default();
        cx.notify();
    }
}

impl Render for OutputForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let sharpening = RadioGroup::horizontal("sharpening")
            .children(Sharpening::ALL.map(Sharpening::label))
            .selected_index(
                Sharpening::ALL
                    .iter()
                    .position(|&s| s == self.look.sharpening),
            )
            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                this.look.sharpening = Sharpening::ALL[*ix];
                cx.notify();
            }));
        let background = RadioGroup::horizontal("background")
            .children(Background::ALL.map(Background::label))
            .selected_index(
                Background::ALL
                    .iter()
                    .position(|&b| b == self.look.background),
            )
            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                this.look.background = Background::ALL[*ix];
                cx.notify();
            }));
        let description = move |text: &'static str| {
            move |_: &mut Window, _: &mut gpui_kit::App| {
                div().text_xs().text_color(muted).child(text)
            }
        };
        v_flex()
            .gap_5()
            .child(
                Form::new()
                    .small()
                    .child(
                        Field::new()
                            .label("Sharpening")
                            .child(sharpening)
                            .description_fn(description(
                                "Applied after the photo is resized to the card.",
                            )),
                    )
                    .child(
                        Field::new()
                            .label("Background")
                            .child(background)
                            .description_fn(description(
                                "Shows around a Contain photo. Outputs are always sRGB.",
                            )),
                    ),
            )
            .child(div().text_sm().text_color(muted).child(PRINTER_NOTE))
    }
}
