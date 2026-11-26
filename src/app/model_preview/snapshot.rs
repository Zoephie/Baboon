//! The Model Preview's snapshot: the frame the viewport shows, drawn again
//! on black and on white, matted into an image with no background in it, and
//! copied to the clipboard (docs/model-preview-snapshot-plan.md).
//!
//! Every pass blends the background in linearly, so each pixel is
//! `C + T · background`. On black it is `C`; on white `C + T`. So both renders
//! give what the model adds and how much of the background shows through,
//! for every game and shading mode, with no shader of theirs changed. The
//! matte runs on the GPU (`renderer::snapshot`); [`matte_pixel`] is the same
//! sum, which the tests hold the shader to.

use super::frame::ModelGpuFrame;
use eframe::egui;
use std::sync::{Arc, Mutex};

/// A snapshot asked for: the frame on screen when it was, with the egui
/// helpers drawn over it (markers, armature, bounding sphere), its size,
/// and where its result goes. The viewport's next paint draws it before its
/// own frame, so nothing has moved on.
pub(in crate::app) struct SnapshotRequest {
    pub(in crate::app::model_preview) frame: ModelGpuFrame,
    /// In pixels.
    pub(in crate::app) size: [i32; 2],
    /// The helpers, tessellated at the snapshot's pixels per point, in
    /// points over the viewport.
    pub(in crate::app) helpers: Vec<egui::ClippedPrimitive>,
    /// The viewport's size in points, which `helpers` are laid out over.
    pub(in crate::app) points: [f32; 2],
    /// Its size over the viewport's (`RenderCore::pixel_scale`).
    pub(in crate::app) scale: f32,
    pub(in crate::app) result: Mutex<Option<Result<SnapshotImage, String>>>,
}

impl SnapshotRequest {
    pub(in crate::app::model_preview) fn new(
        frame: ModelGpuFrame,
        size: [i32; 2],
        helpers: Vec<egui::ClippedPrimitive>,
        points: [f32; 2],
        scale: f32,
    ) -> Arc<Self> {
        Arc::new(Self { frame, size, helpers, points, scale, result: Mutex::new(None) })
    }

    /// What the paint left, once it has drawn: taken, so it is handled once.
    pub(in crate::app) fn take_result(&self) -> Option<Result<SnapshotImage, String>> {
        self.result.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }

    pub(in crate::app) fn set_result(&self, result: Result<SnapshotImage, String>) {
        *self.result.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
    }
}

/// A snapshot's image: straight-alpha RGBA, top row first.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct SnapshotImage {
    pub(in crate::app) width: usize,
    pub(in crate::app) height: usize,
    pub(in crate::app) rgba: Vec<u8>,
}

/// What the viewport drew last pass, kept for a snapshot: the frame, and the
/// egui helpers painted over it (in points from the viewport's corner).
#[derive(Clone)]
pub(in crate::app) struct SnapshotSource {
    pub(in crate::app::model_preview) frame: ModelGpuFrame,
    /// The viewport's size in points.
    pub(in crate::app) points: egui::Vec2,
    pub(in crate::app) pixels_per_point: f32,
    pub(in crate::app) helpers: Vec<egui::epaint::ClippedShape>,
}

impl SnapshotSource {
    /// The image's size at `scale`, in pixels.
    pub(in crate::app) fn size(&self, scale: f32) -> [i32; 2] {
        snapshot_size(self.points, self.pixels_per_point, scale)
    }

    /// The request for a snapshot of this frame at `scale`: its helpers
    /// tessellated as finely as the image is.
    pub(in crate::app) fn request(&self, ctx: &egui::Context, scale: f32) -> Arc<SnapshotRequest> {
        let pixels_per_point = self.pixels_per_point * scale;
        let helpers = if self.helpers.is_empty() { Vec::new() } else { ctx.tessellate(self.helpers.clone(), pixels_per_point) };
        SnapshotRequest::new(self.frame.clone(), self.size(scale), helpers, [self.points.x, self.points.y], scale)
    }
}

/// A viewport of `points` at `pixels_per_point`, scaled by `scale`, in whole
/// pixels, at least one each way.
pub(in crate::app) fn snapshot_size(points: egui::Vec2, pixels_per_point: f32, scale: f32) -> [i32; 2] {
    [points.x, points.y].map(|edge| ((edge * pixels_per_point * scale).round() as i32).max(1))
}

/// Bytes of image past which a snapshot asks first: about 4 GB at its peak,
/// with the copies the clipboard makes.
pub(in crate::app) const LARGE_SNAPSHOT_BYTES: u64 = 1 << 30;

