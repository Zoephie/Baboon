//! Halo 2's lens flares: their reflections, how they are read, and the sprites and occlusion they draw.

use super::*;

/// A `lens_flare`'s `flags`, by name.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct FlareFlags {
    pub(super) no_occlusion_test: bool,
    pub(super) only_first_person: bool,
    pub(super) scale_by_marker: bool,
}

/// A reflection's `flags`, by name.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ReflectionFlags {
    pub(super) align_with_screen_center: bool,
    pub(super) radius_not_scaled_by_distance: bool,
    pub(super) radius_scaled_by_occlusion: bool,
    pub(super) occluded_by_solid_objects: bool,
    pub(super) ignore_light_color: bool,
    pub(super) not_affected_by_inner_occlusion: bool,
}

/// One sprite of a flare, set along the axis through the screen's center.
#[derive(Debug, Clone)]
pub(super) struct H2Reflection {
    pub(super) flags: ReflectionFlags,
    /// Which of [`H2Flare::images`] it draws.
    pub(super) image: usize,
    pub(super) position: f32,
    pub(super) rotation: f32,
    pub(super) radius: [f32; 2],
    pub(super) brightness: [f32; 2],
    pub(super) modulation: f32,
    pub(super) color: [f32; 3],
}

/// A `lens_flare` tag.
#[derive(Debug, Clone)]
pub(crate) struct H2Flare {
    pub(super) flags: FlareFlags,
    /// Cosines of its `falloff angle` and `cutoff angle`.
    pub(super) cones: [f32; 2],
    pub(super) fade: [f32; 2],
    pub(super) occlusion_radius: f32,
    pub(super) occlusion_offset: OcclusionOffset,
    /// The share of the outer patch a second test takes, if there is one.
    pub(super) inner_scale: Option<f32>,
    pub(super) corona_scale: [f32; 2],
    pub(super) rotation_function: i128,
    pub(super) rotation_scale: f32,
    pub(super) reflections: Vec<H2Reflection>,
    pub(super) brightness: Option<Arc<TagFunction>>,
    pub(super) color: Option<Arc<TagFunction>>,
    pub(super) rotation: Option<Arc<TagFunction>>,
    /// The animation's `synchronized`: every instance runs on the same clock.
    /// Without it, `lens_flare_draw_callback` shifts each one's.
    pub(super) synchronized: bool,
    /// `falloff function`: how an effect's flare dims over its event.
    pub(super) falloff: u8,
    pub(crate) images: Vec<Arc<H2Image>>,
}

/// What makes a flare that is no attachment of its own: a light an effect
/// makes, or an effect's `lens_flare` part.
#[derive(Debug, Clone)]
pub(super) enum H2FlareEffect {
    Light(H2EffectLight),
    Part(H2EffectPart),
}

/// A flare at one of the markers an object attaches it to.
#[derive(Debug, Clone)]
pub(crate) struct H2FlareAt {
    /// Which object's functions drive it; 0 is the opened.
    pub(crate) object: u8,
    pub(super) node: usize,
    pub(super) position: [f32; 3],
    pub(super) forward: [f32; 3],
    pub(super) marker_scale: f32,
    /// The region and permutation its marker belongs to, if only one.
    pub(super) only_under: Option<(String, String)>,
    /// `primary scale`, a name the object's functions answer to.
    pub(super) scale_by: String,
    /// `change color`: which of the four, if any.
    pub(super) change_color: Option<usize>,
    /// The light it belongs to, if it is no attachment of its own.
    pub(super) light: Option<H2Light>,
    pub(super) effect: Option<H2FlareEffect>,
    pub(crate) flare: Arc<H2Flare>,
}

impl H2FlareAt {
    /// `lens_flare_draw_callback` for an unsynchronized flare: seconds ahead,
    /// the first byte of a hash over 256 and the second whole. The game hashes
    /// the address of the flare's draw record; the preview hashes where the
    /// flare sits, which is as fixed for it and differs between flares.
    fn unsynchronized_offset(&self) -> f32 {
        let seed = self.position.iter().fold(self.node as u32, |seed, axis| {
            seed.rotate_left(5) ^ axis.to_bits()
        });
        let hashed = lcg_step(seed).to_le_bytes();
        f32::from(hashed[0]) / 256.0 + f32::from(hashed[1])
    }
}

