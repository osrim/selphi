//! The main window: a toolbar with Add, Clear, Fit and the output folder;
//! the photo list next to the card preview; and a footer with the progress
//! and Prepare. It owns the batch, the run and the caches.

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonGroup, ButtonVariants as _},
    h_flex, h_resizable,
    notification::Notification,
    progress::Progress,
    resizable_panel, v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, UniformListScrollHandle,
    Window, div, prelude::FluentBuilder as _, px,
};
use selphy::config::Profile;
use selphy::geometry::Fit;
use selphy::prepare::{self, Job, Options, Planned};

use crate::batch::{Batch, PhotoId, Status};
use crate::preview::{CardPreview, PreviewKey, Setup};
use crate::run::{self, Run, Summary};
use crate::state::{Prefs, ThemeChoice};
use crate::text::{display_path, error_sentence, summary_text};
use crate::thumbnails::Thumbnails;
use crate::{
    AddPhotos, CancelPrepare, ChooseFolder, ClearPhotos, OpenSettings, Prepare, Redo,
    RemoveSelected, SelectNext, SelectPrevious, Undo,
};

/// What the window prepares with: the fit, and the profile from the
/// printer config.
struct PrinterSetup {
    /// Goes up when `source` changes, so that a preview of an older setup
    /// is dropped.
    revision: u64,
    setup: Setup,
    /// The settings revision and the profile the setup was made from. The
    /// profile also changes when the file or an override changes on disk.
    source: (u64, Option<Profile>),
}

pub struct MainWindow {
    prefs: Entity<Prefs>,
    pub(crate) batch: Batch,
    run: Option<Run>,
    /// The workers of the run. Dropping them stops the run after the photos
    /// in progress.
    workers: Vec<Task<()>>,
    worker_limit: usize,
    summary: Option<Summary>,
    printer: PrinterSetup,
    pub(crate) thumbnails: Entity<Thumbnails>,
    preview: Entity<CardPreview>,
    pub(crate) list_scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl MainWindow {
    pub fn new(prefs: Entity<Prefs>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = cx.observe_window_appearance(window, |this, _, cx| {
            if this.prefs.read(cx).settings().theme == ThemeChoice::System {
                ThemeChoice::System.apply(cx);
            }
        });
        let prefs_changed = cx.observe(&prefs, |this, _, cx| this.prefs_changed(cx));
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let printer = load_setup(prefs.read(cx));
        let mut this = Self {
            prefs,
            batch: Batch::default(),
            run: None,
            workers: Vec::new(),
            worker_limit: run::default_workers(),
            summary: None,
            printer,
            thumbnails: cx.new(|_| Thumbnails::default()),
            preview: cx.new(|_| CardPreview::new()),
            list_scroll: UniformListScrollHandle::new(),
            focus_handle,
            _subscriptions: vec![appearance, prefs_changed],
        };
        // Says at once when the config is broken, before any photo is added.
        this.refresh_preview(cx);
        this
    }

    /// Sets how many photos a run prepares at once.
    #[cfg(test)]
    pub fn set_worker_limit(&mut self, limit: usize) {
        self.worker_limit = limit.max(1);
    }

    pub(crate) fn is_running(&self) -> bool {
        self.run.is_some()
    }

    fn can_prepare(&self) -> bool {
        !self.is_running()
            && !self.batch.is_empty()
            && matches!(self.printer.setup, Setup::Ready(_))
    }

    fn prefs_changed(&mut self, cx: &mut Context<Self>) {
        self.reload_setup(cx);
        cx.notify();
    }

    /// Reads the printer config afresh, as `selphy prepare` does, and
    /// refreshes the preview when the setup changed.
    fn reload_setup(&mut self, cx: &mut Context<Self>) {
        let mut next = load_setup(self.prefs.read(cx));
        next.revision = self.printer.revision + u64::from(next.source != self.printer.source);
        self.printer = next;
        self.refresh_preview(cx);
    }

    /// Call after the list or the selection changed: loads the thumbnails
    /// and the preview they need.
    pub(crate) fn list_changed(&mut self, cx: &mut Context<Self>) {
        let photos = self.batch.queue();
        self.thumbnails
            .update(cx, |thumbnails, cx| thumbnails.request(photos, cx));
        if let Some(ix) = self
            .batch
            .selected_id()
            .and_then(|id| self.batch.photos().iter().position(|p| p.id() == id))
        {
            self.list_scroll
                .scroll_to_item(ix, gpui_kit::ScrollStrategy::Nearest);
        }
        self.refresh_preview(cx);
        cx.notify();
    }

    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        let revision = self.printer.revision;
        let photo = self.batch.selected().map(|photo| {
            let key = PreviewKey {
                photo: photo.id(),
                revision,
            };
            (key, photo.source().to_path_buf())
        });
        let setup = self.printer.setup.clone();
        self.preview
            .update(cx, |preview, cx| preview.show(photo, &setup, cx));
    }

