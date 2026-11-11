//! Light volume widgets: their frames, read and posed.

use super::super::super::{RenderModelPreviewMarker, TagReader};
use super::{
    AntennaQuad, Image, SamplerKind, ViewFrame, argb, decode_image, dot, elements,
    function_slot_of, has_flag, object_block, posed_on, rgb_colors_interpolate,
    rgb_interpolation_flags,
};
use blam_tags::bitmap::BitmapDefinition;
use blam_tags::{TagFile, TagStruct};
use std::sync::Arc;

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
    /// Which object's functions drive it; 0 is the opened.
    pub(crate) object: u8,
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
    pub(crate) image: Arc<Image>,
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
        colors: [
            argb(frame, "tint color hither"),
            argb(frame, "tint color yon"),
        ],
        color_exponent: one_if_unset(real("tint color exponent")),
        brightness_exponent: one_if_unset(real("brightness exponent")),
    }
}

/// The first image of a bitmap's sequence `sequence`, the first image where
/// it has no such sequence.
pub(in crate::app::model_preview) fn sequence_first_image(bitmap: &TagFile, sequence: usize) -> usize {
    BitmapDefinition::from_tag(bitmap)
        .sequences
        .get(sequence)
        .map_or(0, |sequence| sequence.first_bitmap_index.max(0) as usize)
}

fn read_light_volume(
    reader: &TagReader,
    path: &str,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
) -> Option<CeLightVolume> {
    let tag = reader.load(path, "light_volume", b"mgs2").ok()?;
    let root = tag.root();
    let frame = read_frame(*elements(root, "frames").first()?);
    let count = root.read_int_any("count").unwrap_or(0).max(0) as usize;
    if count == 0 {
        return None;
    }
    let name = root.read_string("attachment marker").unwrap_or_default();
    let (node, position, forward) = super::super::lights::marker_or_origin(markers, &name, owner)?;
    // `rasterizer_widget_set_texture`: no map is the globals' `glow`.
    let image = match root
        .read_tag_ref_path("map")
        .filter(|path| !path.is_empty())
    {
        Some(map) => {
            let bitmap = reader.load(&map, "bitmap", b"bitm").ok()?;
            let sequence = root.read_int_any("sequence index").unwrap_or(0).max(0) as usize;
            let index = sequence_first_image(&bitmap, sequence);
            Arc::new(decode_image(reader, &map, SamplerKind::Texture2D, index)?)
        }
        None => super::super::super::materials::read_glow(reader)?,
    };
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let mut brightness = [
        real("perpendicular brightness scale"),
        real("parallel brightness scale"),
    ];
    if brightness == [0.0; 2] {
        brightness = [1.0; 2];
    }
    let source_slot = function_slot_of(root, "brightness scale source");
    Some(CeLightVolume {
        object: owner,
        node,
        position,
        forward,
        fade: [real("near fade distance"), real("far fade distance")],
        brightness,
        source: source_slot,
        count,
        interpolation: rgb_interpolation_flags(
            has_flag(root, "flags", "interpolate color in hsv"),
            has_flag(root, "flags", "...more colors"),
        ),
        frame,
        image,
    })
}

/// The light volumes among `object`'s widgets.
pub(in crate::app::model_preview) fn read_ce_light_volumes(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
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
        .filter_map(|(_, path)| read_light_volume(reader, &path, owner, markers))
        .collect()
}

impl super::super::functions::ReadsOutputs for CeLightVolume {
    fn object(&self) -> u8 {
        self.object
    }
    fn outputs_read(&self) -> Vec<usize> {
        self.source.into_iter().collect()
    }
}

impl CeLightVolume {
    /// Where its marker is, which `light_volume_submit` sorts it by.
    pub(crate) fn marker(&self, bones: Option<&[[f32; 4]]>) -> [f32; 3] {
        posed_on(self.node, self.position, self.forward, bones).0
    }

    /// `light_volume_submit` and `light_volume_render`: sprites along the
    /// marker, brighter as it faces the camera the way the tag says.
    pub(crate) fn quads(
        &self,
        outputs: &[f32; 4],
        active: &[bool; 4],
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
    ) -> Vec<AntennaQuad> {
        let (origin, forward) = posed_on(self.node, self.position, self.forward, bones);
        let length = dot(forward, forward).sqrt().max(0.0001);
        let forward = forward.map(|axis| axis / length);
        let value = self.source.map(|slot| outputs[slot]);
        let depth = dot(
            view.forward,
            std::array::from_fn(|axis| origin[axis] - view.eye[axis]),
        );
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
        let mut brightness =
            (perpendicular * (1.0 - facing) + parallel * facing).clamp(0.0, 1.0) * fade;
        // `light_volume_render` scales by the output only while it is on.
        if let (Some(value), Some(slot)) = (value, self.source)
            && active[slot]
        {
            brightness *= value;
        }
        let frame = &self.frame;
        let [hither, yon] = frame.colors;
        if brightness <= 0.0
            || (hither[3] <= 0.0 && yon[3] <= 0.0)
            || (frame.radius[0] <= 0.0 && frame.radius[1] <= 0.0)
        {
            return Vec::new();
        }
        let last = (self.count - 1).max(1) as f32;
        (0..self.count)
            .map(|at| {
                let mut x = at as f32 / last;
                if frame.offset_exponent != 1.0 {
                    x = x.powf(frame.offset_exponent);
                }
                let radius_at = if frame.radius_exponent != 1.0 {
                    x.powf(frame.radius_exponent)
                } else {
                    x
                };
                let radius = (1.0 - radius_at) * frame.radius[0] + frame.radius[1] * radius_at;
                let color_at = if frame.color_exponent != 1.0 {
                    x.powf(frame.color_exponent)
                } else {
                    x
                };
                let brightness_at = if frame.brightness_exponent != 1.0 {
                    x.powf(frame.brightness_exponent)
                } else {
                    x
                };
                let rgb = rgb_colors_interpolate(
                    self.interpolation,
                    [hither[0], hither[1], hither[2]],
                    [yon[0], yon[1], yon[2]],
                    color_at,
                );
                let alpha =
                    ((1.0 - brightness_at) * hither[3] + yon[3] * brightness_at) * brightness;
                let along = frame.length * x + frame.offset;
                let center: [f32; 3] =
                    std::array::from_fn(|axis| origin[axis] + forward[axis] * along);
                let corner = |u: f32, v: f32| {
                    let place: [f32; 3] = std::array::from_fn(|axis| {
                        center[axis]
                            + view.right[axis] * radius * (u * 2.0 - 1.0)
                            + view.up[axis] * radius * (1.0 - v * 2.0)
                    });
                    [place[0], place[1], place[2], u, v]
                };
                AntennaQuad {
                    corners: [
                        corner(0.0, 1.0),
                        corner(1.0, 1.0),
                        corner(0.0, 0.0),
                        corner(1.0, 0.0),
                    ],
                    color: [rgb[0], rgb[1], rgb[2], alpha.clamp(0.0, 1.0)],
                }
            })
            .collect()
    }
}
