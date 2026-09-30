//! Thumbnails for the photo list, loaded in the background a few at a time
//! and cached by photo. A photo removed from the list keeps its thumbnail,
//! so that Undo shows it at once.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{AppContext as _, Context, RenderImage};
use image::{Frame, RgbImage, RgbaImage};
use selphy::imaging;

use crate::batch::PhotoId;

/// The longer side of a thumbnail, in pixels: twice the row's thumbnail,
/// for Retina screens.
const THUMBNAIL_PX: u32 = 96;

/// Thumbnails decoded at once. Each decode holds the full-size photo.
const LOADERS: usize = 2;

#[derive(Debug)]
enum Thumbnail {
    Waiting,
    Ready(Arc<RenderImage>),
    Failed,
}

#[derive(Debug, Default)]
pub struct Thumbnails {
    images: HashMap<PhotoId, Thumbnail>,
    pending: VecDeque<(PhotoId, PathBuf)>,
    loading: usize,
}

impl Thumbnails {
    /// The thumbnail of `id`, once it has loaded.
    pub fn get(&self, id: PhotoId) -> Option<Arc<RenderImage>> {
        match self.images.get(&id)? {
            Thumbnail::Ready(image) => Some(image.clone()),
            Thumbnail::Waiting | Thumbnail::Failed => None,
        }
    }

    /// Starts loading the photos that have no thumbnail yet.
    pub fn request(
        &mut self,
        photos: impl IntoIterator<Item = (PhotoId, PathBuf)>,
        cx: &mut Context<Self>,
    ) {
        for (id, source) in photos {
            if let Entry::Vacant(entry) = self.images.entry(id) {
                entry.insert(Thumbnail::Waiting);
                self.pending.push_back((id, source));
            }
        }
        self.pump(cx);
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        while self.loading < LOADERS {
            let Some((id, source)) = self.pending.pop_front() else {
                return;
            };
            self.loading += 1;
            cx.spawn(async move |this, cx| {
                let image = cx
                    .background_spawn(async move {
                        imaging::thumbnail(&source, THUMBNAIL_PX)
                            .ok()
                            .map(|rgb| render_image(&rgb))
                    })
                    .await;
                this.update(cx, |this, cx| {
                    let thumbnail = image.map_or(Thumbnail::Failed, Thumbnail::Ready);
                    this.images.insert(id, thumbnail);
                    this.loading -= 1;
                    this.pump(cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }
}

/// `rgb` as the BGRA image GPUI draws.
pub fn render_image(rgb: &RgbImage) -> Arc<RenderImage> {
    let bgra = RgbaImage::from_fn(rgb.width(), rgb.height(), |x, y| {
        let [r, g, b] = rgb.get_pixel(x, y).0;
        image::Rgba([b, g, r, 255])
    });
    Arc::new(RenderImage::new([Frame::new(bgra)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_images_are_bgra_of_the_same_size() {
        let rgb = RgbImage::from_pixel(3, 2, image::Rgb([10, 20, 30]));
        let image = render_image(&rgb);
        let size = image.size(0);
        assert_eq!((size.width.0, size.height.0), (3, 2));
        assert_eq!(&image.as_bytes(0).unwrap()[..4], &[30, 20, 10, 255]);
    }
}