pub(super) fn read_h2_flare(reader: &TagReader, path: &str) -> Option<H2Flare> {
    let tag = reader.load(path, "lens_flare", b"lens").ok()?;
    let root = tag.root();
    let bitmap = root.read_tag_ref_path("bitmap").filter(|bitmap| !bitmap.is_empty())?;
    let mut images = Vec::new();
    let mut indices = Vec::new();
    let mut reflections = Vec::new();
    for reflection in elements(root, "reflections") {
        let index = reflection.read_int_any("bitmap index").unwrap_or(0).max(0) as usize;
        let image = indices.iter().position(|known| *known == index).or_else(|| {
            let image = decode_h2_image(reader, &bitmap, SamplerKind::Texture2D, index)?;
            images.push(Arc::new(image));
            indices.push(index);
            Some(images.len() - 1)
        })?;
        let radius = reflection.read_real_bounds("radius");
        let brightness = reflection.read_fraction_bounds("brightness");
        let color = reflection.read_rgb("color");
        reflections.push(H2Reflection {
            flags: {
                let set = |option: &str| has_flag(reflection, "flags", option);
                ReflectionFlags {
                    align_with_screen_center: set("align rotation with screen center"),
                    radius_not_scaled_by_distance: set("radius NOT scaled by distance"),
                    radius_scaled_by_occlusion: set("radius scaled by occlusion factor"),
                    occluded_by_solid_objects: set("occluded by solid objects"),
                    ignore_light_color: set("ignore light color"),
                    not_affected_by_inner_occlusion: set("not affected by inner occlusion"),
                }
            },
            image,
            position: reflection.read_real("position").unwrap_or(0.0),
            rotation: reflection.read_real("rotation offset").unwrap_or(0.0).to_radians(),
            radius: [radius.lower, radius.upper],
            brightness: [brightness.lower, brightness.upper],
            modulation: reflection.read_real("modulation factor").unwrap_or(0.0),
            color: [color.red, color.green, color.blue],
        });
    }
    let angle = |name: &str| root.read_real(name).unwrap_or(0.0).cos();
    let corona = root.read_vec2("corona scale");
    let inner = root.read_int_any("occlusion inner radius scale").unwrap_or(0).clamp(0, 6) as usize;
    let unset_as_one = |scale: f32| if scale == 0.0 { 1.0 } else { scale };
    Some(H2Flare {
        flags: FlareFlags {
            no_occlusion_test: has_flag(root, "flags", "no occlusion test"),
            only_first_person: has_flag(root, "flags", "only render in first person"),
            scale_by_marker: has_flag(root, "flags", "scale by marker"),
        },
        cones: [angle("falloff angle"), angle("cutoff angle")],
        fade: [
            root.read_real("near fade distance").unwrap_or(0.0),
            root.read_real("far fade distance").unwrap_or(0.0),
        ],
        occlusion_radius: root.read_real("occlusion radius").unwrap_or(0.0),
        occlusion_offset: OcclusionOffset::read(root),
        inner_scale: (inner != 0).then_some(INNER_SCALES[inner]),
        corona_scale: [unset_as_one(corona.i), unset_as_one(corona.j)],
        rotation_function: root.read_int_any("rotation function").unwrap_or(0),
        rotation_scale: root.read_real("rotation function scale").unwrap_or(0.0),
        reflections,
        brightness: scale_function(root, "brightness"),
        color: scale_function(root, "color"),
        rotation: scale_function(root, "rotation"),
        // Its second `flags` field holds it.
        synchronized: has_flag_in_any(root, "flags", "synchronized"),
        falloff: root.read_int_any("falloff function").and_then(|index| u8::try_from(index).ok()).unwrap_or(0),
        images,
    })
}

fn from_view(view: &ViewFrame, point: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| point[axis] - view.eye[axis])
}

