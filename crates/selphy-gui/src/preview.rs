//! The card preview: the selected photo rendered by `Job::preview`, the
//! same code as Prepare: the card inside a dashed line at the paper's edge,
//! the rest of the picture dimmed around it, and the placement under it. A render made for another photo or an older revision is
//! dropped when it arrives.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, button::Button,
    spinner::Spinner, v_flex,
};
use gpui_kit::{
    Action as _, App, AppContext as _, BorderStyle, Bounds, Context, Corners, Hsla, IntoElement,
    ParentElement as _, Pixels, Render, RenderImage, SharedString, Styled as _, Task, Window,
    canvas, div, fill, outline, point, px,
};
use selphy::geometry::{Edge, Placement};
use selphy::prepare::Job;

use crate::OpenSettings;
use crate::batch::PhotoId;
use crate::text::{error_sentence, file_name, placement_text};
use crate::thumbnails::render_image;

/// The longer side of the rendered preview, in pixels.
const PREVIEW_PX: u32 = 1200;

/// Which render the preview shows: a photo at a revision of the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewKey {
    pub photo: PhotoId,
    pub revision: u64,
}

/// What the preview can be made with.
#[derive(Debug, Clone)]
pub enum Setup {
    Ready(Job),
    /// The printer config cannot be read.
    Broken(SharedString),
}

struct Rendered {
    /// The canvas, as the output holds it.
    image: Arc<RenderImage>,
    /// The whole placed picture, at the canvas's scale.
    picture: Arc<RenderImage>,
    placement: Placement,
    caption: SharedString,
}

impl Rendered {
    fn drop_images(self, cx: &mut App) {
        cx.drop_image(self.image, None);
        cx.drop_image(self.picture, None);
    }
}

enum State {
    Empty,
    Broken(SharedString),
    Loading,
    Ready(Rendered),
    Failed(SharedString),
}

pub struct CardPreview {
    key: Option<PreviewKey>,
    state: State,
    _render: Option<Task<()>>,
}

impl CardPreview {
    pub fn new() -> Self {
        Self {
            key: None,
            state: State::Empty,
            _render: None,
        }
    }

    /// Shows `photo` as `setup` would print it, or why it cannot be shown.
    /// Does nothing when the same photo at the same revision is already
    /// shown or on its way.
    pub fn show(
        &mut self,
        photo: Option<(PreviewKey, PathBuf)>,
        setup: &Setup,
        cx: &mut Context<Self>,
    ) {
        let job = match setup {
            Setup::Ready(job) => job.clone(),
            // Said with or without a photo, so that the user knows first.
            Setup::Broken(message) => {
                self.key = None;
                return self.set_state(State::Broken(message.clone()), cx);
            }
        };
        let Some((key, source)) = photo else {
            self.key = None;
            return self.set_state(State::Empty, cx);
        };
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.set_state(State::Loading, cx);
        self._render = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let preview = job.preview(&source, PREVIEW_PX)?;
                    let caption = format!(
                        "{} · {} x {}",
                        file_name(&source),
                        preview.source_size.0,
                        preview.source_size.1
                    );
                    anyhow::Ok(Rendered {
                        image: render_image(&preview.image),
                        picture: render_image(&preview.picture),
                        placement: preview.placement,
                        caption: caption.into(),
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                if this.key != Some(key) {
                    if let Ok(rendered) = result {
                        rendered.drop_images(cx);
                    }
                    return;
                }
                let state = match result {
                    Ok(rendered) => State::Ready(rendered),
                    Err(err) => State::Failed(error_sentence(&err)),
                };
                this.set_state(state, cx);
            })
            .ok();
        }));
    }

    fn set_state(&mut self, state: State, cx: &mut Context<Self>) {
        if let State::Ready(old) = std::mem::replace(&mut self.state, state) {
            old.drop_images(cx);
        }
        cx.notify();
    }

    fn render_card(rendered: &Rendered, cx: &App) -> impl IntoElement {
        let images = (rendered.image.clone(), rendered.picture.clone());
        let placement = rendered.placement.clone();
        let colors = CardColors {
            dim: cx.theme().background.opacity(0.6),
            edge: cx.theme().foreground.opacity(0.7),
        };
        v_flex()
            .size_full()
            .min_h_0()
            .gap_3()
            .child(
                // The card, as it will print, inside a dashed line at the
                // paper's edge; around it, dimmed, the picture that the
                // printer cuts off.
                div().relative().flex_1().min_h_0().overflow_hidden().child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            let layout = card_layout(bounds, &placement);
                            paint_card(&layout, &images, colors, window);
                        },
                    )
                    .absolute()
                    .inset_0()
                    .size_full(),
                ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .text_sm()
                    .min_w_0()
                    .child(
                        div()
                            .font_medium()
                            .truncate()
                            .child(rendered.caption.clone()),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(placement_text(&rendered.placement)),
                    ),
            )
    }
}

