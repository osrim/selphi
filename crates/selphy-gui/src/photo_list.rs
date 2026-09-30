//! The photo list pane of the main window: a row per photo with its
//! thumbnail, name and status; the drop target for photos and folders; the
//! row context menu; and the empty state.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, Theme,
    button::Button,
    h_flex,
    menu::{ContextMenuExt as _, PopupMenuItem},
    spinner::Spinner,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::{
    AnyElement, Context, ExternalPaths, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    ObjectFit, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, Window, div, img, prelude::FluentBuilder as _, uniform_list,
};

use crate::AddPhotos;
use crate::batch::{Photo, Status};
use crate::main_window::MainWindow;
use crate::text::file_name;

impl MainWindow {
    pub(crate) fn render_photo_list(
        &self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let content: AnyElement = if self.batch.is_empty() {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::FolderOpen).large())
                .child(div().text_sm().child("Drop photos or folders here"))
                .child(
                    Button::new("add-empty")
                        .label("Add…")
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(AddPhotos), cx)),
                )
                .into_any_element()
        } else {
            uniform_list(
                "photo-list",
                self.batch.len(),
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    this.batch.photos()[range]
                        .iter()
                        .map(|photo| this.render_row(photo, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.list_scroll)
            .size_full()
            .into_any_element()
        };
        div()
            .id("photos")
            .size_full()
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().muted))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.add_paths(paths.paths(), window, cx)
            }))
            .child(content)
    }

    fn render_row(&self, photo: &Photo, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let id = photo.id();
        let selected = self.batch.selected_id() == Some(id);
        let (icon, detail, detail_color) = status_parts(photo, cx.theme());
        let thumbnail = match self.thumbnails.read(cx).get(id) {
            Some(image) => img(image)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => Icon::new(IconName::File)
                .text_color(theme.muted_foreground)
                .into_any_element(),
        };
        let shown = photo.shown_file().to_path_buf();
        let view = cx.weak_entity();
        let running = self.is_running();
        let tooltip = match photo.status() {
            Status::Failed(reason) => Some(SharedString::from(reason.clone())),
            _ => None,
        };
        h_flex()
            .id(("photo", id.key()))
            .w_full()
            .gap_3()
            .px_3()
            .h_16()
            .when(selected, |this| this.bg(theme.list_active))
            .when(!selected, |this| {
                this.hover(|style| style.bg(theme.list_hover))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| this.select(id, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| this.select(id, cx)),
            )
            .when_some(tooltip, |this, reason| {
                this.tooltip(move |window, cx| Tooltip::new(reason.clone()).build(window, cx))
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .size_12()
                    .items_center()
                    .justify_center()
                    .child(thumbnail),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().truncate().child(file_name(photo.source())))
                    .child(
                        div()
                            .text_xs()
                            .text_color(detail_color)
                            .truncate()
                            .child(detail),
                    ),
            )
            // The slot is kept when empty, so that every row lines up.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .size_5()
                    .items_center()
                    .justify_center()
                    .children(icon),
            )
            .context_menu(move |menu, _, _| {
                let shown = shown.clone();
                let view = view.clone();
                menu.item(
                    PopupMenuItem::new("Show in Finder")
                        .on_click(move |_, _, cx| cx.reveal_path(&shown)),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Remove")
                        .disabled(running)
                        .on_click(move |_, _, cx| {
                            view.update(cx, |this, cx| this.remove(id, cx)).ok();
                        }),
                )
            })
            .into_any_element()
    }
}

/// The status icon of a row, its one-line status, and the status's colour.
/// A failure's full reason is in the row's tooltip; the placement is under
/// the preview.
fn status_parts(photo: &Photo, theme: &Theme) -> (Option<AnyElement>, &'static str, Hsla) {
    let icon = |name: IconName, color: Hsla| {
        Some(Icon::new(name).small().text_color(color).into_any_element())
    };
    match photo.status() {
        Status::Waiting => (None, "Not prepared", theme.muted_foreground),
        Status::Preparing => (
            Some(Spinner::new().small().into_any_element()),
            "Preparing…",
            theme.muted_foreground,
        ),
        Status::Prepared { .. } => (
            icon(IconName::CircleCheck, theme.success),
            "Prepared",
            theme.muted_foreground,
        ),
        Status::Failed(_) => (
            icon(IconName::CircleX, theme.danger),
            "Failed",
            theme.danger,
        ),
    }
}