/// `sub_92D350`: a corona's turn for how the marker sits against the view, in
/// units of `atan2 * scale / pi`. The game's third view axis points back at the
/// viewer's side as `-forward`, as Halo CE's does.
fn corona_rotation(
    function: i128,
    scale: f32,
    to_light: [f32; 3],
    forward: [f32; 3],
    view: &ViewFrame,
) -> f32 {
    let turned = cross(cross(forward, view.right), forward);
    let back = view.forward.map(|axis| -axis);
    let (across, along) = match function {
        1 => (dot(turned, view.forward), dot(forward, view.forward)),
        2 => {
            let behind = forward.map(|axis| -axis);
            (dot(behind, view.right), dot(behind, back))
        }
        3 => (dot(to_light, turned), dot(to_light, forward)),
        4 => (dot(to_light, view.right), dot(to_light, back)),
        _ => return 0.0,
    };
    if across == 0.0 {
        return 0.0;
    }
    across.atan2(-along) * scale / std::f32::consts::PI
}

impl H2Flare {
    /// Whether it changes with nothing but time.
    pub(crate) fn animates(&self) -> bool {
        self.brightness.is_some() || self.color.is_some() || self.rotation.is_some()
    }

    fn tests_occlusion(&self) -> bool {
        !self.flags.no_occlusion_test
    }
}

impl H2FlareAt {
    #[cfg(test)]
    pub(crate) fn effect_part(&self) -> Option<&H2EffectPart> {
        match &self.effect {
            Some(H2FlareEffect::Part(part)) => Some(part),
            _ => None,
        }
    }

    /// The function that scales it, which sliders and frames must answer for.
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// Whether it changes with nothing but time.
    pub(crate) fn animates(&self) -> bool {
        let light = self.light.as_ref().is_some_and(|light| light.color_animation.is_some());
        light || self.flare.animates()
    }

    fn posed(&self, bones: Option<&[[f32; 4]]>) -> ([f32; 3], [f32; 3]) {
        posed_on(self.node, self.position, self.forward, bones)
    }

    /// A light passes `lens_flare_submit` a scale of 1, whatever its marker's.
    fn marker_scaled(&self) -> f32 {
        let by_marker = self.flare.flags.scale_by_marker && self.light.is_none();
        if by_marker { self.marker_scale } else { 1.0 }
    }

    pub(super) fn value(&self, values: Option<&FunctionValues>, time: f32, onset: f32) -> Option<f32> {
        match (&self.effect, &self.light) {
            (Some(H2FlareEffect::Light(effect)), Some(light)) => {
                effect.value(light, attachment_value(&self.scale_by, values), time, onset)
            }
            // `sub_515280`: dimmed by the flare's falloff over its event.
            (Some(H2FlareEffect::Part(part)), _) => {
                let lit = part.light(&self.scale_by, values, time, onset)?;
                let dimmed = || 1.0 - transition_evaluate(self.flare.falloff, 1.0 - lit.fade);
                let falloff = if lit.lasts { dimmed() } else { 1.0 };
                Some(lit.brightness * falloff)
            }
            _ => Some(attachment_value(&self.scale_by, values)),
        }
    }

    /// The patches `lens_flare_occlusion_callback` tests: the outer, and an
    /// inner one chained under it. None for a flare that takes no test.
    pub(crate) fn occlusion_patches(
        &self,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
    ) -> Option<(FlareSprite, Option<FlareSprite>)> {
        let flare = &*self.flare;
        if !flare.tests_occlusion() {
            return None;
        }
        let (position, forward) = self.posed(bones);
        let outer = flare.occlusion_radius * self.marker_scaled();
        let patch = |radius: f32| {
            let offset = match flare.occlusion_offset {
                OcclusionOffset::TowardViewer => view.forward.map(|axis| -axis * radius),
                OcclusionOffset::MarkerForward => forward.map(|axis| axis * radius * std::f32::consts::SQRT_2),
                OcclusionOffset::None => [0.0; 3],
            };
            FlareSprite {
                center: std::array::from_fn(|axis| position[axis] + offset[axis]),
                extent: [radius; 2],
                rotation: 0.0,
                color: [0.0; 3],
                alpha: 0.0,
                image: 0,
                highlight: None,
                tint: None,
                depth_tested: false,
            }
        };
        Some((patch(outer), flare.inner_scale.map(|scale| patch(outer * scale))))
    }

