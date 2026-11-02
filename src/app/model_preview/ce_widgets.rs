//! Halo CE's widgets beyond antennas: `light_volume` (`mgs2`), drawn by
//! `light_volume_render` as a row of screen-facing sprites along a marker.
//! Read from the Xbox 360 build's symbols; what tool's postprocess
//! (`sub_5251C0`) fills in for the game is filled in here.

use std::sync::Arc;

use blam_tags::{TagFile, TagStruct};

use super::ce_attachments::{elements, object_block};
use super::ce_lights::{CeAntennaQuad, CeFlareView, posed_on};
use super::ce_passes::argb;
use super::h2_passes::{H2Image, ce_color_interpolate};
use super::materials::decode_h2_image;
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;

/// One of `frames`, as tool leaves it.
#[derive(Debug, Clone, Copy)]
struct LightVolumeFrame {
    offset: f32,
    offset_exponent: f32,
    length: f32,
    radius: [f32; 2],
    radius_exponent: f32,
    /// Red, green, blue and alpha, hither and yon.
    colors: [[f32; 4]; 2],
    color_exponent: f32,
    brightness_exponent: f32,
}

/// A `light_volume` widget at the first marker of its name.
#[derive(Debug, Clone)]
pub(crate) struct CeLightVolume {
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    fade: [f32; 2],
    /// Perpendicular and parallel.
    brightness: [f32; 2],
    source: Option<usize>,
    count: usize,
    interpolation: i128,
    /// `light_volume_interpolate_frames` lerps the first frame with itself,
    /// so the first is the one that draws.
    frame: LightVolumeFrame,
    pub(crate) image: Arc<H2Image>,
}

fn one_if_unset(value: f32) -> f32 {
    if value <= 0.0 { 1.0 } else { value }
}

fn read_frame(frame: TagStruct<'_>) -> LightVolumeFrame {
    let real = |name: &str| frame.read_real(name).unwrap_or(0.0);
    LightVolumeFrame {
        offset: real("offset from marker"),
        offset_exponent: one_if_unset(real("offset exponent")),
        length: real("length"),
        radius: [real("radius hither"), real("radius yon")],
        radius_exponent: one_if_unset(real("radius exponent")),
        colors: [argb(frame, "tint color hither"), argb(frame, "tint color yon")],
        color_exponent: one_if_unset(real("tint color exponent")),
        brightness_exponent: one_if_unset(real("brightness exponent")),
    }
}

fn read_light_volume(source: &TagSource, path: &str, markers: &[RenderModelPreviewMarker]) -> Option<CeLightVolume> {
    let tag = load_referenced_tag_from_source(source, path, "light_volume", b"mgs2").ok()?;
    let root = tag.root();
    let frame = read_frame(*elements(root, "frames").first()?);
    let count = root.read_int_any("count").unwrap_or(0).max(0) as usize;
    if count == 0 {
        return None;
    }
    let name = root.read_string("attachment marker").unwrap_or_default();
    let (node, position, forward) = if name.is_empty() {
        (0, [0.0; 3], [1.0, 0.0, 0.0])
    } else {
        let marker = markers.iter().find(|marker| marker.name.eq_ignore_ascii_case(&name))?;
        (marker.node_index.max(0) as usize, marker.position, marker.axes[0])
    };
    // `rasterizer_widget_set_texture`: no map is the globals' `glow`.
    let image = match root.read_tag_ref_path("map").filter(|path| !path.is_empty()) {
        Some(map) => {
            let bitmap = load_referenced_tag_from_source(source, &map, "bitmap", b"bitm").ok()?;
            let sequence = root.read_int_any("sequence index").unwrap_or(0).max(0) as usize;
            let index = elements(bitmap.root(), "sequences")
                .get(sequence)
                .map_or(0, |sequence| sequence.read_int_any("first bitmap index").unwrap_or(0).max(0) as usize);
            Arc::new(decode_h2_image(source, &map, SamplerKind::Texture2D, index)?)
        }
        None => super::h2_light_volumes::read_glow(source)?,
    };
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let mut brightness = [real("perpendicular brightness scale"), real("parallel brightness scale")];
    if brightness == [0.0; 2] {
        brightness = [1.0; 2];
    }
    let source_slot = usize::try_from(root.read_int_any("brightness scale source").unwrap_or(0) - 1).ok().filter(|slot| *slot < 4);
    Some(CeLightVolume {
        node,
        position,
        forward,
        fade: [real("near fade distance"), real("far fade distance")],
        brightness,
        source: source_slot,
        count,
        interpolation: root.read_int_any("flags").unwrap_or(0) & 3,
        frame,
        image,
    })
}

