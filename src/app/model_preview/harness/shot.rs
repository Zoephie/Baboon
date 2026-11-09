//! Renders a real model through the real renderer with no window, so a pass
//! can be looked at without opening the app. macOS only, through CGL.
//!
//! `H2_SHOT="<model path>|<out file>|<shaders 0/1>|<lights 0/1>|<yaw>|<pitch>"`,
//! several separated by `;`, with `BLAM_TEST_H2EK` naming the kit's tags.
//! With `H2_SHOT_CE` set, that kit is Halo CE's and the models gbxmodels, or
//! bipeds with `H2_SHOT_BIPED`, or vehicles with `H2_SHOT_OBJECT=vehicle`, whose
//! function inputs `H2_SHOT_INPUTS` can set, posed at `H2_SHOT_CLIP`,
//! holding `H2_SHOT_WEAPON` and `H2_SHOT_SECONDARY`, with named Halo 2
//! function inputs from `H2_SHOT_FUNCTIONS=name=value,..`, leaving out the
//! features `H2_SHOT_DROP` names by their paths (`h2.flares`, `particles`,
//! `gen3.prt`, `reach.shield` …), and exports forced by `H2_SHOT_FORCE`.
//! `H2_SHOT_PLAY=<text>@<seconds>` plays each Halo CE event effect whose
//! name holds the text, that long before the shot, and
//! `H2_SHOT_FIRE=<trigger>@<seconds>` fires the held weapon's trigger.

use super::super::*;
use super::gl::{headless_gl, headless_gl_recording, take_feedback_loops, take_gl_log};
use super::gl_state::gl_state;
use crate::core::format::TagNameIndex;
use crate::core::game::GameId;
use crate::core::source::{TagEntry, TagEntryLocation, TagSource};
use eframe::egui;
use eframe::glow::{self, HasContext as _};
use std::sync::Arc;

/// The `H2_SHOT_*` game flag.
fn shot_game() -> GameId {
    if std::env::var_os("H2_SHOT_CE").is_some() {
        GameId::HaloCe
    } else if std::env::var_os("H2_SHOT_ODST").is_some() {
        GameId::Halo3Odst
    } else if std::env::var_os("H2_SHOT_H4").is_some() {
        GameId::Halo4
    } else if std::env::var_os("H2_SHOT_REACH").is_some() {
        GameId::HaloReach
    } else if std::env::var_os("H2_SHOT_H3").is_some() {
        GameId::Halo3
    } else {
        GameId::Halo2
    }
}

/// How a shot is taken beyond its model and angle: what [`shot`] reads
/// from its `H2_SHOT_*` variables, and what the gates set themselves.
#[derive(Debug, Clone)]
pub(in crate::app::model_preview) struct ShotOptions {
    pub(in crate::app::model_preview) game: GameId,
    /// An object tag's kind (`H2_SHOT_OBJECT`), for every shot without its own.
    pub(in crate::app::model_preview) kind: Option<String>,
    pub(in crate::app::model_preview) biped: bool,
    pub(in crate::app::model_preview) weapon: Option<String>,
    pub(in crate::app::model_preview) secondary_weapon: Option<String>,
    /// `H4_TEXTURES`: each pass's samplers, and whether one is bound.
    pub(in crate::app::model_preview) report_textures: bool,
    /// Whole features left out (`H2_SHOT_DROP`).
    pub(in crate::app::model_preview) dropped: Vec<String>,
    pub(in crate::app::model_preview) fixed_light: bool,
    pub(in crate::app::model_preview) variant: Option<String>,
    /// The point to orbit, in the model's space.
    pub(in crate::app::model_preview) focus: Option<[f32; 3]>,
    /// Times the fit.
    pub(in crate::app::model_preview) scale: Option<f32>,
    pub(in crate::app::model_preview) clip: Option<String>,
    /// Materials whose shaders end in these are left out.
    pub(in crate::app::model_preview) hidden: Vec<String>,
    pub(in crate::app::model_preview) time: f32,
    /// `solid`, `wire`, `solid_wire` or `textured_wire`.
    pub(in crate::app::model_preview) mode: Option<String>,
    pub(in crate::app::model_preview) light_theme: bool,
    pub(in crate::app::model_preview) grid: bool,
    pub(in crate::app::model_preview) camo: bool,
    pub(in crate::app::model_preview) bloom: bool,
    pub(in crate::app::model_preview) shadow: bool,
    pub(in crate::app::model_preview) no_prt: bool,
    /// The opened Halo CE object's four inputs.
    pub(in crate::app::model_preview) inputs: Option<[f32; 4]>,
    /// Named Halo 2 function inputs.
    pub(in crate::app::model_preview) functions: Vec<(String, f32)>,
    /// What functions export, set over what they make.
    pub(in crate::app::model_preview) forced: Vec<(String, f32)>,
    /// VISR's ping counts from when it comes on, a frame before.
    pub(in crate::app::model_preview) vision: bool,
    /// `<text>@<seconds>`: plays each Halo CE event effect named with the text.
    pub(in crate::app::model_preview) play: Option<String>,
    /// `<trigger>@<seconds>`: fires the held weapon's trigger.
    pub(in crate::app::model_preview) fire: Option<String>,
    /// The preview keeps no game, as one from a source that names none.
    pub(in crate::app::model_preview) no_game: bool,
}

