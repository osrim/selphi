//! The card preview: the selected photo rendered by `Job::preview`, the
//! same code as Prepare, with the trim zone drawn over it and the placement
//! under it. A render made for another photo or an older revision is
//! dropped when it arrives.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, button::Button,
    spinner::Spinner, v_flex,
};
use gpui_kit::{
    Action as _, App, AppContext as _, BorderStyle, Bounds, Context, Hsla, IntoElement, ObjectFit,
    ParentElement as _, Pixels, Render, RenderImage, SharedString, Styled as _, StyledImage as _,
    Task, Window, canvas, div, fill, img, outline, pattern_slash, point, size,
};
use selphy::geometry::{Canvas, Edge, Placement};
use selphy::paper::Paper;
use selphy::prepare::Job;

use crate::OpenSettings;
use crate::batch::PhotoId;
use crate::text::{error_sentence, file_name, paper_label, placement_text};
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
    /// The paper has no profile yet.
    NotCalibrated(Paper),
    /// The printer config cannot be read.
    Broken(SharedString),
}

struct Rendered {
    image: Arc<RenderImage>,
    placement: Placement,
    caption: SharedString,
}

enum State {
    Empty,
    NotCalibrated(Paper),
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
            Setup::NotCalibrated(paper) => {
                self.key = None;
                return self.set_state(State::NotCalibrated(*paper), cx);
            }
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
                        placement: preview.placement,
                        caption: caption.into(),
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                if this.key != Some(key) {
                    if let Ok(rendered) = result {
                        cx.drop_image(rendered.image, None);
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
            cx.drop_image(old.image, None);
        }
        cx.notify();
    }

    fn render_card(rendered: &Rendered, cx: &App) -> impl IntoElement {
        let image = rendered.image.clone();
        let canvas_px = rendered.placement.canvas.clone();
        let colors = OverlayColors {
            band: cx.theme().muted_foreground.opacity(0.35),
            outline: cx.theme().foreground.opacity(0.6),
        };
        v_flex()
            .size_full()
            .min_h_0()
            .gap_3()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        img(image.clone())
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    )
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                let drawn = ObjectFit::Contain.get_bounds(bounds, image.size(0));
                                paint_trim_zone(&trim_zone(drawn, &canvas_px), colors, window);
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
                    .child(div().font_medium().child(rendered.caption.clone()))
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
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
            State::NotCalibrated(paper) => centred()
                .child(Icon::new(IconName::TriangleAlert).large())
                .child(format!(
                    "{} paper is not calibrated. Calibrate it in Settings to prepare for it.",
                    paper_label(*paper)
                ))
                .child(
                    Button::new("open-settings")
                        .label("Open Settings")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(OpenSettings.boxed_clone(), cx)
                        }),
                )
                .into_any_element(),
            State::Broken(message) | State::Failed(message) => centred()
                .child(
                    Icon::new(IconName::CircleX)
                        .large()
                        .text_color(cx.theme().danger),
                )
                .child(div().text_center().child(message.clone()))
                .into_any_element(),
        }
    }
}

/// The colours of the trim zone, from the theme.
#[derive(Clone, Copy)]
struct OverlayColors {
    band: Hsla,
    outline: Hsla,
}

/// The trim zone of a card drawn at `drawn`: the four bands the printer
/// cuts, left, top, right and bottom, and the safe box inside them.
#[derive(Debug, PartialEq)]
pub struct TrimZone {
    pub bands: [Bounds<Pixels>; 4],
    pub safe: Bounds<Pixels>,
}

/// The trim zone of `canvas` scaled to `drawn`, the bounds the canvas image
/// is drawn in.
pub fn trim_zone(drawn: Bounds<Pixels>, canvas: &Canvas) -> TrimZone {
    let sx = drawn.size.width / canvas.width as f32;
    let sy = drawn.size.height / canvas.height as f32;
    let [left, top, right, bottom] = Edge::ALL.map(|edge| canvas.trim(edge) as f32);
    let (x0, y0) = (drawn.origin.x, drawn.origin.y);
    let (w, h) = (drawn.size.width, drawn.size.height);
    let rect = |x, y, width, height| Bounds::new(point(x, y), size(width, height));
    let top_h = sy * top;
    let bottom_h = sy * bottom;
    let middle_h = h - top_h - bottom_h;
    let safe = rect(
        x0 + sx * left,
        y0 + top_h,
        w - sx * (left + right),
        middle_h,
    );
    TrimZone {
        bands: [
            rect(x0, y0 + top_h, sx * left, middle_h),
            rect(x0, y0, w, top_h),
            rect(x0 + w - sx * right, y0 + top_h, sx * right, middle_h),
            rect(x0, y0 + h - bottom_h, w, bottom_h),
        ],
        safe,
    }
}

fn paint_trim_zone(zone: &TrimZone, colors: OverlayColors, window: &mut Window) {
    for band in zone.bands {
        window.paint_quad(fill(band, pattern_slash(colors.band, 1.0, 4.0)));
    }
    window.paint_quad(outline(zone.safe, colors.outline, BorderStyle::Dashed));
}

#[cfg(test)]
mod tests {
    use gpui_kit::px;
    use selphy::geometry::{Fit, Orientation, place};

    use super::*;

    #[test]
    fn the_bands_follow_the_trims_scaled_to_the_drawn_card() {
        let profile = Paper::Postcard.starting_profile();
        let canvas = Canvas::new(&profile, Orientation::Landscape);
        let scale = 0.5;
        let drawn = Bounds::new(
            point(px(10.), px(20.)),
            size(
                px(canvas.width as f32 * scale),
                px(canvas.height as f32 * scale),
            ),
        );
        let zone = trim_zone(drawn, &canvas);
        let [left, top, right, bottom] = zone.bands;
        let near = |a: Pixels, b: f32| (a.as_f32() - b).abs() < 0.01;

        assert!(near(
            left.size.width,
            canvas.trim(Edge::Left) as f32 * scale
        ));
        assert!(near(top.size.height, canvas.trim(Edge::Top) as f32 * scale));
        assert!(near(
            right.size.width,
            canvas.trim(Edge::Right) as f32 * scale
        ));
        assert!(near(
            bottom.size.height,
            canvas.trim(Edge::Bottom) as f32 * scale
        ));
        assert!(near(
            zone.safe.size.width,
            canvas.safe_width() as f32 * scale
        ));
        assert!(near(
            zone.safe.size.height,
            canvas.safe_height() as f32 * scale
        ));
        assert!(near(
            zone.safe.origin.x,
            10. + canvas.trim(Edge::Left) as f32 * scale
        ));
        assert!(near(
            right.origin.x + right.size.width,
            10. + drawn.size.width.as_f32()
        ));
        assert!(near(
            bottom.origin.y + bottom.size.height,
            20. + drawn.size.height.as_f32()
        ));
    }

    #[test]
    fn a_portrait_canvas_puts_its_trims_on_its_own_edges() {
        let profile = Paper::Postcard.starting_profile();
        let placement = place(&profile, 1080, 1920, Fit::Contain).unwrap();
        let canvas = &placement.canvas;
        let drawn = Bounds::new(
            point(px(0.), px(0.)),
            size(px(canvas.width as f32), px(canvas.height as f32)),
        );
        let [left, top, ..] = trim_zone(drawn, canvas).bands;
        assert_eq!(left.size.width.as_f32(), canvas.trim(Edge::Left) as f32);
        assert_eq!(top.size.height.as_f32(), canvas.trim(Edge::Top) as f32);
    }
}
