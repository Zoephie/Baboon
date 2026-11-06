//! Halo 4's lights on objects: the `light`s (`Midnight_Light_Parameters`) and
//! `cheap_light`s an object attaches, each drawn after the static lighting as
//! a screen-space light (`RenderSSLightInternal`, X360 0x83AE2910) through
//! `rasterizer_globals`' explicit shader 187 `gobo_specular` (artistic) or
//! 204 `pointphysicallycorrect`, added into the light. See
//! `docs/h4-preview-research/lights.md`.

use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use super::super::attachment::MarkerFrame;
use super::super::ce::attachments::{elements, object_block};
use super::super::ce::lights::posed_on;
use super::super::h2::functions::H2FunctionValues;
use super::super::dx9::passes::tag_function_at;
use super::super::gen3::effects::{marker_name, part_markers};
use super::super::gen3::shaders::H3Pass;
use super::super::reach::lights::object_value;
use super::super::*;
use crate::core::hlsl::emit::Stage;
use crate::core::source::TagSource;
use crate::app::model_preview::common::math::{cross, unit_unless_under_micro};
use crate::app::model_preview::common::attached::attached;

const SMALLEST: f32 = 1e-4;
/// Halo 4's simulation rate.
const TICKS_PER_SECOND: f32 = 30.0;
/// An attached light's datum `+20`, which `light_new_attached` (X360
/// 0x84AC4B50) leaves at -1 and nothing on its path writes: its age and its
/// functions' input count from there, not from when it came on.
const ATTACHED_CREATION_TICK: f32 = -1.0;
/// `RenderSSLightInternal`'s angle falloff power, `c[4].w`, for every light.
const SPOT_POWER: f32 = 2.0;

/// `light` `flags`.
const FIRST_PERSON_ONLY: i128 = 1 << 2;
const ALWAYS_ON_FOR_WEAPON: i128 = 1 << 11;

/// `RenderSSLightInternal`'s technique bits: the place among
/// `MAKE_SS_LIGHT_TECHNIQUE`'s sixteen.
pub(crate) const TECHNIQUE_SPOT: usize = 4;
pub(crate) const TECHNIQUE_SPECULAR: usize = 8;

/// `rasterizer_globals`' explicit shaders the lights draw with.
const GOBO_SPECULAR: usize = 187;
const POINT_PHYSICALLY_CORRECT: usize = 204;

/// A light's function: `light_function`, its `Mapping` beside the names of
/// what drives it.
#[derive(Debug, Clone)]
struct H4LightFunction {
    input: String,
    range: String,
    function: Arc<TagFunction>,
}

impl H4LightFunction {
    fn read(root: TagStruct<'_>, name: &str) -> Option<Self> {
        let holder = root.descend(name)?;
        Some(Self {
            input: holder.read_string_id("Input Variable").unwrap_or_default(),
            range: holder.read_string_id("Range Variable").unwrap_or_default(),
            function: Arc::new(tag_function_at(&holder, "Mapping")?),
        })
    }

    /// `light_function::EvaluateFunction` (X360 0x84BCA490): a named input
    /// or range is looked up on object -1, which never answers here (no
    /// level, no interpolators), so a named input reads 0 and a named range
    /// keeps `range`; an unnamed input is `game seconds - at`.
    fn evaluate(&self, time: f32, at: f32, range: f32) -> f32 {
        let input = if self.input.is_empty() {
            time - at
        } else {
            0.0
        };
        self.function.evaluate(input, range)
    }

    fn animates(&self) -> bool {
        self.input.is_empty() && self.function.as_constant().is_none()
    }

