//! The Settings window, opened with Settings… (Cmd-,): a Printer pane and an
//! Appearance pane. There is one Settings window; opening it again brings
//! it to the front. Closing it leaves the app running.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::setting::Settings;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, Global, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, px, size,
};

use crate::state::Prefs;

mod appearance;
pub mod printer;

use printer::PrinterPane;

/// The open Settings window, if any.
#[derive(Default)]
struct SettingsWindowHandle(Option<AnyWindowHandle>);

impl Global for SettingsWindowHandle {}

pub struct SettingsWindow {
    prefs: Entity<Prefs>,
    printer: Entity<PrinterPane>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    fn new(prefs: Entity<Prefs>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let printer = cx.new(|cx| PrinterPane::new(prefs.clone(), window, cx));
        // The pages are drawn from both, so a change to either redraws them.
        let subscriptions = vec![
            cx.observe(&printer, |_, _, cx| cx.notify()),
            cx.observe(&prefs, |_, _, cx| cx.notify()),
        ];
        Self {
            prefs,
            printer,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                Settings::new("settings")
                    .sidebar_width(px(180.))
                    .page(printer::page(&self.printer))
                    .page(appearance::page(&self.prefs)),
            )
    }
}

/// Opens the Settings window, or brings it to the front when it is open.
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
            size(px(760.), px(620.)),
            cx,
        ))),
        window_min_size: Some(size(px(600.), px(480.))),
        ..Default::default()
    };
    let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsWindow::new(prefs, window, cx))
    })?;
    cx.set_global(SettingsWindowHandle(Some(handle)));
    Ok(handle)
}
