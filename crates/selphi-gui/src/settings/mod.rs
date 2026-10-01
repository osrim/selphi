//! The Settings window, opened with Settings… (Cmd-,): the printer profile,
//! the output settings and the theme. Save checks the values, writes
//! `printer.toml` then `gui.toml`, and closes the window. Cancel, Escape or
//! closing the window keeps the files as they were and puts the previous
//! theme back. Closing it leaves the app running.

use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    radio::RadioGroup,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::FluentBuilder as _, px, size,
};

use crate::state::{Prefs, ThemeChoice};
use crate::text::{display_path, error_sentence};
use crate::{CancelSettings, SaveSettings};

pub mod output;
pub mod printer;

use output::OutputForm;
use printer::PrinterForm;

/// The open Settings window, if any.
#[derive(Default)]
struct SettingsWindowHandle(Option<AnyWindowHandle>);

impl Global for SettingsWindowHandle {}

pub struct SettingsWindow {
    prefs: Entity<Prefs>,
    printer: Entity<PrinterForm>,
    output: Entity<OutputForm>,
    /// The theme chosen here, shown at once.
    theme: ThemeChoice,
    /// The theme the windows had when Settings opened, or since the last
    /// Save. Closing without saving puts it back.
    kept_theme: ThemeChoice,
    /// Why the last Save did not finish.
    error: Option<SharedString>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    fn new(prefs: Entity<Prefs>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (file, settings) = {
            let prefs = prefs.read(cx);
            (prefs.config().clone(), prefs.settings().clone())
        };
        let printer = cx.new(|cx| PrinterForm::new(file, settings.out_dir, window, cx));
        let look = printer.read(cx).saved().look();
        let output = cx.new(|_| OutputForm::new(look));
        let subscriptions = vec![
            cx.observe(&printer, |_, _, cx| cx.notify()),
            cx.observe(&output, |_, _, cx| cx.notify()),
            // However the window closes, an unsaved theme is taken back.
            cx.on_release(|this, cx| {
                if this.theme != this.kept_theme {
                    this.kept_theme.apply(cx);
                }
            }),
        ];
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self {
            prefs,
            printer,
            output,
            theme: settings.theme,
            kept_theme: settings.theme,
            error: None,
            focus_handle,
            _subscriptions: subscriptions,
        }
    }

    fn choose_theme(&mut self, theme: ThemeChoice, cx: &mut Context<Self>) {
        self.theme = theme;
        theme.apply(cx);
        cx.notify();
    }

    fn restore_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.printer
            .update(cx, |printer, cx| printer.restore_defaults(window, cx));
        self.output
            .update(cx, |output, cx| output.restore_defaults(cx));
        self.choose_theme(ThemeChoice::default(), cx);
    }

    /// Writes `printer.toml`, then `gui.toml`, and closes the window. An
    /// invalid value, or a file that cannot be written, keeps it open and
    /// says why, naming the file that was saved when only one was.
    fn save(&mut self, _: &SaveSettings, window: &mut Window, cx: &mut Context<Self>) {
        let Some((profile, saved)) = self
            .printer
            .update(cx, |printer, cx| Some((printer.read(cx)?, printer.saved())))
        else {
            return;
        };
        let look = self.output.read(cx).look();
        let file = self.prefs.read(cx).config().clone();
        if let Err(err) = file.save(&saved.with_profile(profile).with_look(look)) {
            let path = display_path(file.path());
            self.error = Some(format!("Couldn't save {path}. {}", error_sentence(&err)).into());
            return cx.notify();
        }
        let theme = self.theme;
        let gui = self.prefs.update(cx, |prefs, cx| {
            prefs.printer_saved();
            let result = prefs.change(|settings| settings.theme = theme);
            cx.notify();
            result.map_err(|err| (prefs.path(), err))
        });
        self.kept_theme = theme;
        match gui {
            Ok(()) => window.remove_window(),
            Err((path, err)) => {
                let message = format!(
                    "Saved {}, but couldn't save {}. {}",
                    display_path(file.path()),
                    display_path(&path),
                    error_sentence(&err)
                );
                self.error = Some(message.into());
                cx.notify();
            }
        }
    }

    fn cancel(&mut self, _: &CancelSettings, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = ThemeChoice::ALL.iter().position(|&t| t == self.theme);
        RadioGroup::horizontal("theme")
            .children(ThemeChoice::ALL.map(ThemeChoice::label))
            .selected_index(selected)
            .on_change(
                cx.listener(|this, ix: &usize, _, cx| this.choose_theme(ThemeChoice::ALL[*ix], cx)),
            )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .px_6()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("restore-defaults")
                    .ghost()
                    .label("Restore Defaults")
                    .on_click(cx.listener(|this, _, window, cx| this.restore_defaults(window, cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .when_some(self.error.clone(), |this, error| this.child(error)),
            )
            .child(Button::new("cancel-settings").label("Cancel").on_click(
                cx.listener(|this, _, window, cx| this.cancel(&CancelSettings, window, cx)),
            ))
            .child(
                Button::new("save-settings")
                    .primary()
                    .label("Save")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.save(&SaveSettings, window, cx)),
                    ),
            )
    }
}

/// A titled section of the form.
fn section(title: &'static str, content: impl IntoElement) -> impl IntoElement {
    v_flex()
        .gap_3()
        .child(div().text_base().font_semibold().child(title))
        .child(content)
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("settings")
            .key_context("SettingsWindow")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::cancel))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div().flex_1().min_h_0().child(
                    v_flex()
                        .id("settings-form")
                        .size_full()
                        .overflow_y_scrollbar()
                        .gap_8()
                        .px_6()
                        .py_5()
                        .child(section("Printer", self.printer.clone()))
                        .child(section("Output", self.output.clone()))
                        .child(section("Appearance", self.render_appearance(cx))),
                ),
            )
            .child(self.render_footer(cx))
    }
}

/// Opens the Settings window, or brings the open one to the front.
pub fn open(prefs: Entity<Prefs>, cx: &mut App) -> anyhow::Result<AnyWindowHandle> {
    if let Some(handle) = cx.default_global::<SettingsWindowHandle>().0
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return Ok(handle);
    }
    // Window geometry is a platform boundary, so it is in pixels.
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Settings".into()),
            ..Default::default()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(640.), px(560.)),
            cx,
        ))),
        window_min_size: Some(size(px(560.), px(440.))),
        ..Default::default()
    };
    let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsWindow::new(prefs, window, cx))
    })?;
    cx.set_global(SettingsWindowHandle(Some(handle)));
    Ok(handle)
}