    /// `light_function::IsDefault` (X360 0x84BCA4F8): an identity mapping
    /// that names neither input nor range.
    fn is_default(&self) -> bool {
        self.function.function_type() == blam_tags::FunctionType::Identity
            && self.input.is_empty()
            && self.range.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H4LightKind {
    Point,
    Spot,
    /// Not attached to objects in the kit, and not drawn.
    Directional,
}

/// A `light` tag, by field name: the kit's layouts differ from the
/// definitions' (`lights.md` §0).
#[derive(Debug, Clone)]
struct H4LightTag {
    kind: H4LightKind,
    /// `Lighting Mode` 0: shader 204.
    physical: bool,
    color: [f32; 3],
    target_color: [f32; 3],
    intensity: H4LightFunction,
    end: H4LightFunction,
    start: f32,
    /// Degrees, whole angle.
    inner: f32,
    outer: H4LightFunction,
    interpolation: Option<H4LightFunction>,
    fade: [f32; 2],
    screenspace: bool,
    specular: f32,
    diffuse: f32,
    first_person_only: bool,
    always_on_for_weapon: bool,
}

fn read_rgb(holder: TagStruct<'_>, name: &str) -> [f32; 3] {
    match holder.field(name).and_then(|field| field.value()) {
        Some(blam_tags::TagFieldData::RealRgbColor(color)) => [color.red, color.green, color.blue],
        _ => [0.0; 3],
    }
}

fn read_light_tag(source: &TagSource, path: &str) -> Option<H4LightTag> {
    let tag = load_referenced_tag_from_source(source, path, "light", b"ligh").ok()?;
    let root = tag.root().descend("Midnight_Light_Parameters")?;
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let int = |name: &str| root.read_int_any(name).unwrap_or(0);
    let flags = int("flags");
    Some(H4LightTag {
        kind: match int("Light Type") {
            1 => H4LightKind::Spot,
            2 => H4LightKind::Directional,
            // Area and sun draw as points (`MidnightLightFromDefinition`).
            _ => H4LightKind::Point,
        },
        physical: int("Lighting Mode") == 0,
        color: read_rgb(root, "Light Color"),
        target_color: read_rgb(root, "Target Color"),
        intensity: H4LightFunction::read(root, "Intensity")?,
        end: H4LightFunction::read(root, "Distance Attenuation End")?,
        start: real("Distance Attenuation Start"),
        inner: real("Inner Cone Angle"),
        outer: H4LightFunction::read(root, "Outer Cone End")?,
        interpolation: H4LightFunction::read(root, "Interpolation Function"),
        fade: [
            real("Camera Distance Fade Start"),
            real("Camera Distance Fade End"),
        ],
        screenspace: int("Screenspace Light") != 0,
        specular: int("Specular Contribution") as f32,
        diffuse: int("Diffuse Contribution") as f32,
        first_person_only: flags & FIRST_PERSON_ONLY != 0,
        always_on_for_weapon: flags & ALWAYS_ON_FOR_WEAPON != 0,
    })
}

/// A `light` at a marker of the object it is attached to
/// (`light_new_attached`, X360 0x84AC4B50).
#[derive(Debug, Clone)]
pub(crate) struct H4Light {
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
    only_under: Option<(String, String)>,
    /// The attachment's `primary scale`; the secondary never reaches a light.
    scale_by: String,
    force_on: bool,
    change_color: Option<usize>,
    /// A weapon on its own: `light_update_active_status` wants `always on
    /// for weapon`.
    lone_weapon: bool,
    tag: Arc<H4LightTag>,
}

/// A `cheap_light` at a marker (`render_object_attachments`' `gldf`).
#[derive(Debug, Clone)]
pub(crate) struct H4CheapLight {
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    only_under: Option<(String, String)>,
    scale_by: String,
    secondary_by: String,
    change_color: Option<usize>,
    /// `function input` 0: game time; 1 and 2 both the attachment's scale.
    on_time: bool,
    color: Arc<TagFunction>,
    intensity: Arc<TagFunction>,
    falloff_end: Arc<TagFunction>,
    begin_ratio: f32,
}

/// A light as one frame draws it: a `MidnightLight` on the draw list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct H4LitLight {
    pub(crate) physical: bool,
    pub(crate) spot: bool,
    pub(crate) specular: f32,
    pub(crate) diffuse: f32,
    pub(crate) position: [f32; 3],
    pub(crate) forward: [f32; 3],
    pub(crate) up: [f32; 3],
    pub(crate) color: [f32; 3],
    pub(crate) start: f32,
    pub(crate) end: f32,
    /// Radians, whole angles.
    pub(crate) outer: f32,
    pub(crate) inner: f32,
}



/// `light_color_too_dark` (X360 0x829ECEE8).
fn too_dark(color: [f32; 3]) -> bool {
    color.iter().map(|channel| channel.abs()).sum::<f32>() <= SMALLEST
}

fn tinted(color: [f32; 3], change_color: Option<usize>, change_colors: &[[f32; 3]; 4]) -> [f32; 3] {
    let tint = change_color
        .and_then(|channel| change_colors.get(channel))
        .copied()
        .unwrap_or([1.0; 3]);
    std::array::from_fn(|axis| color[axis] * tint[axis])
}

impl H4Light {
    pub(crate) fn inputs(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.scale_by.as_str())
    }