impl Render for CardPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let centred = || {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .text_sm()
                .text_color(muted)
        };
        match &self.state {
            State::Empty => centred()
                .child("Select a photo to see its card.")
                .into_any_element(),
            State::Loading => centred().child(Spinner::new().large()).into_any_element(),
            State::Ready(rendered) => Self::render_card(rendered, cx).into_any_element(),
            State::Broken(message) => centred()
                .child(error_icon(cx))
                .child(div().text_center().child(message.clone()))
                .child(open_settings_button())
                .into_any_element(),
            State::Failed(message) => centred()
                .child(error_icon(cx))
                .child(div().text_center().child(message.clone()))
                .into_any_element(),
        }
    }
}

fn error_icon(cx: &App) -> Icon {
    Icon::new(IconName::CircleX)
        .large()
        .text_color(cx.theme().danger)
}

/// Opens Settings, where the config is fixed.
fn open_settings_button() -> Button {
    Button::new("open-settings")
        .label("Open Settings")
        .on_click(|_, window, cx| window.dispatch_action(OpenSettings.boxed_clone(), cx))
}

/// The colours of the preview, from the theme.
#[derive(Clone, Copy)]
struct CardColors {
    /// Over the picture that falls outside the card.
    dim: Hsla,
    /// The dashed line at the paper's edge.
    edge: Hsla,
}

/// Where the preview draws, in the pane.
#[derive(Debug, PartialEq)]
pub struct CardLayout {
    /// The canvas, trims included.
    pub canvas: Bounds<Pixels>,
    /// The card: the canvas's safe box.
    pub card: Bounds<Pixels>,
    /// The whole picture. For Fill card it reaches past the card, and may
    /// reach past the canvas.
    pub picture: Bounds<Pixels>,
}

impl CardLayout {
    /// The picture outside the card, left, top, right and bottom: what the
    /// printer cuts off. The top and bottom bands span the width, and the
    /// side bands only the card's height, so that no part is dimmed twice.
    /// Empty where the picture stays inside.
    pub fn cut(&self) -> Vec<Bounds<Pixels>> {
        let (picture, card) = (self.picture, self.card);
        let from = |x0, y0, x1, y1| Bounds::from_corners(point(x0, y0), point(x1, y1));
        [
            from(picture.left(), card.top(), card.left(), card.bottom()),
            from(picture.left(), picture.top(), picture.right(), card.top()),
            from(card.right(), card.top(), picture.right(), card.bottom()),
            from(
                picture.left(),
                card.bottom(),
                picture.right(),
                picture.bottom(),
            ),
        ]
        .into_iter()
        .map(|band| band.intersect(&picture))
        .filter(|band| band.size.width > Pixels::ZERO && band.size.height > Pixels::ZERO)
        .collect()
    }
}

