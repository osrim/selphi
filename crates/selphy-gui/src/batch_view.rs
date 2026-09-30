//! The window: the photos to prepare, the folder they go to, and the run.

use std::path::{Path, PathBuf};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::{DialogAction, DialogClose, DialogFooter},
    h_flex,
    notification::Notification,
    scroll::ScrollableElement as _,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ExternalPaths, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Render, SharedString, Styled as _,
    Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
};

use selphy::config::ConfigFile;
use selphy::prepare::{self, Job, Options};
use selphy::report::{error_chain, sentence};
use selphy::toml_file;

use crate::appearance::{self, Appearance, ThemeChoice};
use crate::batch::{Batch, Photo, Status};
use crate::config_panel::ConfigPanel;
use crate::{AddPhotos, CancelPrepare, ChooseFolder, OpenConfig, Prepare};

/// How the last run ended, shown beside the Prepare button.
struct Summary {
    prepared: usize,
    failed: usize,
    cancelled: bool,
}

pub struct BatchView {
    batch: Batch,
    out_dir: Option<PathBuf>,
    summary: Option<Summary>,
    /// The printer config file, found once when the window opened.
    config: ConfigFile,
    theme: ThemeChoice,
    /// The run in progress. Dropping it stops the run after the current photo.
    run: Option<Task<()>>,
    /// Cancel was pressed: the run stops after the photo in progress.
    cancelling: bool,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl BatchView {
    pub fn new(config: ConfigFile, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let gui_path = config.sibling(appearance::FILE_NAME);
        let (theme, load_error) = match toml_file::load_or_default::<Appearance>(&gui_path) {
            Ok(appearance) => (appearance.theme, None),
            Err(err) => (ThemeChoice::default(), Some(error_sentence(&err))),
        };
        theme.apply(window, cx);
        if let Some(message) = load_error {
            // Root, which shows notifications, exists once the window is built.
            cx.defer_in(window, move |_, window, cx| {
                show_error("Couldn't read the window settings", message, window, cx)
            });
        }
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme == ThemeChoice::System {
                this.theme.apply(window, cx);
            }
        });
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self {
            batch: Batch::default(),
            out_dir: default_out_dir(),
            summary: None,
            config,
            theme,
            run: None,
            cancelling: false,
            focus_handle,
            _subscriptions: vec![appearance],
        }
    }

    fn is_running(&self) -> bool {
        self.run.is_some()
    }

    fn is_ready(&self) -> bool {
        !self.is_running() && !self.batch.is_empty() && self.out_dir.is_some()
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
    fn add_paths(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
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
                self.batch.add(files);
                self.summary = None;
            }
            Err(err) => show_error("Couldn't add photos", error_sentence(&err), window, cx),
        }
        cx.notify();
    }

    fn choose_folder(&mut self, _: &ChooseFolder, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| match picked.await {
            Ok(Ok(Some(mut paths))) => {
                if let Some(dir) = paths.pop() {
                    this.update(cx, |this, cx| {
                        this.out_dir = Some(dir);
                        cx.notify();
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

    fn open_config(&mut self, _: &OpenConfig, window: &mut Window, cx: &mut Context<Self>) {
        let panel = cx.new(|cx| ConfigPanel::new(self.theme, self.config.clone(), window, cx));
        let view = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let (panel_on_ok, view) = (panel.clone(), view.clone());
            dialog
                .title(div().text_lg().child("Config"))
                // Wide enough for the four trims on one row.
                .w(px(680.))
                .child(panel.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("restore-config")
                                .ghost()
                                .label("Restore defaults")
                                .on_click({
                                    let panel = panel.clone();
                                    move |_, window, cx| {
                                        panel.update(cx, |panel, cx| {
                                            panel.restore_defaults(window, cx)
                                        })
                                    }
                                }),
                        )
                        .child(
                            h_flex()
                                .flex_1()
                                .gap_2()
                                .child(
                                    DialogClose::new()
                                        .child(Button::new("cancel-config").label("Cancel")),
                                )
                                .child(
                                    DialogAction::new()
                                        .child(Button::new("save-config").primary().label("Save")),
                                ),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let Some(theme) = panel_on_ok.update(cx, |panel, cx| panel.save(cx)) else {
                        return false;
                    };
                    view.update(cx, |view, _| view.theme = theme).ok();
                    theme.apply(window, cx);
                    true
                })
        });
    }

    /// Prepares every photo in the batch, one at a time on a background
    /// thread, reading the config file afresh as `selphy prepare` does.
    fn prepare(&mut self, _: &Prepare, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_ready() {
            return;
        }
        let Some(job) = self.job(window, cx) else {
            return;
        };
        let queue = self.batch.start();
        self.summary = None;
        self.cancelling = false;
        self.run = Some(cx.spawn(async move |this, cx| {
            for (id, source) in queue {
                let go_on = this.update(cx, |this, cx| {
                    if this.cancelling {
                        return false;
                    }
                    this.batch.set_status(id, Status::Preparing);
                    cx.notify();
                    true
                });
                match go_on {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(_) => return,
                }
                let job = job.clone();
                let status = cx
                    .background_spawn(async move {
                        let result = job.run_one(&source);
                        Status::from_result(&result, &source)
                    })
                    .await;
                this.update(cx, |this, cx| {
                    this.batch.set_status(id, status);
                    cx.notify();
                })
                .ok();
            }
            this.update(cx, |this, cx| this.finish(cx)).ok();
        }));
        cx.notify();
    }

    /// The job for a run into the chosen folder, for the config's paper,
    /// with the config file read afresh and the env overrides applied, as
    /// `selphy prepare` does. Sources stay in place and no camera reference
    /// is used, as the README says. Shows why and returns `None` when it
    /// cannot be built.
    fn job(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Job> {
        let out_dir = self.out_dir.clone()?;
        let loaded = self
            .config
            .load(None)
            .and_then(|loaded| Ok((loaded.paper, loaded.profile()?)));
        let (paper, profile) = match loaded {
            Ok(loaded) => loaded,
            Err(err) => {
                let message = format!("{} Fix it in Config.", error_sentence(&err));
                show_error(
                    "Couldn't read the printer config",
                    message.into(),
                    window,
                    cx,
                );
                return None;
            }
        };
        let opts = Options {
            out_dir,
            archive_dir: None,
            camera_ref: None,
        };
        Job::new(paper, profile, opts)
            .inspect_err(|err| {
                show_error("Couldn't start preparing", error_sentence(err), window, cx)
            })
            .ok()
    }

    fn cancel_prepare(&mut self, _: &CancelPrepare, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_running() && !self.cancelling {
            self.cancelling = true;
            cx.notify();
        }
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        let (prepared, failed) = self.batch.counts();
        self.summary = Some(Summary {
            prepared,
            failed,
            cancelled: self.cancelling,
        });
        self.run = None;
        self.cancelling = false;
        cx.notify();
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = match self.batch.len() {
            0 => String::new(),
            n => n.to_string(),
        };
        h_flex()
            .gap_2()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().font_medium().child("Photos"))
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(count),
            )
            .child(
                Button::new("clear")
                    .ghost()
                    .small()
                    .label("Clear")
                    .disabled(self.batch.is_empty() || self.is_running())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.batch.clear();
                        this.summary = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("add")
                    .small()
                    .icon(IconName::Plus)
                    .label("Add…")
                    .disabled(self.is_running())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.add_photos(&AddPhotos, window, cx)),
                    ),
            )
    }

    fn render_photos(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyElement = if self.batch.is_empty() {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::FolderOpen).large())
                .child(div().text_sm().child("Drop photos or folders here"))
                .child(Button::new("add-empty").label("Add photos…").on_click(
                    cx.listener(|this, _, window, cx| this.add_photos(&AddPhotos, window, cx)),
                ))
                .into_any_element()
        } else {
            let mut rows = Vec::with_capacity(self.batch.len());
            for photo in self.batch.photos() {
                rows.push(self.render_photo(photo, cx));
            }
            v_flex()
                .id("photo-list")
                .size_full()
                .overflow_y_scrollbar()
                .children(rows)
                .into_any_element()
        };
        div()
            .id("photos")
            .flex_1()
            .min_h_0()
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().muted))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.add_paths(paths.paths(), window, cx)
            }))
            .child(content)
    }

    fn render_photo(&self, photo: &Photo, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (icon, detail, detail_color): (AnyElement, String, _) = match photo.status() {
            Status::Waiting => (
                Icon::new(IconName::File)
                    .small()
                    .text_color(theme.muted_foreground)
                    .into_any_element(),
                photo
                    .source()
                    .parent()
                    .map(display_path)
                    .unwrap_or_default(),
                theme.muted_foreground,
            ),
            Status::Preparing => (
                Spinner::new().small().into_any_element(),
                "Preparing".to_string(),
                theme.muted_foreground,
            ),
            Status::Prepared(summary) => (
                Icon::new(IconName::CircleCheck)
                    .small()
                    .text_color(theme.success)
                    .into_any_element(),
                summary.clone(),
                theme.muted_foreground,
            ),
            Status::Failed(reason) => (
                Icon::new(IconName::CircleX)
                    .small()
                    .text_color(theme.danger)
                    .into_any_element(),
                reason.clone(),
                theme.danger,
            ),
        };
        let name = photo
            .source()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let id = photo.id();
        // A failure may need two lines; a folder or result stays on one.
        let wraps = matches!(photo.status(), Status::Failed(_));
        h_flex()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(div().flex().size_5().justify_center().child(icon))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().truncate().child(name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(detail_color)
                            .when(wraps, |this| this.line_clamp(2))
                            .when(!wraps, |this| this.truncate())
                            .child(detail),
                    ),
            )
            .child(
                Button::new(("remove", id.key()))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip("Remove")
                    .disabled(self.is_running())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.batch.remove(id);
                        cx.notify();
                    })),
            )
    }

    fn render_destination(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let folder = match &self.out_dir {
            Some(dir) => div().child(display_path(dir)),
            None => div()
                .text_color(cx.theme().muted_foreground)
                .child("No folder chosen"),
        };
        h_flex()
            .gap_3()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .text_sm()
            .child(div().font_medium().child("Save to"))
            .child(folder.flex_1().min_w_0().truncate())
            .child(Button::new("choose").small().label("Choose…").on_click(
                cx.listener(|this, _, window, cx| this.choose_folder(&ChooseFolder, window, cx)),
            ))
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status = match &self.summary {
            Some(summary) => summary_text(summary),
            None if !self.batch.is_empty() && self.out_dir.is_none() => {
                "Choose a folder to save to.".to_string()
            }
            None => String::new(),
        };
        let open_folder = match (&self.summary, &self.out_dir) {
            (Some(summary), Some(dir)) if summary.prepared > 0 => Some(dir.clone()),
            _ => None,
        };
        let run_button = if self.is_running() {
            Button::new("cancel-prepare")
                .label("Cancel")
                .loading(self.cancelling)
                .disabled(self.cancelling)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.cancel_prepare(&CancelPrepare, window, cx)
                }))
        } else {
            Button::new("prepare")
                .primary()
                .label("Prepare")
                .disabled(!self.is_ready())
                .on_click(cx.listener(|this, _, window, cx| this.prepare(&Prepare, window, cx)))
        };
        h_flex()
            .gap_2()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(theme.border)
            .child(
                Button::new("config")
                    .outline()
                    .icon(IconName::Settings)
                    .accessibility_label("Config…")
                    .tooltip_with_action("Config", &OpenConfig, Some("Batch"))
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.open_config(&OpenConfig, window, cx)
                        }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(status),
            )
            .when_some(open_folder, |this, dir| {
                this.child(
                    Button::new("open-folder")
                        .ghost()
                        .icon(IconName::FolderOpen)
                        .label("Open folder")
                        .on_click(move |_, _, cx| cx.open_with_system(&dir)),
                )
            })
            .child(run_button)
    }
}

