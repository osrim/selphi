//! A few flows through the real windows: add and prepare, cancel, and
//! closing Settings. The model tests in `batch`, `run` and `state` carry
//! the logic.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use selphi::config::ConfigFile;
use selphi::test_util::{exif_with_orientation, fresh_dir, write_jpeg};

use crate::batch::Status;
use crate::main_window::MainWindow;
use crate::state::{self, GuiSettings};
use crate::{OpenSettings, init_app, open_main_window};

/// A folder with `count` photos, a printer config that does not exist yet,
/// and `gui.toml` pointing the output at `out`.
fn fixture(name: &str, count: usize) -> (PathBuf, Vec<PathBuf>, ConfigFile) {
    let dir = fresh_dir(name);
    let photos = (0..count)
        .map(|i| {
            write_jpeg(
                &dir.join(format!("{i}.jpg")),
                60,
                40,
                &exif_with_orientation(1),
            )
        })
        .collect();
    let config = ConfigFile::at(dir.join("printer.toml"));
    let settings = GuiSettings {
        out_dir: dir.join("out"),
        ..GuiSettings::default()
    };
    settings.save(&config.sibling(state::FILE_NAME)).unwrap();
    (dir, photos, config)
}

/// Opens the main window. Returns it, its view, and how many times closing
/// it would have quit the app.
fn open(
    config: ConfigFile,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<MainWindow>, Rc<Cell<usize>>) {
    let quits = Rc::new(Cell::new(0));
    let counter = quits.clone();
    let (handle, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        init_app(cx);
        open_main_window(config, move |_| counter.set(counter.get() + 1), cx).unwrap()
    });
    (handle, view, quits)
}

fn add(
    handle: AnyWindowHandle,
    view: &Entity<MainWindow>,
    photos: &[PathBuf],
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.add_paths(photos, window, cx));
    })
    .unwrap();
}

fn statuses(view: &Entity<MainWindow>, cx: &mut TestAppContext) -> Vec<Status> {
    cx.update(|cx| {
        view.read(cx)
            .batch
            .photos()
            .iter()
            .map(|photo| photo.status().clone())
            .collect()
    })
}

fn output(dir: &Path, i: usize) -> PathBuf {
    dir.join("out").join(format!("{i}-selphi.jpg"))
}

#[gpui_kit::test]
fn prepare_writes_every_photo(cx: &mut TestAppContext) {
    let (dir, photos, config) = fixture("gui-flow-prepare", 2);
    let (handle, view, _) = open(config, cx);
    add(handle, &view, &photos, cx);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("prepare", cx);
    })
    .unwrap();
    cx.run_until_parked();

    let statuses = statuses(&view, cx);
    assert!(
        statuses
            .iter()
            .all(|s| matches!(s, Status::Prepared { .. })),
        "{statuses:?}"
    );
    assert!(output(&dir, 0).exists() && output(&dir, 1).exists());
    cx.update(|cx| assert!(!view.read(cx).is_running()));
}

#[gpui_kit::test]
fn cancel_leaves_the_later_photos_waiting(cx: &mut TestAppContext) {
    let (dir, photos, config) = fixture("gui-flow-cancel", 3);
    let (handle, view, _) = open(config, cx);
    cx.update(|cx| view.update(cx, |view, _| view.set_worker_limit(1)));
    add(handle, &view, &photos, cx);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("prepare", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();

    let statuses = statuses(&view, cx);
    assert!(
        matches!(statuses[0], Status::Prepared { .. }),
        "{statuses:?}"
    );
    assert_eq!(statuses[1..], [Status::Waiting, Status::Waiting]);
    assert!(output(&dir, 0).exists());
    assert!(!output(&dir, 1).exists());
}

#[gpui_kit::test]
fn closing_settings_does_not_quit(cx: &mut TestAppContext) {
    let (_, _, config) = fixture("gui-flow-settings", 0);
    let (handle, _, quits) = open(config, cx);
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(OpenSettings), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let settings = cx.update(|cx| cx.windows().into_iter().find(|w| *w != handle));
    let settings = settings.expect("Settings… opens a window");

    settings
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(quits.get(), 0);

    handle
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(quits.get(), 1, "closing the main window quits");
}