impl ShotOptions {
    /// A plain shot of `game`: dark theme, at rest, nothing set or left out.
    pub(in crate::app::model_preview) fn new(game: GameId) -> Self {
        Self {
            game,
            kind: None,
            biped: false,
            weapon: None,
            secondary_weapon: None,
            report_textures: false,
            dropped: Vec::new(),
            fixed_light: false,
            variant: None,
            focus: None,
            scale: None,
            clip: None,
            hidden: Vec::new(),
            time: 0.0,
            mode: None,
            light_theme: false,
            grid: false,
            camo: false,
            bloom: false,
            shadow: false,
            no_prt: false,
            inputs: None,
            functions: Vec::new(),
            forced: Vec::new(),
            vision: false,
            play: None,
            fire: None,
            no_game: false,
        }
    }

    fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        let set = |name: &str| std::env::var_os(name).is_some();
        let list = |name: &str| {
            var(name).map_or_else(Vec::new, |list| {
                list.split(',').map(str::to_owned).collect()
            })
        };
        let pairs = |name: &str| {
            list(name)
                .iter()
                .map(|pair| {
                    let (name, value) = pair.split_once('=').unwrap();
                    (name.to_owned(), value.parse().unwrap())
                })
                .collect()
        };
        let floats = |text: String| {
            text.split(',')
                .map(|value| value.parse().unwrap())
                .collect::<Vec<f32>>()
        };
        Self {
            game: shot_game(),
            kind: var("H2_SHOT_OBJECT"),
            biped: set("H2_SHOT_BIPED"),
            weapon: var("H2_SHOT_WEAPON"),
            secondary_weapon: var("H2_SHOT_SECONDARY"),
            report_textures: set("H4_TEXTURES"),
            dropped: list("H2_SHOT_DROP"),
            fixed_light: set("H2_SHOT_FIXED_LIGHT"),
            variant: var("H2_SHOT_VARIANT"),
            focus: var("H2_SHOT_FOCUS").map(|focus| {
                let point = floats(focus);
                std::array::from_fn(|axis| point[axis])
            }),
            scale: var("H2_SHOT_SCALE").map(|scale| scale.parse().unwrap()),
            clip: var("H2_SHOT_CLIP"),
            hidden: list("H2_SHOT_HIDE"),
            time: var("H2_SHOT_TIME").map_or(0.0, |time| time.parse().unwrap()),
            mode: var("H2_SHOT_MODE"),
            light_theme: set("H2_SHOT_LIGHT"),
            grid: set("H2_SHOT_GRID"),
            camo: set("H2_SHOT_CAMO"),
            bloom: set("H2_SHOT_BLOOM"),
            shadow: set("H2_SHOT_SHADOW"),
            no_prt: set("H2_SHOT_NO_PRT"),
            inputs: var("H2_SHOT_INPUTS").map(|values| {
                let values = floats(values);
                std::array::from_fn(|index| values[index])
            }),
            functions: pairs("H2_SHOT_FUNCTIONS"),
            forced: pairs("H2_SHOT_FORCE"),
            vision: set("H2_SHOT_VISION"),
            play: var("H2_SHOT_PLAY"),
            fire: var("H2_SHOT_FIRE"),
            no_game: set("H2_SHOT_NO_GAME"),
        }
    }
}