    pub(crate) fn animates(&self) -> bool {
        let tag = &self.tag;
        // A point's cone only gates it, and an open one stays open.
        let cone = tag.kind == H4LightKind::Spot && tag.outer.animates();
        cone || [&tag.intensity, &tag.end]
            .iter()
            .any(|function| function.animates())
            || tag
                .interpolation
                .as_ref()
                .is_some_and(|function| !function.is_default() && function.animates())
    }

    /// `light_update_state`, `MidnightLightFromDefinition` and
    /// `AddDatumLightToDrawList` for this frame at game seconds `time`, or
    /// none while it does not exist or gives no light.
    pub(crate) fn lit(
        &self,
        values: Option<&H2FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        time: f32,
        eye: [f32; 3],
    ) -> Option<H4LitLight> {
        let tag = &*self.tag;
        if tag.kind == H4LightKind::Directional || (self.lone_weapon && !tag.always_on_for_weapon) {
            return None;
        }
        // `attachments_update`: it exists only while its scale is on.
        let scale = object_value(values, &self.scale_by);
        if !self.force_on && !self.scale_by.is_empty() && scale <= 0.0 {
            return None;
        }
        // `light_update_state` (X360 0x84AC6788): the intensity on the
        // light's age in seconds, from game ticks less its creation tick,
        // through `EvaluateFunction` when it names a driver.
        let age = (time * TICKS_PER_SECOND - ATTACHED_CREATION_TICK) / TICKS_PER_SECOND;
        let named = !tag.intensity.input.is_empty() || !tag.intensity.range.is_empty();
        let mut intensity = if named {
            tag.intensity.evaluate(time, age, 0.5)
        } else {
            tag.intensity.function.evaluate(age, 0.5)
        };
        let rgb = tinted(tag.color, self.change_color, change_colors);
        if !self.scale_by.is_empty() {
            intensity *= scale;
        }
        let mut color = rgb.map(|channel| channel * intensity);
        // `MidnightLightFromDefinition` (X360 0x83AE3488): every other
        // function runs on `game seconds - (float)creation tick`, mixing
        // units as the code does, at range 1.
        let created = ATTACHED_CREATION_TICK;
        let end = tag.end.evaluate(time, created, 1.0);
        let outer = tag.outer.evaluate(time, created, 1.0).to_radians();
        if end <= SMALLEST || outer <= SMALLEST {
            return None;
        }
        // `EvaluateColorInterpolant` (X360 0x84BCA628): none while default.
        let interpolant = tag
            .interpolation
            .as_ref()
            .filter(|function| !function.is_default())
            .map_or(0.0, |function| function.evaluate(time, created, 1.0));
        if interpolant > 0.0 {
            let target = tag.intensity.evaluate(time, created, 0.5);
            color = std::array::from_fn(|axis| {
                (1.0 - interpolant) * color[axis] + interpolant * target * tag.target_color[axis]
            });
        }
        let spot = tag.kind == H4LightKind::Spot;
        // A spot that is neither screen space nor physically correct draws
        // forward, re-rendering what it lights (`RenderSpotlightInternal`),
        // which is not built; no kit object at rest has one.
        if spot && !tag.screenspace && !tag.physical {
            return None;
        }
        // `AddDatumLightToDrawList` (X360 0x82A27E10).
        if too_dark(color) {
            return None;
        }
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        let (_, up) = posed_on(self.node, self.position, self.up, bones);
        // `ApplyThrottles`: the camera fade, at a fade scale of 1.
        let [fade_start, fade_end] = tag.fade;
        let distance = (0..3)
            .map(|axis| (position[axis] - eye[axis]).powi(2))
            .sum::<f32>()
            .sqrt();
        if fade_end != 0.0 && distance >= fade_end {
            return None;
        }
        if distance > fade_start && fade_end > fade_start {
            let fade = 1.0 - (distance - fade_start) / (fade_end - fade_start);
            color = color.map(|channel| channel * fade);
        }
        Some(H4LitLight {
            physical: tag.physical,
            spot,
            specular: tag.specular,
            diffuse: tag.diffuse,
            position,
            forward: unit_unless_under_micro(forward),
            up: unit_unless_under_micro(up),
            color,
            start: tag.start.min(end),
            end,
            outer,
            inner: tag.inner.to_radians().min(outer),
        })
    }
}

