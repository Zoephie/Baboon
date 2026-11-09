//! Halo 2's sphere and frustum (gel) lights: read, and lit for a frame.

use super::super::super::TagReader;
use super::flares::{H2Flare, H2FlareAt, read_h2_flare};
use super::place::{
    BRIGHTEST, FIELD_OF_VIEW, NEAR_EDGE_BIAS, NEAR_EDGE_DIVISOR, light_value, scale_function,
    second_field_int,
};
use super::{
    Dx9Layer, Dx9Light, FunctionValues, H2EffectLight, Image, SamplerKind, UNSET, attached, cross,
    decode_image, has_flag, posed_on, rgb_colors_interpolate, rgb_interpolation_flags,
};
use blam_tags::{TagFunction, TagStruct};
use std::sync::Arc;

/// What `sub_8ED2A0` works a `light` out from, for the flare it submits.
#[derive(Debug, Clone)]
pub(in crate::app::model_preview::h2) struct H2Light {
    pub(super) interpolation: i128,
    pub(super) size: [f32; 2],
    pub(super) specular: [[f32; 3]; 2],
    pub(super) diffuse: [[f32; 3]; 2],
    pub(super) brightness: [f32; 2],
    pub(super) color_animation: Option<Arc<TagFunction>>,
    pub(super) brightness_animation: Option<Arc<TagFunction>>,
    /// The diffuse pass's variant its `diffuse contrast` picks.
    pub(super) diffuse_layer: Dx9Layer,
    pub(super) radius: f32,
    pub(super) specular_radius: f32,
    /// `duration`: how long it lasts when an effect makes it, or 0 for ever.
    pub(in crate::app::model_preview::h2) effect_duration: f32,
    /// The second `falloff function`: how it dims over that duration.
    pub(in crate::app::model_preview::h2) effect_falloff: u8,
}

/// A sphere light at the marker an object attaches it to, lighting the model.
#[derive(Debug, Clone)]
pub(crate) struct H2PointLight {
    /// Which object's functions drive it; 0 is the opened.
    pub(crate) object: u8,
    pub(super) node: usize,
    pub(super) position: [f32; 3],
    /// `primary scale`, a name the object's functions answer to.
    pub(super) scale_by: String,
    pub(super) light: H2Light,
    pub(super) effect: Option<H2EffectLight>,
    /// The region and permutation its marker belongs to, if only one.
    pub(super) only_under: Option<(String, String)>,
}

/// A frustum's shape.
#[derive(Debug, Clone)]
pub(super) struct H2Frustum {
    pub(super) near_width: f32,
    /// `height stretch` by the gel's aspect.
    pub(super) stretch: f32,
    pub(super) half_view: f32,
    pub(super) falloff: f32,
    pub(super) cutoff: f32,
    pub(super) gel: Option<Arc<Image>>,
}

/// A frustum light at its marker, which only a gel pass draws.
#[derive(Debug, Clone)]
pub(crate) struct H2GelLight {
    /// Which object's functions drive it; 0 is the opened.
    pub(crate) object: u8,
    pub(super) node: usize,
    pub(super) position: [f32; 3],
    pub(super) forward: [f32; 3],
    pub(super) up: [f32; 3],
    /// `primary scale`, a name the object's functions answer to.
    pub(super) scale_by: String,
    pub(super) light: H2Light,
    pub(super) frustum: H2Frustum,
    pub(super) effect: Option<H2EffectLight>,
    /// The region and permutation its marker belongs to, if only one.
    pub(super) only_under: Option<(String, String)>,
}

/// The vectors `lightaccum_frustum_no_specular_gel` reads as c33 to c36, in a
/// light's frame of `sub_946960`. All but `apex` take the vector to the light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H2GelRows {
    pub(crate) apex: [f32; 3],
    pub(crate) rows: [[f32; 4]; 4],
}

/// Whether a `light` can ever be on in a third person view: nothing else sets
/// the bit `sub_8ED2A0` turns a light on through.
pub(in crate::app::model_preview) fn light_can_shine(root: TagStruct<'_>) -> bool {
    let flag = |option: &str| has_flag(root, "flags", option);
    // `default lightmap setting`'s `lightmaps only`: the game makes no such light.
    !flag("no illumination")
        && !flag("only render in first person")
        && !root
            .field("default lightmap setting")
            .is_some_and(|field| field.enum_is("lightmaps only"))
}