    fn add_photos(&mut self, _: &AddPhotos, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_running() {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Add".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Err(err)) => {
                    let message = error_sentence(&err);
                    cx.update(|window, cx| {
                        show_error("Couldn't open the file picker", message, window, cx)
                    })
                    .ok();
                    return;
                }
                _ => return,
            };
            this.update_in(cx, |this, window, cx| this.add_paths(&paths, window, cx))
                .ok();
        })
        .detach();
    }

    /// Adds the named files, and the photos directly inside the named folders.
    pub fn add_paths(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        if self.is_running() {
            return;
        }
        match prepare::collect_inputs(paths) {
            Ok(files) if files.is_empty() => {
                window.push_notification(
                    Notification::warning("The folder has no JPEG, PNG or TIFF files.")
                        .title("No photos added"),
                    cx,
                );
            }
            Ok(files) => {
                if self.batch.add(files) > 0 {
                    self.summary = None;
                }
            }
            Err(err) => show_error("Couldn't add photos", error_sentence(&err), window, cx),
        }
        self.list_changed(cx);
    }

    fn choose_folder(&mut self, _: &ChooseFolder, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_running() {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| match picked.await {
            Ok(Ok(Some(mut paths))) => {
                if let Some(dir) = paths.pop() {
                    this.update_in(cx, |this, window, cx| {
                        // The last run's prints are not in the new folder.
                        this.summary = None;
                        this.change_settings(|s| s.out_dir = dir, window, cx)
                    })
                    .ok();
                }
            }
            Ok(Err(err)) => {
                let message = error_sentence(&err);
                cx.update(|window, cx| {
                    show_error("Couldn't open the folder picker", message, window, cx)
                })
                .ok();
            }
            _ => {}
        })
        .detach();
    }

    /// Changes the window settings and saves them. The change applies even
    /// when the file cannot be written; the notice says so.
    fn change_settings(
        &mut self,
        change: impl FnOnce(&mut crate::state::GuiSettings),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self.prefs.update(cx, |prefs, cx| {
            let result = prefs.change(change);
            cx.notify();
            result.map_err(|err| (prefs.path(), err))
        });
        if let Err((path, err)) = result {
            let message = format!("{} Not saved to {}.", error_sentence(&err), path.display());
            show_error(
                "Couldn't save the window settings",
                message.into(),
                window,
                cx,
            );
        }
    }

    fn remove_selected(&mut self, _: &RemoveSelected, _: &mut Window, cx: &mut Context<Self>) {
        self.batch.remove_selected();
        self.list_changed(cx);
    }

    pub(crate) fn remove(&mut self, id: PhotoId, cx: &mut Context<Self>) {
        self.batch.remove(id);
        self.list_changed(cx);
    }

    fn clear(&mut self, _: &ClearPhotos, _: &mut Window, cx: &mut Context<Self>) {
        self.batch.clear();
        self.summary = None;
        self.list_changed(cx);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.batch.undo() {
            self.list_changed(cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.batch.redo() {
            self.list_changed(cx);
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.batch.move_selection(1);
        self.list_changed(cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.batch.move_selection(-1);
        self.list_changed(cx);
    }

    pub(crate) fn select(&mut self, id: PhotoId, cx: &mut Context<Self>) {
        self.batch.select(id);
        self.list_changed(cx);
    }

    /// Prepares every photo, `worker_limit` at a time, with the printer
    /// config read afresh as `selphy prepare` does. Two photos that would
    /// write one output are an error before any is prepared.
    fn prepare(&mut self, _: &Prepare, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_running() || self.batch.is_empty() {
            return;
        }
        self.reload_setup(cx);
        let job = match &self.printer.setup {
            Setup::Ready(job) => job.clone(),
            Setup::Broken(message) => {
                let message = format!("{message} Fix it in Settings.");
                show_error(
                    "Couldn't read the printer config",
                    message.into(),
                    window,
                    cx,
                );
                return cx.notify();
            }
        };
        let mut run = match Run::plan(self.batch.queue(), &job.options().out_dir) {
            Ok(run) => run,
            Err(err) => {
                show_error("Couldn't start preparing", error_sentence(&err), window, cx);
                return;
            }
        };
        self.batch.start();
        self.summary = None;
        // The first photos are handed out now, so that their rows show
        // Preparing at once and a Cancel right after Prepare still lets them
        // finish.
        let mut first = Vec::new();
        while first.len() < self.worker_limit {
            let Some((id, planned)) = run.take() else {
                break;
            };
            self.batch.set_status(id, Status::Preparing);
            first.push((id, planned));
        }
        self.run = Some(run);
        self.workers = first
            .into_iter()
            .map(|item| self.spawn_worker(item, job.clone(), cx))
            .collect();
        cx.notify();
    }

    /// A worker: prepares `first`, then each next photo of the run, until
    /// there is none.
    fn spawn_worker(
        &self,
        first: (PhotoId, Planned),
        job: Job,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut next = Some(first);
            while let Some((id, planned)) = next.take() {
                let job = job.clone();
                let status = cx
                    .background_spawn(async move {
                        let result = job.run_one(&planned);
                        Status::from_result(&result, &planned.source)
                    })
                    .await;
                match this.update(cx, |this, cx| this.photo_done(id, status, cx)) {
                    Ok(item) => next = item,
                    Err(_) => return,
                }
            }
        })
    }

    /// Records a finished photo, and hands out the next one.
    fn photo_done(
        &mut self,
        id: PhotoId,
        status: Status,
        cx: &mut Context<Self>,
    ) -> Option<(PhotoId, Planned)> {
        let run = self.run.as_mut()?;
        run.record(matches!(status, Status::Prepared { .. }));
        self.batch.set_status(id, status);
        let next = run.take();
        if let Some((next_id, _)) = &next {
            self.batch.set_status(*next_id, Status::Preparing);
        }
        if run.is_done() {
            self.summary = Some(run.summary());
            self.run = None;
            self.workers.clear();
            self.batch.finish();
        }
        cx.notify();
        next
    }

    fn cancel_prepare(&mut self, _: &CancelPrepare, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(run) = &mut self.run
            && !run.is_cancelled()
        {
            run.cancel();
            cx.notify();
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.prefs.read(cx).settings();
        let fit = settings.fit;
        let out_dir = display_path(&settings.out_dir);
        let running = self.is_running();
        let fits = ButtonGroup::new("fit")
            .small()
            .outline()
            .disabled(running)
            .children(Fit::ALL.map(|choice| {
                Button::new(choice.name())
                    .label(choice.label())
                    .selected(choice == fit)
            }))
            .on_click(cx.listener(|this, clicked: &Vec<usize>, window, cx| {
                if let Some(&ix) = clicked.first() {
                    this.change_settings(|s| s.fit = Fit::ALL[ix], window, cx);
                }
            }));
        let muted = cx.theme().muted_foreground;
        h_flex()
            .gap_4()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("add")
                            .small()
                            .icon(IconName::Plus)
                            .label("Add…")
                            .disabled(running)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_photos(&AddPhotos, window, cx)
                            })),
                    )
                    .child(
                        // No confirmation: Undo brings the photos back.
                        Button::new("clear")
                            .small()
                            .ghost()
                            .label("Clear")
                            .disabled(running || self.batch.is_empty())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.clear(&ClearPhotos, window, cx)
                            })),
                    ),
            )
            .child(fits)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .justify_end()
                    .gap_2()
                    .text_sm()
                    .child(div().text_color(muted).flex_none().child("Save to"))
                    .child(div().min_w_0().truncate().child(out_dir))
                    .child(
                        Button::new("choose")
                            .small()
                            .label("Choose…")
                            .disabled(running)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_folder(&ChooseFolder, window, cx)
                            })),
                    ),
            )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status: SharedString = match (&self.run, &self.summary) {
            (Some(run), _) => {
                let (done, total) = run.progress();
                if run.is_cancelled() {
                    format!("Cancelling… {done} of {total}").into()
                } else {
                    format!("{done} of {total}").into()
                }
            }
            (None, Some(summary)) => summary_text(summary).into(),
            _ => SharedString::default(),
        };
        let progress = self.run.as_ref().map(|run| {
            let (done, total) = run.progress();
            done as f32 / total.max(1) as f32 * 100.
        });
        let show_folder = match &self.summary {
            Some(summary) if summary.prepared > 0 && !self.is_running() => {
                Some(self.prefs.read(cx).settings().out_dir.clone())
            }
            _ => None,
        };
        let run_button = match &self.run {
            Some(run) => Button::new("cancel-prepare")
                .label("Cancel")
                .loading(run.is_cancelled())
                .disabled(run.is_cancelled())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.cancel_prepare(&CancelPrepare, window, cx)
                })),
            None => Button::new("prepare")
                .primary()
                .label("Prepare")
                .disabled(!self.can_prepare())
                .on_click(cx.listener(|this, _, window, cx| this.prepare(&Prepare, window, cx))),
        };
        h_flex()
            .gap_3()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(theme.border)
            .child(
                Button::new("settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .accessibility_label("Settings")
                    .tooltip_with_action("Settings", &OpenSettings, None)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
            )
            .when_some(progress, |this, value| {
                this.child(
                    div().w(px(160.)).child(
                        Progress::new("run-progress")
                            .value(value)
                            .accessibility_label(status.clone()),
                    ),
                )
            })
            .child(
                div()
                    .id("run-status")
                    // Read out as it changes, as the run goes.
                    .role(gpui_kit::Role::Status)
                    .aria_label(status.clone())
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(status),
            )
            .when_some(show_folder, |this, dir| {
                this.child(
                    Button::new("show-output")
                        .ghost()
                        .label("Show in Finder")
                        .on_click(move |_, _, cx| cx.open_with_system(&dir)),
                )
            })
            .child(run_button)
    }
}