impl H4CheapLight {
    pub(crate) fn inputs(&self) -> impl Iterator<Item = &str> {
        [&self.scale_by, &self.secondary_by]
            .into_iter()
            .map(String::as_str)
    }

    pub(crate) fn animates(&self) -> bool {
        self.on_time
            && [&self.color, &self.intensity, &self.falloff_end]
                .iter()
                .any(|function| function.as_constant().is_none())
    }

    /// `render_object_attachments`' cheap light this frame (X360
    /// 0x82AD1AE8, MCC 0x14090E933), then `AddCheapLightToDrawList` (X360
    /// 0x829ECB78): an artistic, diffuse-only point. The attachment's scale
    /// is its functions' input and its gate, never its brightness.
    pub(crate) fn lit(
        &self,
        values: Option<&H2FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        time: f32,
    ) -> Option<H4LitLight> {
        let scale = object_value(values, &self.scale_by) * object_value(values, &self.secondary_by);
        if scale <= 0.0 {
            return None;
        }
        let input = if self.on_time { time } else { scale };
        let color = self.color.evaluate_color(input, 1.0);
        let intensity = self.intensity.evaluate(input, 1.0);
        // `EvaluateFalloffEnd` (X360 0x82B121E8), unthrottled.
        let end = self.falloff_end.evaluate(input, 1.0);
        let color = tinted(
            [color.red, color.green, color.blue].map(|channel| channel * intensity),
            self.change_color,
            change_colors,
        );
        if too_dark(color) || end <= SMALLEST {
            return None;
        }
        let (position, _) = posed_on(self.node, self.position, [1.0, 0.0, 0.0], bones);
        Some(H4LitLight {
            physical: false,
            spot: false,
            specular: 0.0,
            diffuse: 1.0,
            position,
            forward: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            color,
            start: end * self.begin_ratio,
            end,
            outer: 2.0 * PI,
            inner: 2.0 * PI,
        })
    }
}

/// How a light is added into the light: `SetBlendReturnColor` (MCC): added
/// while any channel is above zero, else taken away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum H4LightBlend {
    Add,
    Subtract,
}

impl H4LitLight {
    /// The technique among the shader's sixteen: no gobo (none is read
    /// yet) and no shadow, the spot falloff while the cone closes, specular
    /// when asked for.
    pub(crate) fn technique(&self) -> usize {
        let mut technique = 0;
        if self.spot && (self.outer * 0.5).cos() >= SMALLEST {
            technique |= TECHNIQUE_SPOT;
        }
        if self.specular > SMALLEST {
            technique |= TECHNIQUE_SPECULAR;
        }
        technique
    }

    pub(crate) fn blend(&self) -> H4LightBlend {
        if self.color.iter().any(|channel| *channel > 0.0) {
            H4LightBlend::Add
        } else {
            H4LightBlend::Subtract
        }
    }