/// `sub_8EB720`, and what tool's postprocess gives a tag for the game: the
/// light, whether it lights the model on the sphere path, and its flare. `None`
/// for a light the game makes nothing of in a third person view.
pub(super) fn read_h2_light(
    reader: &TagReader,
    path: &str,
) -> Option<(H2Light, bool, Option<H2Frustum>, Option<Arc<H2Flare>>)> {
    let tag = reader.load(path, "light", b"ligh").ok()?;
    let root = tag.root();
    let flag = |option: &str| has_flag(root, "flags", option);
    if !light_can_shine(root) {
        return None;
    }
    let flare = root
        .read_tag_ref_path("lens flare")
        .filter(|path| !path.is_empty());
    let flare = flare
        .and_then(|path| read_h2_flare(reader, &path))
        .map(Arc::new);
    let rgb = |name: &str| {
        let color = root.read_rgb(name);
        [color.red, color.green, color.blue]
    };
    let unset = |colors: &[[f32; 3]; 2]| colors.iter().flatten().all(|channel| *channel < UNSET);
    let size = root.read_real_bounds("size modifer");
    let size = if size.lower < UNSET && size.upper < UNSET {
        [1.0, 1.0]
    } else {
        [size.lower, size.upper]
    };
    let mut diffuse = [rgb("diffuse lower bound"), rgb("diffuse upper bound")];
    if unset(&diffuse) {
        diffuse = [[1.0; 3]; 2];
    }
    let mut specular = [rgb("specular lower bound"), rgb("specular upper bound")];
    if flag("no specular") {
        specular = [[0.0; 3]; 2];
    } else if unset(&specular) {
        specular = diffuse;
    }
    let brightness = root.read_real_bounds("brightness bounds");
    let brightness = if brightness.lower == 0.0 && brightness.upper == 0.0 {
        [1.0, 1.0]
    } else {
        [brightness.lower, brightness.upper]
    };
    let radius = root.read_real("radius").unwrap_or(0.0);
    let specular_radius = root
        .read_real("specular radius")
        .filter(|radius| *radius != 0.0);
    let saturate = |colors: [[f32; 3]; 2]| colors.map(|color| color.map(|c| c.clamp(0.0, 1.0)));
    let light = H2Light {
        interpolation: rgb_interpolation_flags(
            has_flag(root, "interpolation flags", "blend in hsv"),
            has_flag(root, "interpolation flags", "...more colors"),
        ),
        size,
        specular: saturate(specular),
        diffuse: saturate(diffuse),
        brightness,
        color_animation: scale_function(root, "color animation"),
        brightness_animation: scale_function(root, "brightness animation"),
        diffuse_layer: Dx9Layer::diffuse_with_contrast(
            root.read_enum_option("diffuse contrast").as_deref(),
        ),
        radius,
        specular_radius: specular_radius.unwrap_or(radius),
        effect_duration: root.read_real("duration").unwrap_or(0.0),
        effect_falloff: second_field_int(root, "falloff function")
            .and_then(|index| u8::try_from(index).ok())
            .unwrap_or(0),
    };
    // `sub_6BA660` leaves the object unlit only with both.
    let lit = !(flag("ignore parent object") && flag("ignore all parents"));
    // Implementation 1 has no gel to read, so a sphere's own never shows.
    // A sphere is the only type that takes the sphere path.
    let kind = |option: &str| {
        root.field("type")
            .is_some_and(|field| field.enum_is(option))
    };
    let lights_model = kind("sphere") && lit;
    let frustum = lit.then(|| read_h2_frustum(reader, root)).flatten();
    Some((light, lights_model, frustum, flare))
}