/// Lays out `placement` in `pane`: the card and the whole picture together,
/// centred and as large as fits.
pub fn card_layout(pane: Bounds<Pixels>, placement: &Placement) -> CardLayout {
    let canvas = &placement.canvas;
    let (left, top) = (canvas.trim(Edge::Left), canvas.trim(Edge::Top));
    let card = (
        left,
        top,
        left + canvas.safe_width(),
        top + canvas.safe_height(),
    );
    let picture = (
        placement.x,
        placement.y,
        placement.x + placement.width,
        placement.y + placement.height,
    );
    // What is shown, in canvas pixels: the card and the picture.
    let (x0, y0) = (card.0.min(picture.0), card.1.min(picture.1));
    let (x1, y1) = (card.2.max(picture.2), card.3.max(picture.3));
    let (w, h) = ((x1 - x0) as f32, (y1 - y0) as f32);
    let scale = (pane.size.width.as_f32() / w).min(pane.size.height.as_f32() / h);
    let origin = point(
        pane.origin.x + (pane.size.width - px(w * scale)) / 2.,
        pane.origin.y + (pane.size.height - px(h * scale)) / 2.,
    );
    // A rectangle from canvas pixels to the pane.
    let rect = |(ax, ay, bx, by): (i64, i64, i64, i64)| {
        let at = |x: i64, y: i64| {
            point(
                origin.x + px((x - x0) as f32 * scale),
                origin.y + px((y - y0) as f32 * scale),
            )
        };
        Bounds::from_corners(at(ax, ay), at(bx, by))
    };
    CardLayout {
        canvas: rect((0, 0, canvas.width, canvas.height)),
        card: rect(card),
        picture: rect(picture),
    }
}

/// Paints the whole picture with what the printer cuts dimmed, then the
/// card from the canvas as the output holds it, then the paper's edge.
fn paint_card(
    layout: &CardLayout,
    (canvas, picture): &(Arc<RenderImage>, Arc<RenderImage>),
    colors: CardColors,
    window: &mut Window,
) {
    let mut paint = |bounds, image_bounds, image: &Arc<RenderImage>| {
        window
            .paint_image(
                bounds,
                image_bounds,
                Corners::default(),
                image.clone(),
                0,
                false,
            )
            .ok();
    };
    paint(layout.picture, layout.picture, picture);
    paint(layout.card, layout.canvas, canvas);
    for cut in layout.cut() {
        window.paint_quad(fill(cut, colors.dim));
    }
    window.paint_quad(outline(layout.card, colors.edge, BorderStyle::Dashed));
}

#[cfg(test)]
mod tests {
    use gpui_kit::size;
    use selphy::geometry::{Fit, place};

    use super::*;

    fn near(a: Pixels, b: f32) -> bool {
        (a.as_f32() - b).abs() < 0.01
    }

    fn pane() -> Bounds<Pixels> {
        Bounds::new(point(px(10.), px(20.)), size(px(600.), px(600.)))
    }

    #[test]
    fn a_whole_photo_card_fills_the_pane_and_nothing_is_cut() {
        let profile = selphy::paper::Paper::Postcard.default_profile();
        let placement = place(&profile, 1600, 1600, Fit::Contain).unwrap();
        let canvas = &placement.canvas;
        let layout = card_layout(pane(), &placement);

        assert!(near(layout.card.size.width, 600.), "the card is the widest");
        assert!(near(layout.card.origin.x, 10.));
        let scale = 600. / canvas.safe_width() as f32;
        assert!(near(
            layout.card.left() - layout.canvas.left(),
            canvas.trim(Edge::Left) as f32 * scale
        ));
        assert!(layout.cut().is_empty());
    }

    #[test]
    fn a_fill_card_picture_is_shown_whole_and_its_overflow_is_cut() {
        let profile = selphy::paper::Paper::Postcard.default_profile();
        let placement = place(&profile, 1600, 1600, Fit::Cover).unwrap();
        assert!(placement.x < 0, "the picture reaches past the canvas");
        let layout = card_layout(pane(), &placement);

        assert!(
            near(layout.picture.size.width, 600.),
            "the picture is the widest"
        );
        assert!(layout.picture.left() < layout.canvas.left());
        let cut = layout.cut();
        assert_eq!(cut.len(), 4);
        let empty =
            |b: Bounds<Pixels>| b.size.width <= Pixels::ZERO || b.size.height <= Pixels::ZERO;
        for (i, band) in cut.iter().enumerate() {
            assert!(
                empty(band.intersect(&layout.card)),
                "band {i} covers the card"
            );
            for other in &cut[i + 1..] {
                assert!(empty(band.intersect(other)), "bands overlap");
            }
        }
    }
}