    /// `ps_screen_space_light_constants`' fourteen registers
    /// (`RenderSSLightInternal`, `lights.md` §2.2). `pixel_to_world` is the
    /// matrix that takes `(pixel x, pixel y, depth, 1)` to the world less
    /// the light; `exposure` is `ps_view_exposure.x`.
    pub(crate) fn constants(
        &self,
        pixel_to_world: &[[f32; 4]; 4],
        eye: [f32; 3],
        exposure: f32,
    ) -> [[f32; 4]; 14] {
        let (ratio, end) = if self.physical {
            // `ComputePhysicalRadius` (X360 0x829C21E0).
            let brightest = self
                .color
                .iter()
                .fold(0.0f32, |most, channel| most.max(channel.abs()));
            (1.0, (brightest / PI).sqrt())
        } else {
            (1.0 / (self.end - self.start), self.end)
        };
        let (outer, inner) = if self.spot {
            (self.outer, self.inner)
        } else {
            (2.0 * PI, 2.0 * PI)
        };
        let mut cutoff = (outer * 0.5).cos();
        let falloff_ratio = if cutoff < SMALLEST {
            cutoff = 0.0;
            f32::MAX
        } else {
            let spread = (inner * 0.5).cos() - cutoff;
            if spread > SMALLEST {
                1.0 / spread
            } else {
                f32::MAX
            }
        };
        let exposure = if self.physical {
            exposure / PI
        } else {
            exposure
        };
        let [r, g, b] = self.color.map(|channel| channel.abs() * exposure);
        // `BuildGoboRotationMatrix` (X360 0x83AE1508): a point's frame is the
        // world's axes; a spot's is its marker's.
        let (forward, up) = if self.spot {
            (self.forward, self.up)
        } else {
            ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0])
        };
        let left = cross(up, forward);
        let (a, bb) = if self.spot {
            let cot = 1.0 / (outer * 0.5).tan();
            (cot, cot)
        } else {
            (1.0, 1.0)
        };
        let row = |side: [f32; 3], by: f32, w: f32| {
            [
                0.5 * (forward[0] - side[0] * by),
                0.5 * (forward[1] - side[1] * by),
                0.5 * (forward[2] - side[2] * by),
                w,
            ]
        };
        let column = |at: usize| {
            [
                pixel_to_world[0][at],
                pixel_to_world[1][at],
                pixel_to_world[2][at],
                pixel_to_world[3][at],
            ]
        };
        [
            // `mul(clip, float4x4(c0..c3))`: each register a column.
            column(0),
            column(1),
            column(2),
            column(3),
            [r, g, b, SPOT_POWER],
            [end * ratio, -ratio, -cutoff * falloff_ratio, falloff_ratio],
            [
                self.position[0] - eye[0],
                self.position[1] - eye[1],
                self.position[2] - eye[2],
                0.0,
            ],
            row(up, a, 0.0),
            row(left, bb, 0.0),
            [forward[0], forward[1], forward[2], 0.0],
            [0.0; 4],
            [0.0; 4],
            [0.0; 4],
            [1.0, 0.0, self.specular, self.diffuse / PI],
        ]
    }
}

/// An object's attached `light`s and `cheap_light`s, at every marker of
/// each name, first-person-only lights left out of a third-person view.
/// `lone_weapon` is a weapon opened on its own.
pub(crate) fn read_h4_lights(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
    lone_weapon: bool,
) -> (Vec<H4Light>, Vec<H4CheapLight>) {
    let (mut lights, mut cheap) = (Vec::new(), Vec::new());
    let Some(block) = object_block(object, "attachments") else {
        return (lights, cheap);
    };
    let light_group = u32::from_be_bytes(*b"ligh");
    let cheap_group = u32::from_be_bytes(*b"gldf");
    let mut light_tags: HashMap<String, Option<Arc<H4LightTag>>> = HashMap::new();
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if path.is_empty() || (group != light_group && group != cheap_group) {
            continue;
        }
        let name = marker_name(attachment, "marker");
        let name = if name.is_empty() {
            "root".to_owned()
        } else {
            name
        };
        let scale_by = attachment
            .read_string_id("primary scale")
            .unwrap_or_default();
        let secondary_by = attachment
            .read_string_id("secondary scale")
            .unwrap_or_default();
        // The first option is none; the channels follow.
        let change_color = attachment
            .read_int_any("change color")
            .and_then(|color| usize::try_from(color - 1).ok());
        let force_on = attachment
            .read_flag_names("flags")
            .unwrap_or_default()
            .iter()
            .any(|(_, flag)| flag.starts_with("force always on"));
        if group == light_group {
            let tag = light_tags
                .entry(path.clone())
                .or_insert_with(|| read_light_tag(source, &path).map(Arc::new));
            let Some(tag) = tag.clone().filter(|tag| !tag.first_person_only) else {
                continue;
            };
            for marker in part_markers(markers, &name, false) {
                let (place, node) = frame(&marker);
                lights.push(H4Light {
                    object: owner,
                    node,
                    position: place.position,
                    forward: place.axes[0],
                    up: place.axes[2],
                    only_under: marker.only_under.clone(),
                    scale_by: scale_by.clone(),
                    force_on,
                    change_color,
                    lone_weapon,
                    tag: Arc::clone(&tag),
                });
            }
        } else if let Some(read) = read_cheap_light(source, &path) {
            for marker in part_markers(markers, &name, false) {
                let (place, node) = frame(&marker);
                cheap.push(H4CheapLight {
                    object: owner,
                    node,
                    position: place.position,
                    only_under: marker.only_under.clone(),
                    scale_by: scale_by.clone(),
                    secondary_by: secondary_by.clone(),
                    change_color,
                    ..read.clone()
                });
            }
        }
    }
    (lights, cheap)
}