/// The bytes an image of `size` takes, as RGBA.
pub(in crate::app) fn snapshot_bytes(size: [i32; 2]) -> u64 {
    size[0].max(0) as u64 * size[1].max(0) as u64 * 4
}

/// Where a snapshot is.
#[derive(Default)]
pub(in crate::app) enum SnapshotPhase {
    #[default]
    Idle,
    /// A large snapshot waiting on a second press, at this size.
    Confirming([i32; 2]),
    /// Asked for this pass: the viewport makes the request from the frame on
    /// screen.
    Asked,
    /// Drawn in the viewport's next paint.
    Drawing(Arc<SnapshotRequest>),
    /// Being written to the clipboard; the message when it is.
    Copying(Arc<Mutex<Option<String>>>),
}

impl SnapshotPhase {
    /// Taking a snapshot: the camera waits for it.
    pub(in crate::app) fn busy(&self) -> bool {
        matches!(self, Self::Asked | Self::Drawing(_) | Self::Copying(_))
    }
}

/// Copy `image` to the clipboard on a worker, and say when it has, or why
/// it couldn't. The clipboard stays open for the app's life: on X11 and
/// Wayland a copy lasts only as long as its owner does.
pub(in crate::app) fn copy_to_clipboard(image: SnapshotImage, ctx: &egui::Context) -> Arc<Mutex<Option<String>>> {
    let done = Arc::new(Mutex::new(None));
    let (slot, ctx) = (Arc::clone(&done), ctx.clone());
    crate::app::shell::spawn_background("copying the preview to the clipboard", move || {
        let (width, height) = (image.width, image.height);
        // A panic still says so, or the snapshot would wait on it forever.
        let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| write_clipboard(image)))
            .unwrap_or_else(|panic| Err(crate::app::shell::worker::panic_detail(panic.as_ref()).to_owned()));
        let message = match written {
            Ok(()) => format!("Copied the preview to the clipboard ({width} × {height})"),
            Err(error) => format!("Could not copy the preview to the clipboard: {error}"),
        };
        *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message);
        ctx.request_repaint();
    });
    done
}

#[cfg(not(test))]
fn write_clipboard(image: SnapshotImage) -> Result<(), String> {
    static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);
    let mut held = CLIPBOARD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if held.is_none() {
        *held = Some(arboard::Clipboard::new().map_err(|error| error.to_string())?);
    }
    let clipboard = held.as_mut().expect("made above");
    clipboard
        .set_image(arboard::ImageData { width: image.width, height: image.height, bytes: image.rgba.into() })
        .map_err(|error| error.to_string())
}

/// Under test, what would have gone to the clipboard, so a test can read it
/// and the suite never overwrites the machine's clipboard.
#[cfg(test)]
pub(in crate::app) static TEST_CLIPBOARD: Mutex<Option<SnapshotImage>> = Mutex::new(None);

#[cfg(test)]
fn write_clipboard(image: SnapshotImage) -> Result<(), String> {
    *TEST_CLIPBOARD.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(image);
    Ok(())
}