/// The shape `sub_8EB100` and `sub_8E84F0` read from a frustum `light`, with
/// tool's postprocess applied. A shadow's pass needs a map of the level from
/// the light, which a preview has none of, so it lights without one.
fn read_h2_frustum(reader: &TagReader, root: TagStruct<'_>) -> Option<H2Frustum> {
    let is =
        |field: &str, option: &str| root.field(field).is_some_and(|field| field.enum_is(option));
    // Orthogonal lights assert in `sub_946960`, so these are the frustums;
    // tool's postprocess makes `directional` of a frustum's default falloff.
    let (projective, pyramid) = (is("type", "projective"), is("type", "pyramid"));
    let falloff = is("falloff geometry", "default") || is("falloff geometry", "directional");
    if !(projective || pyramid) || !falloff {
        return None;
    }
    let gel = root
        .read_tag_ref_path("gel map")
        .filter(|gel| !gel.is_empty());
    let gel = gel
        .and_then(|gel| decode_image(reader, &gel, SamplerKind::Texture2D, 0))
        .map(Arc::new);
    let aspect = gel.as_deref().map_or(1.0, |image| match image {
        Image::Flat(image) => image.height as f32 / image.width as f32,
        _ => 1.0,
    });
    let stretch = root
        .read_real("height stretch")
        .filter(|stretch| *stretch != 0.0)
        .unwrap_or(1.0);
    let [narrowest, widest] = FIELD_OF_VIEW;
    let view = root
        .read_real("field of view")
        .unwrap_or(0.0)
        .clamp(narrowest, widest);
    Some(H2Frustum {
        near_width: if pyramid {
            0.0
        } else {
            root.read_real("near width").unwrap_or(0.0)
        },
        stretch: stretch * aspect,
        half_view: view.to_radians() * 0.5,
        falloff: root.read_real("falloff distance").unwrap_or(0.0),
        cutoff: root.read_real("cutoff distance").unwrap_or(0.0),
        gel,
    })
}

impl H2Light {
    /// `sub_8ED2A0`'s color for `value`, or none while the light is out. Both
    /// bounds' alphas are 1 here, so a change color adds nothing.
    pub(super) fn diffuse_at(&self, value: f32, time: f32) -> Option<[f32; 3]> {
        let at = |[lower, upper]: [[f32; 3]; 2]| {
            rgb_colors_interpolate(self.interpolation, lower, upper, value)
        };
        let (mut diffuse, specular) = (at(self.diffuse), at(self.specular));
        if let Some(function) = &self.color_animation {
            let color = function.evaluate_color(time, 0.0);
            let scale = [color.red, color.green, color.blue];
            diffuse = std::array::from_fn(|channel| diffuse[channel] * scale[channel]);
        }
        let diffuse = diffuse.map(|channel| channel.clamp(0.0, 1.0));
        let specular = specular.map(|channel| channel.clamp(0.0, 1.0));
        let size = (1.0 - value) * self.size[0] + value * self.size[1];
        let lit = diffuse
            .iter()
            .chain(&specular)
            .any(|channel| *channel != 0.0);
        (size > 0.0 && lit).then_some(diffuse)
    }

    /// `sub_8ED2A0`'s brightness for `value`.
    fn shining(&self, value: f32, time: f32) -> f32 {
        let [dim, bright] = self.brightness;
        let mut brightness = (1.0 - value) * dim + value * bright;
        if let Some(function) = &self.brightness_animation {
            brightness *= function.evaluate(time, 0.0);
        }
        (brightness * 2.0).clamp(0.0, BRIGHTEST)
    }
}

impl H2PointLight {
    /// The layer its diffuse draws on.
    pub(crate) fn diffuse_layer(&self) -> Dx9Layer {
        self.light.diffuse_layer
    }

    /// The function that scales it, which sliders and frames must answer for.
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// Whether it changes with nothing but time.
    pub(crate) fn animates(&self) -> bool {
        self.light.color_animation.is_some() || self.light.brightness_animation.is_some()
    }

    /// What `sub_947870` hands the passes this frame, if the light is on.
    /// `onset` is when its function last came on, for an effect's light.
    pub(crate) fn lit(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        time: f32,
        onset: f32,
    ) -> Option<Dx9Light> {
        let value = light_value(
            &self.light,
            self.effect.as_ref(),
            &self.scale_by,
            values,
            time,
            onset,
        )?;
        let light = &self.light;
        let diffuse = light.diffuse_at(value, time)?;
        let specular = rgb_colors_interpolate(
            light.interpolation,
            light.specular[0],
            light.specular[1],
            value,
        );
        let brightness = light.shining(value, time);
        let size = (1.0 - value) * light.size[0] + value * light.size[1];
        let radius = (light.radius * size).max(light.specular_radius * size);
        if radius <= UNSET {
            return None;
        }
        let (position, _) = posed_on(self.node, self.position, [1.0, 0.0, 0.0], bones);
        Some(Dx9Light {
            position,
            color: diffuse.map(|channel| (channel * brightness).min(BRIGHTEST)),
            specular: specular.map(|channel| (channel * brightness).min(BRIGHTEST)),
            radius,
        })
    }
}

