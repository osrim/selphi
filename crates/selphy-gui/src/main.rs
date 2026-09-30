//! `selphy-gui`: a window for `selphy prepare`. Add photos, choose a folder,
//! and prepare them; the Config dialog edits the config file the command
//! line reads.

use std::path::PathBuf;

use gpui_kit::{
    AppContext as _, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions, actions, px, size,
};

mod appearance;
mod batch;
mod batch_view;
mod config_panel;

use selphy::config::ConfigFile;

use batch_view::BatchView;

actions!(
    selphy,
    [
        AddPhotos,
        CancelPrepare,
        ChooseFolder,
        OpenConfig,
        Prepare,
        Quit
    ]
);

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);

            // Keys before menus: `set_menus` reads the keymap when it is called.
            cx.bind_keys([
                KeyBinding::new("cmd-o", AddPhotos, Some("Batch")),
                KeyBinding::new("cmd-shift-o", ChooseFolder, Some("Batch")),
                KeyBinding::new("cmd-,", OpenConfig, Some("Batch")),
                KeyBinding::new("enter", Prepare, Some("Batch")),
                KeyBinding::new("escape", CancelPrepare, Some("Batch")),
                KeyBinding::new("cmd-q", Quit, None),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.set_menus([
                Menu::new("selphy").items([MenuItem::action("Quit selphy", Quit)]),
                Menu::new("File").items([
                    MenuItem::action("Add Photos…", AddPhotos),
                    MenuItem::action("Choose Folder…", ChooseFolder),
                    MenuItem::separator(),
                    MenuItem::action("Prepare", Prepare),
                    MenuItem::action("Cancel", CancelPrepare),
                    MenuItem::separator(),
                    MenuItem::action("Config…", OpenConfig),
                ]),
            ]);
            // One window: closing it ends the app.
            cx.on_window_closed(|cx, _| cx.quit()).detach();

            // Window geometry is a platform boundary, so it is in pixels.
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("selphy".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(800.), px(600.)),
                    cx,
                ))),
                window_min_size: Some(size(px(640.), px(480.))),
                ..Default::default()
            };
            let config = ConfigFile::locate(
                // Empty counts as not set, as it does for the CLI's --config.
                std::env::var_os("SELPHY_CONFIG")
                    .filter(|path| !path.is_empty())
                    .map(PathBuf::from),
            );
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| BatchView::new(config, window, cx))
            })
            .expect("failed to open the window");
            cx.activate(true);
        });
}