impl Render for BatchView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("batch")
            .key_context("Batch")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::add_photos))
            .on_action(cx.listener(Self::choose_folder))
            .on_action(cx.listener(Self::open_config))
            .on_action(cx.listener(Self::prepare))
            .on_action(cx.listener(Self::cancel_prepare))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_header(cx))
            .child(self.render_photos(cx))
            .child(self.render_destination(cx))
            .child(self.render_footer(cx))
    }
}

/// "3 prepared", "2 prepared, 1 failed", "Cancelled after 2 prepared", or
/// "Cancelled".
fn summary_text(summary: &Summary) -> String {
    let counts = match (summary.prepared, summary.failed) {
        (0, 0) => String::new(),
        (p, 0) => format!("{p} prepared"),
        (0, f) => format!("{f} failed"),
        (p, f) => format!("{p} prepared, {f} failed"),
    };
    match (summary.cancelled, counts.is_empty()) {
        (true, true) => "Cancelled".to_string(),
        (true, false) => format!("Cancelled after {counts}"),
        (false, _) => counts,
    }
}

/// `~/Pictures` on macOS; elsewhere there is no default.
fn default_out_dir() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Pictures"))
}

/// Shows an error as a notification that stays until it is closed.
fn show_error(title: &str, message: SharedString, window: &mut Window, cx: &mut App) {
    window.push_notification(
        Notification::error(message)
            .title(title.to_string())
            .autohide(false),
        cx,
    );
}

/// An error that is not about one photo, as a sentence for a notice.
pub fn error_sentence(err: &anyhow::Error) -> SharedString {
    sentence(&error_chain(err)).into()
}

/// The path with the home folder shown as `~`.
fn display_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) => Path::new("~").join(rest).display().to_string(),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(prepared: usize, failed: usize, cancelled: bool) -> String {
        summary_text(&Summary {
            prepared,
            failed,
            cancelled,
        })
    }

    #[test]
    fn summary_text_leaves_out_zero_counts() {
        assert_eq!(summary(3, 0, false), "3 prepared");
        assert_eq!(summary(0, 2, false), "2 failed");
        assert_eq!(summary(2, 1, false), "2 prepared, 1 failed");
    }

    #[test]
    fn summary_text_says_when_the_run_was_cancelled() {
        assert_eq!(summary(0, 0, true), "Cancelled");
        assert_eq!(summary(2, 1, true), "Cancelled after 2 prepared, 1 failed");
    }
}