fn read_cheap_light(source: &TagSource, path: &str) -> Option<H4CheapLight> {
    let tag = load_referenced_tag_from_source(source, path, "cheap_light", b"gldf").ok()?;
    let root = tag.root();
    let function = |name: &str| {
        root.descend(name)
            .and_then(|holder| tag_function_at(&holder, "Mapping"))
            .map(Arc::new)
    };
    Some(H4CheapLight {
        object: 0,
        node: 0,
        position: [0.0; 3],
        only_under: None,
        scale_by: String::new(),
        secondary_by: String::new(),
        change_color: None,
        on_time: root.read_int_any("function input").unwrap_or(0) == 0,
        color: function("color")?,
        intensity: function("intensity")?,
        falloff_end: function("Falloff End")?,
        // `cheap_light_postprocess` (X360 0x84AEFB98) clamps it.
        begin_ratio: root
            .read_real("Falloff Begin Ratio")
            .unwrap_or(0.0)
            .clamp(0.0, 1.0),
    })
}

/// The light passes a preview's lights can ask for, keyed by (physically
/// correct, technique): explicit 187 or 204's `tiny_position` pixel shader
/// over a full-screen triangle. A spot's cone may close to a point's, so a
/// spot brings both.
pub(crate) fn read_h4_light_passes(source: &TagSource, preview: &mut RenderModelPreview) {
    let mut wanted: Vec<(bool, usize)> = Vec::new();
    for light in &preview.h4_lights {
        let tag = &light.tag;
        let specular = if tag.specular > SMALLEST {
            TECHNIQUE_SPECULAR
        } else {
            0
        };
        wanted.push((tag.physical, specular));
        if tag.kind == H4LightKind::Spot {
            wanted.push((tag.physical, specular | TECHNIQUE_SPOT));
        }
    }
    if !preview.h4_cheap_lights.is_empty() {
        wanted.push((false, 0));
    }
    wanted.retain(|key| !preview.h4_light_passes.contains_key(key));
    if wanted.is_empty() {
        return;
    }
    let mut programs = HashMap::new();
    for key in wanted {
        let program = programs
            .entry(key.0)
            .or_insert_with(|| h4_light_program(source, key.0));
        let Some(program) = program.as_ref() else {
            continue;
        };
        if let Ok(pass) = h4_light_pass(program, key.1) {
            preview.h4_light_passes.insert(key, Arc::new(pass));
        }
    }
}

/// Explicit 204's effect when `physical`, else 187's, parsed.
pub(crate) fn h4_light_program(
    source: &TagSource,
    physical: bool,
) -> Option<crate::core::hlsl::parse::Program> {
    let globals = load_referenced_tag_from_source(
        source,
        "globals\\rasterizer_globals",
        "rasterizer_globals",
        b"rasg",
    )
    .ok()?;
    let index = if physical {
        POINT_PHYSICALLY_CORRECT
    } else {
        GOBO_SPECULAR
    };
    let explicit = elements(globals.root(), "explicit shaders");
    let path = explicit
        .get(index)?
        .read_tag_ref_path("explicit pixel shader")
        .filter(|path| !path.is_empty())?;
    let shader = super::shaders::H4ShaderSource::read(source, &path).ok()?;
    shader.program(&super::shaders::H4_DEFINES).ok()
}