#[derive(Clone)]
pub(in crate::app::model_preview) struct Shot {
    pub(in crate::app::model_preview) model: String,
    pub(in crate::app::model_preview) shaders: bool,
    pub(in crate::app::model_preview) lights: bool,
    pub(in crate::app::model_preview) yaw: f32,
    pub(in crate::app::model_preview) pitch: f32,
    /// An object tag's kind; else the options'.
    pub(in crate::app::model_preview) kind: Option<String>,
}

/// `H2_SHOT_OBJECT`'s kinds: the object tag's extension and group.
fn object_kind(kind: &str) -> (&'static str, &'static [u8; 4]) {
    match kind {
        "scenery" => ("scenery", b"scen"),
        "machine" => ("device_machine", b"mach"),
        "control" => ("device_control", b"ctrl"),
        "crate" => ("crate", b"bloc"),
        "giant" => ("giant", b"gint"),
        "equipment" => ("equipment", b"eqip"),
        "weapon" => ("weapon", b"weap"),
        "vehicle" => ("vehicle", b"vehi"),
        "effect_scenery" => ("effect_scenery", b"efsc"),
        "projectile" => ("projectile", b"proj"),
        _ => ("biped", b"bipd"),
    }
}

/// What [`render`] gives back of a shot.
pub(super) struct Rendered {
    /// RGBA, bottom row first.
    pub(super) pixels: Vec<u8>,
    /// The load's, which names its slot in the renderer.
    pub(super) geometry_id: u64,
    /// [`gl_state`] as the paint left it.
    pub(super) gl_state: String,
}