impl H2Frustum {
    /// `sub_8E84F0` and `sub_946960` at `size`, for a marker's `position`.
    /// `falloff geometry` directional; gel animation rests at no offset.
    pub(super) fn rows(
        &self,
        position: [f32; 3],
        forward: [f32; 3],
        up: [f32; 3],
        size: f32,
    ) -> H2GelRows {
        let tan_x = self.half_view.tan();
        let tan_y = tan_x * self.stretch;
        let (falloff, cutoff) = (self.falloff * size, self.cutoff * size);
        let behind = self.near_width / tan_x * 0.5;
        let apex = std::array::from_fn(|axis| position[axis] - forward[axis] * behind);
        let left = cross(forward, up);
        let scale = -1.0 / (cutoff - falloff);
        let across = |axis: usize| (forward[axis] - left[axis] / tan_x) * scale * 0.5;
        let along = |axis: usize| (forward[axis] + up[axis] / tan_y) * scale * 0.5;
        let reach = forward.map(|axis| axis * scale);
        let edge = (cutoff - falloff) * 0.5 / NEAR_EDGE_DIVISOR;
        H2GelRows {
            apex,
            rows: [
                [across(0), across(1), across(2), 0.0],
                [along(0), along(1), along(2), 0.0],
                [reach[0], reach[1], reach[2], (behind + falloff) * scale],
                [
                    reach[0] * edge,
                    reach[1] * edge,
                    reach[2] * edge,
                    edge * scale * (behind + NEAR_EDGE_BIAS) + 0.5,
                ],
            ],
        }
    }
}

impl H2GelLight {
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    pub(crate) fn animates(&self) -> bool {
        self.light.brightness_animation.is_some() || self.light.color_animation.is_some()
    }

    pub(crate) fn gel(&self) -> Option<&Arc<Image>> {
        self.frustum.gel.as_ref()
    }

    /// The light for a gel pass, and its frustum there. None while it's out.
    pub(crate) fn lit(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        time: f32,
        onset: f32,
    ) -> Option<(Dx9Light, H2GelRows)> {
        let value = light_value(
            &self.light,
            self.effect.as_ref(),
            &self.scale_by,
            values,
            time,
            onset,
        )?;
        let light = &self.light;
        let diffuse = light.diffuse_at(value, time)?;
        let size = (1.0 - value) * light.size[0] + value * light.size[1];
        let brightness = light.shining(value, time);
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        let (_, up) = posed_on(self.node, self.position, self.up, bones);
        let rows = self.frustum.rows(position, forward, up, size);
        let color = diffuse.map(|channel| (channel * brightness).min(BRIGHTEST));
        let specular = rgb_colors_interpolate(
            light.interpolation,
            light.specular[0],
            light.specular[1],
            value,
        );
        let specular = specular.map(|channel| (channel * brightness).min(BRIGHTEST));
        Some((
            Dx9Light {
                position: rows.apex,
                color,
                specular,
                radius: 1.0,
            },
            rows,
        ))
    }
}

/// `rasterizer_render_light`'s flag 1: whether a frustum takes implementation 0.
pub(crate) fn has_specular(light: &Dx9Light) -> bool {
    light.specular.iter().any(|channel| *channel > 0.01)
}

/// `sub_946960`: a color past 1 goes to the pass halved or quartered, for a
/// variant that scales its output back up, as passes saturate on the way. The
/// preview keeps the first variant: the same pass drawn again is the same sum.
pub(crate) fn overbrighten(light: &Dx9Light) -> (Dx9Light, usize) {
    let brightest = light
        .color
        .iter()
        .chain(&light.specular)
        .copied()
        .fold(0.0, f32::max);
    let (scale, draws) = match brightest {
        value if value <= 1.0 => (1.0, 1),
        value if value <= 2.0 => (0.5, 2),
        _ => (0.25, 4),
    };
    let scaled = |color: [f32; 3]| color.map(|channel| (channel * scale).clamp(0.0, 1.0));
    let dimmed = Dx9Light {
        color: scaled(light.color),
        specular: scaled(light.specular),
        ..*light
    };
    (dimmed, draws)
}

attached!(H2PointLight, H2GelLight, H2FlareAt);