/// A technique's `tiny_position` pixel shader over a full-screen triangle.
pub(crate) fn h4_light_pass(
    program: &crate::core::hlsl::parse::Program,
    technique: usize,
) -> Result<H3Pass, String> {
    let pixel =
        super::shaders::translate_technique(program, technique, "tiny_position", Stage::Pixel)?;
    Ok(H3Pass {
        fullscreen: true,
        vertex: super::super::materials::h4_screen_vertex().to_owned(),
        pixel: pixel.source,
        uniforms: pixel.uniforms,
        textures: pixel.textures,
    })
}

attached!(H4Light, H4CheapLight);

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(physical: bool, spot: bool) -> H4LitLight {
        H4LitLight {
            physical,
            spot,
            specular: 0.0,
            diffuse: 1.0,
            position: [1.0, 2.0, 3.0],
            forward: [0.0, 0.0, -1.0],
            up: [1.0, 0.0, 0.0],
            color: [2.0, 1.0, 0.5],
            start: 1.0,
            end: 5.0,
            outer: 60f32.to_radians(),
            inner: 30f32.to_radians(),
        }
    }

    fn kit() -> Option<TagSource> {
        let root = std::path::PathBuf::from(crate::core::test_kits::tag_path("halo4_mcc", ""));
        if !root.join("objects").is_dir() {
            eprintln!("skipped: set BLAM_TEST_H4EK to a Halo 4 kit's tags folder");
            return None;
        }
        Some(TagSource::LooseFolder {
            root,
            game: Some(crate::core::game::GameId::Halo4),
            definitions_root: crate::core::test_kits::definitions().to_path_buf(),
        })
    }

    fn attached(tag: H4LightTag) -> H4Light {
        H4Light {
            object: 0,
            node: 0,
            position: [0.0; 3],
            forward: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            only_under: None,
            scale_by: String::new(),
            force_on: false,
            change_color: None,
            lone_weapon: false,
            tag: Arc::new(tag),
        }
    }

    /// The forge's white light is a physically correct point that draws
    /// from the first frame: its outer cone is the identity on
    /// `game seconds + 1`, and its default colour interpolation leaves the
    /// colour alone.
    /// The Marines' monocle light, colour 0 in the kit, never draws.
    #[test]
    fn the_forge_light_draws_and_the_monocle_light_does_not() {
        let Some(source) = kit() else { return };
        let forge = read_light_tag(
            &source,
            "objects\\levels\\forge\\ff_light_white\\ff_light_white",
        )
        .unwrap();
        assert!(forge.physical && forge.kind == H4LightKind::Point);
        let lit = attached(forge.clone())
            .lit(None, &[[1.0; 3]; 4], None, 0.0, [3.0, 0.0, 0.0])
            .expect("the forge light");
        assert_eq!(lit.color, [80.0; 3]);
        assert_eq!(lit.end, 100.0);
        assert!(
            (lit.outer - 1f32.to_radians()).abs() < 1e-6,
            "{}",
            lit.outer
        );
        let monocle =
            read_light_tag(&source, "objects\\characters\\marine\\fx\\monocle_light").unwrap();
        assert!(
            attached(monocle)
                .lit(None, &[[1.0; 3]; 4], None, 0.0, [3.0, 0.0, 0.0])
                .is_none()
        );
        // A weapon on its own wants `always on for weapon`, which it lacks.
        let lone = H4Light {
            lone_weapon: true,
            ..attached(forge)
        };
        assert!(
            lone.lit(None, &[[1.0; 3]; 4], None, 0.0, [3.0, 0.0, 0.0])
                .is_none()
        );
    }

    /// The Jackal's sniper-eye cheap light runs on its attachment's scale
    /// (`alive`), off while that is 0, and starts falling off two thirds of
    /// the way out.
    #[test]
    fn the_jackal_eye_light_runs_on_its_scale() {
        let Some(source) = kit() else { return };
        let light = H4CheapLight {
            scale_by: "alive".to_owned(),
            ..read_cheap_light(&source, "objects\\characters\\storm_jackal\\fx\\sniper_eye")
                .unwrap()
        };
        assert!(!light.on_time);
        assert!(
            light.lit(None, &[[1.0; 3]; 4], None, 0.0).is_none(),
            "a scale no one answers reads 0"
        );
        let mut values = H2FunctionValues::default();
        values.insert("alive".to_owned(), 1.0);
        let lit = light
            .lit(Some(&values), &[[1.0; 3]; 4], None, 0.0)
            .expect("lit while alive");
        assert!(!lit.physical && lit.diffuse == 1.0 && lit.specular == 0.0);
        assert!(
            (lit.start - lit.end * 0.66).abs() < 1e-5,
            "{} {}",
            lit.start,
            lit.end
        );
    }

    const IDENTITY: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    /// The artistic falloff `saturate(c5.x + d·c5.y)` is 1 at the start and
    /// 0 at the end; a spot's `saturate(dot·c5.w + c5.z)` is 1 inside the
    /// inner cone and 0 at the outer.
    #[test]
    fn an_artistic_light_falls_off_from_its_start_to_its_end_and_across_its_cone() {
        let c = lit(false, true).constants(&IDENTITY, [0.0; 3], 0.5);
        let falloff = |d: f32| (c[5][0] + d * c[5][1]).clamp(0.0, 1.0);
        assert!(
            (falloff(1.0) - 1.0).abs() < 1e-5
                && falloff(5.0).abs() < 1e-5
                && (falloff(3.0) - 0.5).abs() < 1e-5
        );
        let cone = |angle: f32| (angle.to_radians().cos() * c[5][3] + c[5][2]).clamp(0.0, 1.0);
        assert!(
            (cone(15.0) - 1.0).abs() < 1e-4 && cone(30.0).abs() < 1e-4,
            "{} {}",
            cone(15.0),
            cone(30.0)
        );
        assert_eq!(
            &c[4],
            &[1.0, 0.5, 0.25, SPOT_POWER],
            "the colour takes the exposure"
        );
        assert_eq!(&c[6][..3], &[1.0, 2.0, 3.0], "camera to light");
        assert_eq!(
            &c[9][..3],
            &[0.0, 0.0, -1.0],
            "a spot shines along its marker"
        );
        assert_eq!(c[13], [1.0, 0.0, 0.0, 1.0 / PI]);
    }

    /// Shader 204 ends where `sqrt(max|rgb| / π)` says, and its colour takes
    /// a further `1/π`; a point has no cone, whatever its tag's angles.
    #[test]
    fn a_physically_correct_point_ends_at_its_physical_radius() {
        let light = lit(true, false);
        let c = light.constants(&IDENTITY, [0.0; 3], 0.5);
        assert!((c[5][0] - (2.0 / PI).sqrt()).abs() < 1e-6);
        assert!((c[4][0] - 1.0 / PI).abs() < 1e-6);
        assert_eq!((c[5][2], c[5][3]), (0.0, f32::MAX), "no cutoff");
        assert_eq!(&c[9][..3], &[1.0, 0.0, 0.0], "the world's forward");
        assert_eq!(light.technique(), 0);
        assert_eq!(lit(false, true).technique(), TECHNIQUE_SPOT);
        let wide = H4LitLight {
            outer: PI,
            specular: 1.0,
            ..lit(false, true)
        };
        assert_eq!(
            wide.technique(),
            TECHNIQUE_SPECULAR,
            "a cone open to a half space has no spot falloff"
        );
    }

    /// The pixel-to-world matrix goes in as columns, as `mul(clip, M)` reads
    /// its rows of `c0..c3`.
    #[test]
    fn the_pixel_to_world_registers_are_its_columns() {
        let mut matrix = IDENTITY;
        matrix[0][3] = 7.0;
        let c = lit(false, false).constants(&matrix, [0.0; 3], 1.0);
        assert_eq!(c[3], [7.0, 0.0, 0.0, 1.0]);
        assert_eq!(c[0], [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_light_adds_while_any_channel_is_lit_and_takes_away_otherwise() {
        assert_eq!(lit(false, false).blend(), H4LightBlend::Add);
        let dark = H4LitLight {
            color: [-1.0, 0.0, -0.5],
            ..lit(false, false)
        };
        assert_eq!(dark.blend(), H4LightBlend::Subtract);
        assert!(too_dark([0.00003, -0.00003, 0.00003]));
        assert!(!too_dark([0.0, -0.001, 0.0]));
    }
}
