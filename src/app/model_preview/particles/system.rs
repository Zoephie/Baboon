//! Particle systems: their parts, the strips they carry, and reading them off an effect.

use super::*;

/// A system in an event of an attached effect, at a marker.
#[derive(Debug)]
pub(crate) struct H3ParticleSystem {
    pub(super) node: usize,
    pub(super) position: [f32; 3],
    pub(super) axes: [[f32; 3]; 3],
    only_under: Option<(String, String)>,
    pub(crate) object: u8,
    pub(super) scale_by: String,
    pub(super) secondary_by: String,
    pub(super) sort_bias: f32,
    /// `coordinate system`: particles left where they were made.
    pub(super) world: bool,
    pub(super) location_seed: f32,
    pub(super) near_range: f32,
    pub(super) near_cutoff: f32,
    pub(super) times: Arc<EffectTimes>,
    pub(super) event: usize,
    pub(crate) look: Arc<H3ParticleLook>,
    pub(super) emitters: Vec<Arc<H3Emitter>>,
    /// What its particles start: how far along the list each system lies.
    pub(super) attached: Vec<(H3ParticleTrigger, usize)>,
    /// Started by a particle, at the particle: nothing of its own runs it.
    pub(super) child: bool,
    /// `use parent position but world orientation`: its axes do not turn.
    pub(super) world_oriented: bool,
    /// Contrails its particles start, each following the particle.
    pub(super) carried: Vec<Arc<H3StripAt>>,
    pub(super) hidden: bool,
    pub(super) die_with_effect: bool,
    pub(super) lod: Option<[f32; 4]>,
}

/// What a tag's `particle system` holds, wherever it ends up.
#[derive(Clone)]
pub(super) struct H3SystemParts {
    /// `don't render system`: it still runs, for what its particles start.
    hidden: bool,
    /// `particles die when effect ends`.
    die_with_effect: bool,
    lod: Option<[f32; 4]>,
    sort_bias: f32,
    world: bool,
    near_range: f32,
    near_cutoff: f32,
    look: Arc<H3ParticleLook>,
    pub(super) emitters: Vec<Arc<H3Emitter>>,
}

/// A system's `lod always 1.0`, and what older layouts label it.
const LOD_ALWAYS_ONE: [&str; 3] = ["lod always 1.0", "ignore lod (render always)", "lod always 1"];

/// A system's `particles die when effect ends`, and its older label.
const DIE_WITH_EFFECT: [&str; 2] = ["particles die when effect ends", "die when orphaned"];

pub(super) fn read_system_parts(reader: &TagReader, system: TagStruct<'_>) -> Option<H3SystemParts> {
    let flagged = |wanted: &str| has_flag(system, "flags", wanted);
    if !part_runs_by(system, "environment", "disposition") {
        return None;
    }
    let path = system.read_tag_ref_path("particle").filter(|path| !path.is_empty())?;
    let own = ParticleRules::of(reader.source.game()).own_system;
    let (look, emitters): (Arc<H3ParticleLook>, Vec<_>) = if let Some(own) = own {
        (own.read)(reader, system, &path)?
    } else {
        let look = read_look(reader, &path).map(Arc::new)?;
        let size_scale = system.read_real("size scale").unwrap_or(1.0);
        let read = |emitter: &TagStruct<'_>| read_emitter(reader, *emitter, flagged, &look, size_scale).map(Arc::new);
        let emitters = elements(system, "emitters").iter().filter_map(read).collect();
        (look, emitters)
    };
    if emitters.is_empty() {
        return None;
    }
    let real = |field: &str| system.read_real(field).unwrap_or(0.0);
    let cutoff = if flagged("override near fade (use with caution)") { real("near fade override") } else { real("near fade cutoff") };
    Some(H3SystemParts {
        hidden: flagged("don't render system"),
        die_with_effect: has_flag_labelled(system, "flags", &DIE_WITH_EFFECT),
        lod: {
            let raw = [
                real("LOD in distance"),
                real("LOD out distance"),
                real("LOD feather in delta"),
                real("LOD feather out delta"),
            ];
            let any_camera = system.field("camera mode").is_some_and(|field| field.enum_is("independent of camera mode"));
            let mut rates = emitters.iter().map(|emitter| emitter.rate.most());
            if let Some(own) = own {
                Some((own.baked_lod)(raw, any_camera, &mut rates))
            } else if !has_flag_labelled(system, "flags", &LOD_ALWAYS_ONE) {
                Some(if look.reach { reach_baked_lod(raw) } else { baked_lod(raw, any_camera, rates) })
            } else {
                None
            }
        },
        sort_bias: system.read_int_any("sort bias").unwrap_or(0) as f32,
        world: system.read_enum_option("coordinate system").as_deref() == Some("world"),
        near_range: near_fade_range(look.reach, real("near fade range")),
        near_cutoff: cutoff,
        look,
        emitters,
    })
}