    /// `lens_flare_draw_callback`. `occlusion` is `rasterizer_occlusion_get_result`
    /// with the chained test, and of the outer test alone.
    pub(crate) fn sprites(
        &self,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
        occlusion: [f32; 2],
    ) -> Vec<FlareSprite> {
        let flare = &*self.flare;
        let (position, forward) = self.posed(bones);
        let to_light = from_view(view, position);
        let depth = dot(to_light, view.forward);
        // `sub_92DA00` drops it past its far fade.
        if flare.fade[0] != flare.fade[1] && flare.fade[1] <= depth {
            return Vec::new();
        }
        let scaled_by_marker = flare.flags.scale_by_marker;
        let Some(value) = self.value(values, time, onset) else {
            return Vec::new();
        };
        // A marker's scale takes the place of the attachment's, and of its color.
        let interpolation = if scaled_by_marker { 1.0 } else { value };
        let radius_scale = self.marker_scaled();
        let attached = match (scaled_by_marker, self.change_color) {
            (false, Some(index)) => change_colors.get(index).copied().unwrap_or([1.0; 3]),
            _ => [1.0; 3],
        };
        let external = match &self.light {
            Some(light) => {
                let Some(diffuse) = light.diffuse_at(value, time) else {
                    return Vec::new();
                };
                if scaled_by_marker { [1.0; 3] } else { diffuse }
            }
            None => attached,
        };
        let mut fade = 1.0;
        if flare.fade[0] != flare.fade[1] {
            fade = ((depth - flare.fade[1]) / (flare.fade[0] - flare.fade[1])).clamp(0.0, 1.0);
        }
        let [falloff, cutoff] = flare.cones;
        if falloff != cutoff {
            let slope = 1.0 / (falloff - cutoff);
            fade *= (-cutoff * slope - dot(forward, to_light) * slope).clamp(0.0, 1.0);
        }
        let [chained, outer] = occlusion;
        let time = if flare.synchronized { time } else { time + self.unsynchronized_offset() };
        let mut brightness = fade * chained;
        let not_inner = fade * outer;
        if let Some(function) = &flare.brightness {
            brightness *= function.evaluate(time, 0.0);
        }
        if brightness <= 0.0 {
            return Vec::new();
        }
        let color = flare.color.as_ref().map_or([1.0; 3], |function| {
            let color = function.evaluate_color(time, 0.0);
            [color.red, color.green, color.blue]
        });
        let mut corona = corona_rotation(
            flare.rotation_function,
            flare.rotation_scale,
            to_light,
            forward,
            view,
        );
        if let Some(function) = &flare.rotation {
            corona += function.evaluate(time, 0.0) * std::f32::consts::PI;
        }
        let screen_angle = dot(to_light, view.right).atan2(dot(to_light, view.up));
        let mirror: [f32; 3] =
            std::array::from_fn(|axis| (view.forward[axis] * depth - to_light[axis]) * 2.0);
        let mut sprites = Vec::new();
        for (index, reflection) in flare.reflections.iter().enumerate() {
            let [low, high] = reflection.brightness;
            let scale = lerp(low, high, interpolation).clamp(0.0, 1.0);
            let [small, large] = reflection.radius;
            let mut radius = lerp(small, large, interpolation) * radius_scale;
            let kept = reflection.flags.not_affected_by_inner_occlusion;
            let alpha = if kept { not_inner } else { brightness } * scale;
            if alpha <= 0.0 || radius <= 0.0 {
                continue;
            }
            let mut rotation = reflection.rotation;
            let mut stretch = [1.0; 2];
            if index == 0 {
                rotation += corona;
                stretch = flare.corona_scale;
            }
            if reflection.flags.align_with_screen_center {
                rotation += screen_angle;
            }
            if reflection.flags.radius_scaled_by_occlusion {
                radius *= (chained * 0.5 + 0.5).clamp(0.0, 1.0);
            }
            if reflection.flags.radius_not_scaled_by_distance {
                radius *= depth;
            }
            let tint: [f32; 3] = std::array::from_fn(|channel| {
                let own = color[channel] * reflection.color[channel];
                if !reflection.flags.ignore_light_color { own * external[channel] } else { own }
            });
            sprites.push(FlareSprite {
                center: std::array::from_fn(|axis| {
                    position[axis] + mirror[axis] * reflection.position
                }),
                extent: [radius * stretch[0], radius * stretch[1]],
                rotation,
                color: tint,
                alpha,
                image: reflection.image,
                highlight: Some((1.0 - reflection.modulation).clamp(0.0, 1.0)),
                tint: None,
                depth_tested: reflection.flags.occluded_by_solid_objects,
            });
        }
        sprites
    }
}