impl Focusable for MainWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("main-window")
            .key_context("MainWindow")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::add_photos))
            .on_action(cx.listener(Self::choose_folder))
            .on_action(cx.listener(Self::prepare))
            .on_action(cx.listener(Self::cancel_prepare))
            .when(!self.is_running(), |this| {
                this.on_action(cx.listener(Self::remove_selected))
                    .on_action(cx.listener(Self::clear))
                    .when(self.batch.can_undo(), |this| {
                        this.on_action(cx.listener(Self::undo))
                    })
                    .when(self.batch.can_redo(), |this| {
                        this.on_action(cx.listener(Self::redo))
                    })
            })
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_toolbar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("split")
                        .child(
                            resizable_panel()
                                .size(px(320.))
                                .size_range(px(240.)..px(560.))
                                .child(self.render_photo_list(window, cx)),
                        )
                        .child(
                            resizable_panel()
                                .size_range(px(320.)..gpui_kit::Pixels::MAX)
                                .child(
                                    div()
                                        .size_full()
                                        .p_6()
                                        .border_l_1()
                                        .border_color(cx.theme().border)
                                        .child(self.preview.clone()),
                                ),
                        ),
                ),
            )
            .child(self.render_footer(cx))
    }
}

/// The setup for the settings' fit, from the printer config as it is on
/// disk now, with the env overrides. Its revision is 0; the main
/// window numbers the setups it loads.
fn load_setup(prefs: &Prefs) -> PrinterSetup {
    let settings = prefs.settings();
    let revision = 0;
    let loaded = match prefs.config().load(Some(settings.fit)) {
        Ok(loaded) => loaded,
        Err(err) => {
            return PrinterSetup {
                revision,
                setup: Setup::Broken(error_sentence(&err)),
                source: (prefs.revision(), None),
            };
        }
    };
    let opts = Options {
        out_dir: settings.out_dir.clone(),
        archive_dir: None,
        camera_ref: None,
        fit: settings.fit,
    };
    let setup = match loaded.profile().and_then(|profile| Job::new(profile, opts)) {
        Ok(job) => Setup::Ready(job),
        Err(err) => Setup::Broken(error_sentence(&err)),
    };
    let profile = loaded.profile().ok();
    PrinterSetup {
        revision,
        setup,
        source: (prefs.revision(), profile),
    }
}

/// Shows an error as a notification that stays until it is closed.
pub(crate) fn show_error(title: &str, message: SharedString, window: &mut Window, cx: &mut App) {
    window.push_notification(
        Notification::error(message)
            .title(title.to_string())
            .autohide(false),
        cx,
    );
}