/// The light volumes among `object`'s widgets.
pub(super) fn read_ce_light_volumes(
    source: &TagSource,
    object: &TagFile,
    markers: &[RenderModelPreviewMarker],
) -> Vec<CeLightVolume> {
    let Some(block) = object_block(object, "widgets") else {
        return Vec::new();
    };
    let group = u32::from_be_bytes(*b"mgs2");
    elements(block, "widgets")
        .iter()
        .filter_map(|widget| widget.read_tag_ref_with_group("reference"))
        .filter(|(kind, path)| *kind == group && !path.is_empty())
        .filter_map(|(_, path)| read_light_volume(source, &path, markers))
        .collect()
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl CeLightVolume {
    /// `light_volume_submit` and `light_volume_render`: sprites along the
    /// marker, brighter as it faces the camera the way the tag says.
    pub(crate) fn quads(&self, outputs: &[f32; 4], bones: Option<&[[f32; 4]]>, view: &CeFlareView) -> Vec<CeAntennaQuad> {
        let (origin, forward) = posed_on(self.node, self.position, self.forward, bones);
        let length = dot(forward, forward).sqrt().max(0.0001);
        let forward = forward.map(|axis| axis / length);
        let value = self.source.map(|slot| outputs[slot]);
        if value.is_some_and(|value| value <= 0.0) {
            return Vec::new();
        }
        let depth = dot(view.forward, std::array::from_fn(|axis| origin[axis] - view.eye[axis]));
        let [near, far] = self.fade;
        let mut fade = 1.0;
        if far > 0.0 {
            if depth >= far {
                return Vec::new();
            }
            fade = ((depth - far) / (near - far)).clamp(0.0, 1.0);
        }
        let facing = dot(view.forward, forward).abs();
        let [perpendicular, parallel] = self.brightness;
        let mut brightness = (perpendicular * (1.0 - facing) + parallel * facing).clamp(0.0, 1.0) * fade;
        if let Some(value) = value {
            brightness *= value;
        }
        let frame = &self.frame;
        let [hither, yon] = frame.colors;
        if brightness <= 0.0 || (hither[3] <= 0.0 && yon[3] <= 0.0) || (frame.radius[0] <= 0.0 && frame.radius[1] <= 0.0) {
            return Vec::new();
        }
        let last = (self.count - 1).max(1) as f32;
        (0..self.count)
            .map(|at| {
                let mut x = at as f32 / last;
                if frame.offset_exponent != 1.0 {
                    x = x.powf(frame.offset_exponent);
                }
                let radius_at = if frame.radius_exponent != 1.0 { x.powf(frame.radius_exponent) } else { x };
                let radius = (1.0 - radius_at) * frame.radius[0] + frame.radius[1] * radius_at;
                let color_at = if frame.color_exponent != 1.0 { x.powf(frame.color_exponent) } else { x };
                let brightness_at = if frame.brightness_exponent != 1.0 { x.powf(frame.brightness_exponent) } else { x };
                let rgb = ce_color_interpolate(
                    self.interpolation,
                    [hither[0], hither[1], hither[2]],
                    [yon[0], yon[1], yon[2]],
                    color_at,
                );
                let alpha = ((1.0 - brightness_at) * hither[3] + yon[3] * brightness_at) * brightness;
                let along = frame.length * x + frame.offset;
                let center: [f32; 3] = std::array::from_fn(|axis| origin[axis] + forward[axis] * along);
                let corner = |u: f32, v: f32| {
                    let place: [f32; 3] = std::array::from_fn(|axis| {
                        center[axis] + view.right[axis] * radius * (u * 2.0 - 1.0) + view.up[axis] * radius * (1.0 - v * 2.0)
                    });
                    [place[0], place[1], place[2], u, v]
                };
                CeAntennaQuad {
                    corners: [corner(0.0, 1.0), corner(1.0, 1.0), corner(0.0, 0.0), corner(1.0, 0.0)],
                    color: [rgb[0], rgb[1], rgb[2], alpha.clamp(0.0, 1.0)],
                }
            })
            .collect()
    }
}