/// `image` zoomed about its centre as a composite's `view_scale` zooms the
/// frame: each pixel shows the one `(place - ½) · view_scale + ½` of the
/// way across, sampled between its neighbours in premultiplied colour, and
/// nothing past the edges. What a tiled snapshot's composites left out,
/// since a tile can't sample its neighbours.
pub(in crate::app) fn zoom_about_centre(image: &SnapshotImage, view_scale: f32) -> SnapshotImage {
    let (width, height) = (image.width, image.height);
    let premultiplied = |x: usize, y: usize| {
        let at = (y * width + x) * 4;
        let alpha = image.rgba[at + 3] as f32 / 255.0;
        [image.rgba[at] as f32 / 255.0 * alpha, image.rgba[at + 1] as f32 / 255.0 * alpha, image.rgba[at + 2] as f32 / 255.0 * alpha, alpha]
    };
    let mut rgba = vec![0u8; image.rgba.len()];
    for y in 0..height {
        for x in 0..width {
            let place = [(x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32];
            let source = place.map(|axis| (axis - 0.5) * view_scale + 0.5);
            if source.iter().any(|axis| !(0.0..=1.0).contains(axis)) {
                continue;
            }
            let (sx, sy) = (source[0] * width as f32 - 0.5, source[1] * height as f32 - 0.5);
            let (x0, y0) = (sx.floor().max(0.0) as usize, sy.floor().max(0.0) as usize);
            let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
            let (fx, fy) = ((sx - x0 as f32).clamp(0.0, 1.0), (sy - y0 as f32).clamp(0.0, 1.0));
            let mix = |a: [f32; 4], b: [f32; 4], t: f32| -> [f32; 4] { std::array::from_fn(|c| a[c] + (b[c] - a[c]) * t) };
            let sample = mix(mix(premultiplied(x0, y0), premultiplied(x1, y0), fx), mix(premultiplied(x0, y1), premultiplied(x1, y1), fx), fy);
            let alpha = sample[3];
            if alpha < 0.5 / 255.0 {
                continue;
            }
            let at = (y * width + x) * 4;
            for channel in 0..3 {
                rgba[at + channel] = ((sample[channel] / alpha).clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            rgba[at + 3] = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    SnapshotImage { width, height, rgba }
}

/// One pixel of the matte, as `snapshot_matte.frag` makes it, from the frame
/// drawn on black and on white: straight colour and alpha, each 0 to 1. The
/// shader's oracle in the tests.
#[cfg(test)]
pub(in crate::app) fn matte_pixel(black: [f32; 3], white: [f32; 3]) -> [f32; 4] {
    let c = black.map(|channel| channel.max(0.0));
    let t: [f32; 3] = std::array::from_fn(|at| (white[at] - c[at]).clamp(0.0, 1.0));
    let cover = 1.0 - (t[0] + t[1] + t[2]) / 3.0;
    let alpha = cover.max(c[0].max(c[1]).max(c[2])).clamp(0.0, 1.0);
    if alpha < 0.5 / 255.0 {
        return [0.0; 4];
    }
    [(c[0] / alpha).clamp(0.0, 1.0), (c[1] / alpha).clamp(0.0, 1.0), (c[2] / alpha).clamp(0.0, 1.0), alpha]
}

#[cfg(test)]
mod tests {
    use super::{SnapshotImage, matte_pixel, zoom_about_centre};

    /// The zoom left out of a tiled snapshot's composites: at 1 nothing
    /// changes; at ½ the middle half fills the image (a centred square
    /// doubles); at 4 the picture shrinks, and past its edge is clear.
    #[test]
    fn a_tiled_snapshots_zoom_is_about_its_centre() {
        // 8 × 8, clear but for a red 2 × 2 square in the middle.
        let mut rgba = vec![0u8; 8 * 8 * 4];
        for (x, y) in [(3, 3), (4, 3), (3, 4), (4, 4)] {
            rgba[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
        }
        let image = SnapshotImage { width: 8, height: 8, rgba };
        assert_eq!(zoom_about_centre(&image, 1.0), image, "at 1, as it was");
        let zoomed = zoom_about_centre(&image, 0.5);
        let at = |x: usize, y: usize| zoomed.rgba[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4].to_vec();
        assert_eq!(at(3, 3), [255, 0, 0, 255], "inside the grown square");
        assert_eq!(at(0, 0), [0, 0, 0, 0], "outside it");
        let wider = zoom_about_centre(&image, 4.0);
        assert_eq!(wider.rgba[..4], [0, 0, 0, 0], "past the picture's edge, clear");
    }

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5)
    }

    /// Each kind of pixel a frame has, matted: background alone is clear;
    /// an opaque surface is whole; a half-covering one is half; light added
    /// over nothing counts as cover as far as it is bright; a multiply blend
    /// darkens what shows through; an alpha a byte can't show is clear, with
    /// no colour left in it.
    #[test]
    fn the_matte_makes_each_kind_of_pixel() {
        assert_eq!(matte_pixel([0.0; 3], [1.0; 3]), [0.0; 4], "background alone");
        assert!(close(matte_pixel([0.2, 0.4, 0.6], [0.2, 0.4, 0.6]), [0.2, 0.4, 0.6, 1.0]), "opaque");
        // Half a red surface over the background: C = 0.5 red, T = 0.5.
        assert!(close(matte_pixel([0.5, 0.0, 0.0], [1.0, 0.5, 0.5]), [1.0, 0.0, 0.0, 0.5]), "half covered");
        // Light added over nothing: C = glow, T = 1 (clamped on white).
        let glow = matte_pixel([0.3, 0.15, 0.0], [1.0, 1.0, 1.0]);
        assert!(close(glow, [1.0, 0.5, 0.0, 0.3]), "glow: {glow:?}");
        // A multiply blend by 0.25 grey: C = 0, T = 0.25.
        assert!(close(matte_pixel([0.0; 3], [0.25; 3]), [0.0, 0.0, 0.0, 0.75]), "multiply");
        assert_eq!(matte_pixel([0.001, 0.0, 0.0], [1.0, 1.0, 1.0]), [0.0; 4], "below a byte");
    }
}