/// The systems of the effects a look's particles start, and their events.
fn read_attached_parts(
    reader: &TagReader,
    look: &H3ParticleLook,
) -> Vec<(H3ParticleTrigger, H3SystemParts, Arc<EffectTimes>, usize)> {
    let mut parts = Vec::new();
    for (trigger, path) in &look.attachments {
        let Ok(effect) = reader.load(path, "effect", b"effe") else {
            continue;
        };
        let times = Arc::new(EffectTimes::read(effect.root()));
        for (at, event) in elements(effect.root(), "events").into_iter().enumerate() {
            for system in elements(event, "particle systems") {
                if let Some(read) = read_system_parts(reader, system) {
                    parts.push((*trigger, read, Arc::clone(&times), at));
                }
            }
        }
    }
    let carried = look.carried_systems.as_ref();
    if let Some(tag) = carried.and_then(|path| reader.load(path, "particle", b"prt3").ok()) {
        let lasting = Arc::new(EffectTimes::lasting());
        for system in elements(tag.root(), "attached particle systems") {
            if let Some(read) = read_system_parts(reader, system) {
                parts.push((H3ParticleTrigger::Carried, read, Arc::clone(&lasting), 0));
            }
        }
    }
    parts
}

/// The contrails these systems' particles start, each once: a system at many
/// markers shares its own.
pub(in crate::app::model_preview) fn carried_strips(systems: &[Arc<H3ParticleSystem>]) -> Vec<Arc<H3StripAt>> {
    let mut strips: Vec<Arc<H3StripAt>> = Vec::new();
    for strip in systems.iter().flat_map(|system| &system.carried) {
        if !strips.iter().any(|known| Arc::ptr_eq(known, strip)) {
            strips.push(Arc::clone(strip));
        }
    }
    strips
}

impl H3ParticleSystem {
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    pub(crate) fn scale_names(&self) -> [&str; 2] {
        [&self.scale_by, &self.secondary_by]
    }

}

/// The systems of an object's attached effects, one at each marker of a location.
pub(in crate::app::model_preview) fn read_h3_particles(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H3ParticleSystem> {
    let mut systems = Vec::new();
    for object_effect in object_effects(reader, object) {
        let Ok(effect) = reader.load(&object_effect.path, "effect", b"effe") else {
            continue;
        };
        let locations = elements(effect.root(), "locations");
        let times = Arc::new(object_effect.times_of(effect.root()));
        let world_oriented = ObjectEffect::world_oriented(effect.root());
        for (at, event) in elements(effect.root(), "events").into_iter().enumerate() {
            for system in elements(event, "particle systems") {
                let location = usize::try_from(system.read_block_index("location")).ok();
                let Some(location) = location.and_then(|location| locations.get(location)) else {
                    continue;
                };
                let name = object_effect.marker_of(*location);
                let Some(parts) = read_system_parts(reader, system) else {
                    continue;
                };
                let children = read_attached_parts(reader, &parts.look);
                let carried: Vec<_> = parts
                    .look
                    .attachments
                    .iter()
                    .filter(|(trigger, _)| *trigger == H3ParticleTrigger::Birth)
                    .flat_map(|(_, path)| read_carried_strips(reader, path))
                    .map(Arc::new)
                    .collect();
                for marker in &object_effect.locations(markers, &name, &frame) {
                    let (place, node) = frame(marker);
                    let attached = children.iter().enumerate().map(|(at, child)| (child.0, at + 1)).collect();
                    let parts = parts.clone();
                    systems.push(H3ParticleSystem {
                        node,
                        position: place.position,
                        axes: place.axes,
                        only_under: marker.only_under.clone(),
                        object: owner,
                        scale_by: object_effect.scale_by.clone(),
                        secondary_by: object_effect.secondary_by.clone(),
                        sort_bias: parts.sort_bias,
                        world: parts.world,
                        location_seed: (systems.len() as f32 * 0.618_034).fract(),
                        near_range: parts.near_range,
                        near_cutoff: parts.near_cutoff,
                        times: Arc::clone(&times),
                        event: at,
                        look: parts.look,
                        emitters: parts.emitters,
                        attached,
                        child: false,
                        world_oriented,
                        carried: carried.clone(),
                        hidden: parts.hidden,
                        die_with_effect: parts.die_with_effect,
                        lod: parts.lod,
                    });
                    for (_, parts, times, event) in &children {
                        let parts = parts.clone();
                        systems.push(H3ParticleSystem {
                            node,
                            position: place.position,
                            axes: place.axes,
                            only_under: marker.only_under.clone(),
                            object: owner,
                            scale_by: String::new(),
                            secondary_by: String::new(),
                            sort_bias: parts.sort_bias,
                            world: parts.world,
                            location_seed: (systems.len() as f32 * 0.618_034).fract(),
                            near_range: parts.near_range,
                            near_cutoff: parts.near_cutoff,
                            times: Arc::clone(times),
                            event: *event,
                            look: parts.look,
                            emitters: parts.emitters,
                            attached: Vec::new(),
                            child: true,
                            world_oriented: false,
                            carried: Vec::new(),
                            hidden: parts.hidden,
                            die_with_effect: parts.die_with_effect,
                            lod: parts.lod,
                        });
                    }
                }
            }
        }
    }
    systems
}

impl H3ParticleSystem {
    /// Its sort position, nudged by its tag: `add_element`'s `sort_bias`.
    pub(crate) fn distance(&self, bones: Option<&[[f32; 4]]>, view: &ViewFrame) -> f32 {
        let (origin, _) = location(self, bones);
        let eye: [f32; 3] = std::array::from_fn(|axis| origin[axis] - view.eye[axis]);
        dot(view.forward, eye) - self.sort_bias * 0.05
    }

}

attached!(H3ParticleSystem);