/// Takes `shot` with `renderer`.
pub(super) fn render(
    gl: &Arc<glow::Context>,
    renderer: &mut ModelGlRenderer,
    source: &TagSource,
    shot: &Shot,
    options: &ShotOptions,
) -> Rendered {
    let names = match source {
        TagSource::LooseFolder {
            definitions_root, ..
        } => TagNameIndex::load_game(definitions_root, options.game).unwrap(),
        _ => unreachable!(),
    };
    let TagSource::LooseFolder { root, .. } = source else {
        unreachable!()
    };
    // A sky has a render_model and nothing above it.
    let file = root.join(format!("{}.model", shot.model.replace('\\', "/")));
    let (extension, group) = if let Some(kind) = shot.kind.clone().or_else(|| options.kind.clone())
    {
        object_kind(&kind)
    } else if options.biped {
        ("biped", b"bipd")
    } else if options.game == GameId::HaloCe {
        ("gbxmodel", b"mod2")
    } else if file.exists() {
        ("model", b"hlmt")
    } else {
        ("render_model", b"mode")
    };
    let tag = load_referenced_tag_from_source(source, &shot.model, extension, group).unwrap();
    let entry = TagEntry {
        key: format!("{}.{extension}", shot.model),
        display_path: format!("{}.{extension}", shot.model),
        group_tag: u32::from_be_bytes(*group),
        group_name: Some(extension.to_owned()),
        location: TagEntryLocation::LooseFile(file.with_extension(extension)),
    };
    let settings = PreviewLoadSettings {
        weapon: options.weapon.clone(),
        secondary_weapon: options.secondary_weapon.clone(),
        equippable: None,
        ..Default::default()
    };
    let mut data = load_model_preview(&tag, &entry, &names, Some(source), &settings).unwrap();
    if options.no_game {
        Arc::make_mut(&mut data.preview).game = None;
    }
    let textures = materials::resolve_model_textures(source, &data.preview.materials);
    let with_passes = textures
        .iter()
        .filter(|t| t.dx9_layers.iter().any(|l| !l.is_empty()))
        .count();
    eprintln!(
        "{}: {} materials, {with_passes} with passes",
        shot.model,
        textures.len()
    );
    for (material, texture) in data.preview.materials.iter().zip(textures.iter()) {
        eprintln!(
            "NOTE MAT {} h3 {} {:?}",
            material.shader_path,
            texture.gen3.is_some(),
            texture.error
        );
        // `H4_TEXTURES=1`: each pass's samplers, and whether one is bound.
        if options.report_textures
            && let Some(h3) = &texture.gen3
        {
            let passes = h3.albedo.iter().chain(std::iter::once(&h3.lit));
            for (name, kind) in passes.flat_map(|pass| pass.textures.iter()) {
                let bound = h3.textures.iter().find(|known| &known.uniform == name);
                let detail = bound
                    .map(|known| format!("{:?} {:?}", known.texels, known.sampler))
                    .unwrap_or_else(|| "UNBOUND".to_owned());
                eprintln!("NOTE   TEX {name} {kind:?} {detail}");
            }
        }
    }
    data.textures = Some(Arc::new(textures));
    eprintln!(
        "NOTE {} cheap particle emitters",
        data.preview.gen3.cheap_emitters.len()
    );
    eprintln!(
        "NOTE H4 {} lights, {} cheap lights, {} light passes, {} flares, {} particle systems ({} on materials), {} light volumes ({} on materials), {} tracers",
        data.preview.h4.lights.len(),
        data.preview.h4.cheap_lights.len(),
        data.preview.h4.light_passes.len(),
        data.preview.h4.flares.len(),
        data.preview.particles.len(),
        data.preview
            .particles
            .iter()
            .filter(|system| system.look.material.is_some())
            .count(),
        data.preview.gen3.light_volumes.len(),
        data.preview
            .gen3
            .light_volumes
            .iter()
            .filter(|volume| volume.h4_material().is_some())
            .count(),
        data.preview.h4.tracers.len()
    );
    // Whole features left out, to see one on its own.
    if !options.dropped.is_empty() {
        let preview = Arc::make_mut(&mut data.preview);
        for feature in &options.dropped {
            match feature.as_str() {
                // Not features: data the materials draw with.
                "gen3.prt" => preview.gen3.prt = None,
                "reach.shield" => preview.reach.shield = None,
                path => {
                    assert!(
                        preview.clear_feature(path),
                        "H2_SHOT_DROP: no feature {path}"
                    );
                    // The sheet's own batches go with it; its vertices stay, unused.
                    if path == "cloth" {
                        preview
                            .batches
                            .retain(|batch| batch.permutation_name != "cloth");
                    }
                }
            }
        }
    }
    let mut state = ModelPreviewState {
        light_follows_camera: !options.fixed_light,
        ..Default::default()
    };
    let variant = options.variant.as_ref();
    let variant = variant.and_then(|name| {
        data.variants
            .iter()
            .position(|variant| &variant.name == name)
    });
    reset_model_preview_selection(&mut state, &data, variant.or(default_variant_index(&data)));
    state.game_shaders = shot.shaders;
    state.default_lights = shot.lights;
    state.yaw = shot.yaw;
    state.pitch = shot.pitch;
    state.perspective = true;
    // The focus is orbited, at the scale times the fit.
    let (center, radius) = preview_center_radius(&data.preview);
    eprintln!("NOTE center {center:?} radius {radius}");
    if let Some(point) = options.focus {
        state.focus = std::array::from_fn(|axis| point[axis] - center[axis]);
    }
    if let Some(scale) = options.scale {
        state.scale = scale;
    }
    state.weapon = settings.weapon.clone();
    state.secondary_weapon = settings.secondary_weapon.clone();
    if let Some(clip) = &options.clip {
        animation::pose_for_shot(source, &entry, &mut data, &mut state, clip);
    }

    let (width, height) = (SHOT_SIZE[0] as i32, SHOT_SIZE[1] as i32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, height as f32));
    // The shot's options, as the panel would set them.
    if let Some(mode) = match options.mode.as_deref() {
        Some("solid") => Some(ModelRenderMode::Solid),
        Some("wire") => Some(ModelRenderMode::Wireframe),
        Some("solid_wire") => Some(ModelRenderMode::SolidWireframe),
        Some("textured_wire") => Some(ModelRenderMode::TexturedWireframe),
        _ => None,
    } {
        state.render_mode = mode;
    }
    state.show_grid = options.grid;
    state.active_camo = options.camo;
    state.bloom = options.bloom;
    state.self_shadow = options.shadow;
    state.prt = !options.no_prt;
    if let Some(values) = options.inputs {
        state
            .ce
            .function_inputs
            .insert((data.geometry_id, 0), values);
    }
    for (name, value) in &options.functions {
        state
            .function_inputs
            .insert((data.geometry_id, name.clone()), *value);
    }
    let background = preview_background(&if options.light_theme {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    });
    let mut frame = super::super::frame::build_frame(
        &data,
        &state,
        rect,
        options.time,
        background,
        &options.forced,
    );
    let preview = Arc::clone(&frame.preview);
    frame.visible_batches.retain(|&batch| {
        let material = preview.batches[batch].material_index as usize;
        !options.hidden.iter().any(|name| {
            preview.materials[material]
                .shader_path
                .ends_with(name.as_str())
        })
    });
    eprintln!("NOTE moves {}", frame.moves());
    unsafe {
        let color = gl.create_renderbuffer().unwrap();
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(color));
        gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, width, height);
        let depth = gl.create_renderbuffer().unwrap();
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
        gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, width, height);
        let framebuffer = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::RENDERBUFFER,
            Some(color),
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(depth),
        );
        assert_eq!(
            gl.check_framebuffer_status(glow::FRAMEBUFFER),
            glow::FRAMEBUFFER_COMPLETE
        );
        // As the renderer's own targets leave it, so no shot's state says
        // whether the renderer made its targets in it.
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        gl.viewport(0, 0, width, height);
        gl.color_mask(true, true, true, true);
        gl.depth_mask(true);
        let [r, g, b] = frame.background;
        gl.clear_color(r, g, b, 1.0);
        gl.clear_depth_f32(1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        // VISR's ping counts from when it comes on, which takes a frame before.
        if options.vision {
            let end = std::mem::replace(&mut frame.time, 0.0);
            renderer.paint(gl, &frame);
            frame.time = end;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }
        let play = options.play.as_ref().map(|play| (play, false));
        if let Some((play, fire)) = play.or_else(|| options.fire.as_ref().map(|fire| (fire, true)))
        {
            let (text, before) = play
                .split_once('@')
                .expect("a play or fire is <what>@<seconds>");
            let end = frame.time;
            frame.time = end - before.parse::<f32>().unwrap();
            renderer.paint(gl, &frame);
            if fire {
                let weapon = data.preview.ce.firing.last().expect("a weapon to fire");
                let fired = state
                    .ce
                    .shots
                    .entry((data.geometry_id, weapon.object))
                    .or_default();
                let effect = fired.fire(
                    weapon,
                    text.parse().unwrap(),
                    state.ce.event_scales[0],
                    frame.time,
                );
                eprintln!(
                    "NOTE fired, playing {:?}",
                    effect.and_then(|at| data.preview.ce.effects[at].event.as_ref())
                );
                if let Some(effect) = effect {
                    frame.ce.plays.counts[effect] += 1;
                }
            }
            for (index, effect) in data.preview.ce.effects.iter().enumerate() {
                if let Some(event) = effect
                    .event
                    .as_ref()
                    .filter(|event| !fire && event.name.contains(text))
                {
                    eprintln!("NOTE playing {}", event.name);
                    frame.ce.plays.counts[index] += 1;
                }
            }
            // Step to the shot a frame at a time, as the app would.
            let mut at = frame.time;
            while at < end {
                at = (at + 1.0 / 60.0).min(end);
                frame.time = at;
                frame.ce.objects =
                    object_states(&data, &state.ce.function_inputs, &state.ce.shots, at);
                frame.ce.points = frame.ce_point_lights();
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                renderer.paint(gl, &frame);
            }
            eprintln!(
                "NOTE states {:?}",
                frame
                    .ce
                    .objects
                    .iter()
                    .map(|state| state.outputs)
                    .collect::<Vec<_>>()
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }
        renderer.paint(gl, &frame);
        assert_eq!(gl.get_error(), 0, "GL error during the frame");
        let gl_state = gl_state(gl, framebuffer);
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.read_pixels(
            0,
            0,
            width,
            height,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
        Rendered {
            pixels,
            geometry_id: data.geometry_id,
            gl_state,
        }
    }
}

/// What [`shoot`] gives back of a shot.
pub(in crate::app::model_preview) struct Shown {
    /// RGBA, bottom row first.
    pub(in crate::app::model_preview) pixels: Vec<u8>,
    /// A line per simulation field its slot kept: [`super::super::gates::field_lines`].
    pub(in crate::app::model_preview) simulation: String,
    /// A line per GL setting the paint left.
    pub(in crate::app::model_preview) gl_state: String,
    /// Every GL state call the shot made, in order: [`take_gl_log`].
    pub(in crate::app::model_preview) gl_calls: Vec<String>,
    /// Each draw that sampled what it drew into: [`take_feedback_loops`].
    pub(in crate::app::model_preview) feedback_loops: Vec<String>,
}

/// Takes each shot in one headless context, from the kit `source` reads.
pub(in crate::app::model_preview) fn shoot(
    source: &TagSource,
    shots: &[(Shot, ShotOptions)],
) -> Vec<Shown> {
    let gl = Arc::new(headless_gl_recording());
    let mut renderer = ModelGlRenderer::new(&gl).unwrap();
    take_gl_log();
    shots
        .iter()
        .map(|(shot, options)| {
            // Each shot starts from the same bindings, so what G-gl reads
            // after it is the shot's own, not the shots before it.
            unsafe { reset_bindings(&gl) };
            take_gl_log();
            take_feedback_loops();
            let Rendered {
                pixels,
                geometry_id,
                gl_state,
            } = render(&gl, &mut renderer, source, shot, options);
            let gl_calls = take_gl_log();
            let mut simulation = String::new();
            renderer.for_each_simulation(geometry_id, |name, state| {
                super::super::gates::field_lines(&mut simulation, name, &state);
            });
            let feedback_loops = take_feedback_loops();
            Shown {
                pixels,
                simulation,
                gl_state,
                gl_calls,
                feedback_loops,
            }
        })
        .collect()
}

/// Unbinds what a paint may leave bound: every unit's textures, the program,
/// the vertex array and the buffers, with the first unit active.
unsafe fn reset_bindings(gl: &glow::Context) {
    unsafe {
        for unit in 0..16 {
            gl.active_texture(glow::TEXTURE0 + unit);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_texture(glow::TEXTURE_CUBE_MAP, None);
            gl.bind_texture(glow::TEXTURE_3D, None);
        }
        gl.active_texture(glow::TEXTURE0);
        gl.use_program(None);
        gl.bind_vertex_array(None);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }
}

/// The size [`render`] shoots at.
pub(in crate::app::model_preview) const SHOT_SIZE: [u32; 2] = [1000, 760];

#[test]
fn shot() {
    let (Ok(shots), Ok(root)) = (std::env::var("H2_SHOT"), std::env::var("BLAM_TEST_H2EK")) else {
        return;
    };
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(shot_game()),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let options = ShotOptions::from_env();
    let gl = Arc::new(headless_gl());
    unsafe {
        eprintln!("GL {}", gl.get_parameter_string(glow::VERSION));
        eprintln!("renderer {}", gl.get_parameter_string(glow::RENDERER));
    }
    let mut renderer = ModelGlRenderer::new(&gl).unwrap();
    for spec in shots.split(';') {
        let f: Vec<&str> = spec.split('|').collect();
        let shot = Shot {
            model: f[0].to_owned(),
            shaders: f[2] == "1",
            lights: f[3] == "1",
            yaw: f[4].parse().unwrap(),
            pitch: f[5].parse().unwrap(),
            kind: None,
        };
        std::fs::write(
            f[1],
            render(&gl, &mut renderer, &source, &shot, &options).pixels,
        )
        .unwrap();
    }
}

/// Halo 3's self shadow applies by its own pipeline's normals. Each gen3
/// game keeps its own targets, so a Reach preview that made its own after
/// Halo 3's leaves Halo 3's next shadow as it was.
#[test]
fn halo_3s_self_shadow_reads_its_own_normals_after_another_gen3_preview() {
    let (Some(halo3), Some(reach)) = (
        super::kit::kit_source(GameId::Halo3),
        super::kit::kit_source(GameId::HaloReach),
    ) else {
        return;
    };
    let shot = |model: &str, kind: &str| Shot {
        model: model.to_owned(),
        shaders: true,
        lights: true,
        yaw: 0.6,
        pitch: 0.25,
        kind: Some(kind.to_owned()),
    };
    let chief = shot("objects\\characters\\masterchief\\masterchief", "biped");
    let warthog = shot("objects\\vehicles\\human\\warthog\\warthog", "vehicle");
    let h3 = ShotOptions {
        shadow: true,
        ..ShotOptions::new(GameId::Halo3)
    };
    let on_reach = ShotOptions {
        shadow: true,
        ..ShotOptions::new(GameId::HaloReach)
    };
    let gl = Arc::new(headless_gl());
    let mut renderer = ModelGlRenderer::new(&gl).unwrap();
    let before = render(&gl, &mut renderer, &halo3, &chief, &h3).pixels;
    let reach_drawn = render(&gl, &mut renderer, &reach, &warthog, &on_reach).pixels;
    let after = render(&gl, &mut renderer, &halo3, &chief, &h3).pixels;
    assert!(
        reach_drawn.iter().any(|byte| *byte != 0),
        "the Reach warthog drew"
    );
    assert!(
        before == after,
        "Halo 3's shadow read another pipeline's normals"
    );
}

/// The gen3 pipelines share one set of targets: a Reach frame after a Halo 3
/// one, at the same size, draws through the set Halo 3 made rather than a
/// second, and hands it back.
#[test]
fn gen3_games_share_one_set_of_targets() {
    let (Some(halo3), Some(reach)) = (
        super::kit::kit_source(GameId::Halo3),
        super::kit::kit_source(GameId::HaloReach),
    ) else {
        return;
    };
    let shot = |model: &str, kind: &str| Shot {
        model: model.to_owned(),
        shaders: true,
        lights: true,
        yaw: 0.6,
        pitch: 0.25,
        kind: Some(kind.to_owned()),
    };
    let gl = Arc::new(headless_gl());
    let mut renderer = ModelGlRenderer::new(&gl).unwrap();
    let chief = shot("objects\\characters\\masterchief\\masterchief", "biped");
    render(
        &gl,
        &mut renderer,
        &halo3,
        &chief,
        &ShotOptions::new(GameId::Halo3),
    );
    let made = renderer.gen3_targets_normal();
    assert!(made.is_some(), "Halo 3's frame handed its targets back");
    let warthog = shot("objects\\vehicles\\human\\warthog\\warthog", "vehicle");
    render(
        &gl,
        &mut renderer,
        &reach,
        &warthog,
        &ShotOptions::new(GameId::HaloReach),
    );
    assert_eq!(
        renderer.gen3_targets_normal(),
        made,
        "Reach drew through Halo 3's set"
    );
}
