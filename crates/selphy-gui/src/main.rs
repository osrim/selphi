//! `selphy-gui`: a window for `selphy prepare`. Add photos, see each one's
//! card, choose the fit, and prepare them. Settings… edits the
//! printer config the command line reads.

use std::path::PathBuf;

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, KeyBinding, Menu, MenuItem,
    TitlebarOptions, WindowBounds, WindowOptions, actions, px, size,
};
use selphy::config::ConfigFile;

mod batch;
mod main_window;
mod photo_list;
mod preview;
mod run;
mod settings;
mod state;
mod text;
mod thumbnails;

#[cfg(test)]
mod flow_tests;

use main_window::{MainWindow, show_error};
use state::{GuiSettings, Prefs};
use text::error_sentence;

actions!(
    selphy,
    [
        AddPhotos,
        CancelPrepare,
        CancelSettings,
        ChooseFolder,
        ClearPhotos,
        OpenSettings,
        Prepare,
        Quit,
        Redo,
        RemoveSelected,
        SaveSettings,
        SelectNext,
        SelectPrevious,
        Undo,
    ]
);

/// The key context of the main window.
const MAIN: Option<&str> = Some("MainWindow");

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            init_app(cx);
            let config = ConfigFile::locate(
                // Empty counts as not set, as it does for the CLI's --config.
                std::env::var_os("SELPHY_CONFIG")
                    .filter(|path| !path.is_empty())
                    .map(PathBuf::from),
            );
            open_main_window(config, |cx| cx.quit(), cx).expect("failed to open the window");
            cx.activate(true);
        });
}

/// Binds the keys and sets the menus.
fn init_app(cx: &mut App) {
    // Keys before menus: `set_menus` reads the keymap when it is called.
    cx.bind_keys([
        KeyBinding::new("cmd-o", AddPhotos, MAIN),
        KeyBinding::new("cmd-shift-o", ChooseFolder, MAIN),
        KeyBinding::new("cmd-enter", Prepare, MAIN),
        KeyBinding::new("escape", CancelPrepare, MAIN),
        KeyBinding::new("up", SelectPrevious, MAIN),
        KeyBinding::new("down", SelectNext, MAIN),
        KeyBinding::new("delete", RemoveSelected, MAIN),
        KeyBinding::new("backspace", RemoveSelected, MAIN),
        KeyBinding::new("cmd-z", Undo, MAIN),
        KeyBinding::new("cmd-shift-z", Redo, MAIN),
        KeyBinding::new("escape", CancelSettings, Some("SettingsWindow")),
        KeyBinding::new("cmd-s", SaveSettings, Some("SettingsWindow")),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.set_menus([
        Menu::new("selphy").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Quit selphy", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("Add…", AddPhotos),
            MenuItem::action("Choose Output Folder…", ChooseFolder),
            MenuItem::separator(),
            MenuItem::action("Prepare", Prepare),
            MenuItem::action("Cancel", CancelPrepare),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", Undo),
            MenuItem::action("Redo", Redo),
            MenuItem::separator(),
            MenuItem::action("Remove", RemoveSelected),
            MenuItem::action("Clear", ClearPhotos),
        ]),
    ]);
}

/// Opens the main window for `config`, with the window settings from
/// `gui.toml` beside it. Settings… opens the Settings window for the same
/// files. Closing the main window calls `on_main_closed`, which quits the
/// app; closing another window does not.
fn open_main_window(
    config: ConfigFile,
    on_main_closed: impl Fn(&mut App) + 'static,
    cx: &mut App,
) -> anyhow::Result<(AnyWindowHandle, Entity<MainWindow>)> {
    let gui_path = config.sibling(state::FILE_NAME);
    let (settings, load_error) = match GuiSettings::load(&gui_path) {
        Ok(settings) => (settings, None),
        Err(err) => (GuiSettings::default(), Some(error_sentence(&err))),
    };
    settings.theme.apply(cx);
    let prefs = cx.new(|_| Prefs::new(config, settings));
    cx.on_action({
        let prefs = prefs.clone();
        move |_: &OpenSettings, cx| {
            if let Err(err) = settings::open(prefs.clone(), cx) {
                eprintln!("selphy-gui: couldn't open Settings: {err:#}");
            }
        }
    });

    // Window geometry is a platform boundary, so it is in pixels.
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("selphy".into()),
            ..Default::default()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1000.), px(680.)),
            cx,
        ))),
        window_min_size: Some(size(px(720.), px(480.))),
        ..Default::default()
    };
    let (handle, view) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| MainWindow::new(prefs, window, cx))
    })?;
    let main_id = handle.window_id();
    cx.on_window_closed(move |cx, closed| {
        if closed == main_id {
            on_main_closed(cx);
        }
    })
    .detach();
    if let Some(message) = load_error {
        handle.update(cx, |_, window, cx| {
            show_error("Couldn't read the window settings", message, window, cx)
        })?;
    }
    Ok((handle, view))
}
