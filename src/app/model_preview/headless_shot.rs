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
//! features `H2_SHOT_DROP` names, and exports forced by `H2_SHOT_FORCE`.
//! `H2_SHOT_PLAY=<text>@<seconds>` plays each Halo CE event effect whose
//! name holds the text, that long before the shot, and
//! `H2_SHOT_FIRE=<trigger>@<seconds>` fires the held weapon's trigger.

use super::*;
use super::super::gl::link::link;
use crate::core::format::TagNameIndex;
use crate::core::game::GameId;
use crate::core::source::{TagEntry, TagEntryLocation, TagSource};
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};

#[link(name = "OpenGL", kind = "framework")]
unsafe extern "C" {
    fn CGLChoosePixelFormat(attributes: *const i32, format: *mut *mut c_void, count: *mut i32) -> i32;
    fn CGLCreateContext(format: *mut c_void, share: *mut c_void, context: *mut *mut c_void) -> i32;
    fn CGLSetCurrentContext(context: *mut c_void) -> i32;
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

fn headless_gl() -> glow::Context {
    const PROFILE: i32 = 99;
    const GL4_CORE: i32 = 0x4100;
    const COLOR_SIZE: i32 = 8;
    const DEPTH_SIZE: i32 = 12;
    unsafe {
        let attributes = [PROFILE, GL4_CORE, COLOR_SIZE, 24, DEPTH_SIZE, 24, 0];
        let (mut format, mut count, mut context) = (std::ptr::null_mut(), 0, std::ptr::null_mut());
        assert_eq!(CGLChoosePixelFormat(attributes.as_ptr(), &mut format, &mut count), 0);
        assert_eq!(CGLCreateContext(format, std::ptr::null_mut(), &mut context), 0);
        assert_eq!(CGLSetCurrentContext(context), 0);
        glow::Context::from_loader_function(|name| {
            let name = CString::new(name).unwrap();
            dlsym(-2isize as *mut c_void, name.as_ptr()) as *const c_void
        })
    }
}

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
        let list = |name: &str| var(name).map_or_else(Vec::new, |list| list.split(',').map(str::to_owned).collect());
        let pairs = |name: &str| {
            list(name)
                .iter()
                .map(|pair| {
                    let (name, value) = pair.split_once('=').unwrap();
                    (name.to_owned(), value.parse().unwrap())
                })
                .collect()
        };
        let floats = |text: String| text.split(',').map(|value| value.parse().unwrap()).collect::<Vec<f32>>();
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

/// The GL state a paint leaves, a line per setting. Bindings say only whether
/// something is bound (the framebuffer, whether it is the shot's own), as GL
/// names follow creation order, which a change of order alone would move.
fn gl_state(gl: &glow::Context, target: glow::NativeFramebuffer) -> String {
    use glow::HasContext as _;
    let mut out = String::new();
    let mut put = |name: &str, value: String| {
        out.push_str(name);
        out.push(' ');
        out.push_str(&value);
        out.push('\n');
    };
    unsafe {
        for (name, capability) in [
            ("blend", glow::BLEND),
            ("depth_test", glow::DEPTH_TEST),
            ("cull_face", glow::CULL_FACE),
            ("scissor_test", glow::SCISSOR_TEST),
            ("stencil_test", glow::STENCIL_TEST),
            ("framebuffer_srgb", glow::FRAMEBUFFER_SRGB),
            ("polygon_offset_fill", glow::POLYGON_OFFSET_FILL),
            ("sample_alpha_to_coverage", glow::SAMPLE_ALPHA_TO_COVERAGE),
            ("program_point_size", glow::PROGRAM_POINT_SIZE),
        ] {
            put(name, gl.is_enabled(capability).to_string());
        }
        for (name, parameter) in [
            ("blend_src_rgb", glow::BLEND_SRC_RGB),
            ("blend_dst_rgb", glow::BLEND_DST_RGB),
            ("blend_src_alpha", glow::BLEND_SRC_ALPHA),
            ("blend_dst_alpha", glow::BLEND_DST_ALPHA),
            ("blend_equation_rgb", glow::BLEND_EQUATION_RGB),
            ("blend_equation_alpha", glow::BLEND_EQUATION_ALPHA),
            ("depth_func", glow::DEPTH_FUNC),
            ("depth_writemask", glow::DEPTH_WRITEMASK),
            ("cull_face_mode", glow::CULL_FACE_MODE),
            ("front_face", glow::FRONT_FACE),
            ("active_texture", glow::ACTIVE_TEXTURE),
            ("pack_alignment", glow::PACK_ALIGNMENT),
            ("unpack_alignment", glow::UNPACK_ALIGNMENT),
            ("stencil_writemask", glow::STENCIL_WRITEMASK),
        ] {
            put(name, format!("{:#x}", gl.get_parameter_i32(parameter)));
        }
        let mut mask = [0; 4];
        gl.get_parameter_i32_slice(glow::COLOR_WRITEMASK, &mut mask);
        put("color_writemask", format!("{mask:?}"));
        // egui_glow does not restore it, so a wireframe left here would draw
        // the UI that way.
        let mut polygon_mode = [0; 2];
        gl.get_parameter_i32_slice(glow::POLYGON_MODE, &mut polygon_mode);
        put("polygon_mode", format!("{polygon_mode:x?}"));
        let mut viewport = [0; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
        put("viewport", format!("{viewport:?}"));
        let framebuffer = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
        put("draw_framebuffer", if framebuffer == Some(target) { "shot" } else if framebuffer.is_some() { "other" } else { "none" }.to_owned());
        let read = gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING);
        put("read_framebuffer", if read == Some(target) { "shot" } else if read.is_some() { "other" } else { "none" }.to_owned());
        for (name, parameter) in [
            ("program", glow::CURRENT_PROGRAM),
            ("vertex_array", glow::VERTEX_ARRAY_BINDING),
            ("array_buffer", glow::ARRAY_BUFFER_BINDING),
            ("renderbuffer", glow::RENDERBUFFER_BINDING),
        ] {
            put(name, (gl.get_parameter_i32(parameter) != 0).to_string());
        }
        // Which units hold a 2D texture, then back to the unit that was active.
        let active = gl.get_parameter_i32(glow::ACTIVE_TEXTURE) as u32;
        let bound: Vec<u32> = (0..16)
            .filter(|unit| {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.get_parameter_i32(glow::TEXTURE_BINDING_2D) != 0
            })
            .collect();
        gl.active_texture(active);
        put("texture_2d_units", format!("{bound:?}"));
    }
    out
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
        TagSource::LooseFolder { definitions_root, .. } => {
            TagNameIndex::load_game(definitions_root, options.game).unwrap()
        }
        _ => unreachable!(),
    };
    let TagSource::LooseFolder { root, .. } = source else { unreachable!() };
    // A sky has a render_model and nothing above it.
    let file = root.join(format!("{}.model", shot.model.replace('\\', "/")));
    let (extension, group) = if let Some(kind) = shot.kind.clone().or_else(|| options.kind.clone()) {
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
        h2_equippable: None,
        ..Default::default()
    };
    let mut data = load_model_preview(&tag, &entry, &names, Some(source), &settings).unwrap();
    if options.no_game {
        Arc::make_mut(&mut data.preview).game = None;
    }
    let textures = materials::resolve_model_textures(source, &data.preview.materials);
    let with_passes = textures.iter().filter(|t| t.h2_layers.iter().any(|l| !l.is_empty())).count();
    eprintln!("{}: {} materials, {with_passes} with passes", shot.model, textures.len());
    for (material, texture) in data.preview.materials.iter().zip(textures.iter()) {
        eprintln!("NOTE MAT {} h3 {} {:?}", material.shader_path, texture.h3.is_some(), texture.error);
        // `H4_TEXTURES=1`: each pass's samplers, and whether one is bound.
        if options.report_textures
            && let Some(h3) = &texture.h3
        {
            let passes = h3.albedo.iter().chain(std::iter::once(&h3.lit));
            for (name, kind) in passes.flat_map(|pass| pass.textures.iter()) {
                let bound = h3.textures.iter().find(|known| &known.uniform == name);
                let detail = bound.map(|known| format!("{:?} {:?}", known.texels, known.sampler)).unwrap_or_else(|| "UNBOUND".to_owned());
                eprintln!("NOTE   TEX {name} {kind:?} {detail}");
            }
        }
    }
    data.textures = Some(Arc::new(textures));
    eprintln!("NOTE {} cheap particle emitters", data.preview.gen3.cheap_emitters.len());
    eprintln!(
        "NOTE H4 {} lights, {} cheap lights, {} light passes, {} flares, {} particle systems ({} on materials), {} light volumes ({} on materials), {} tracers",
        data.preview.h4.lights.len(),
        data.preview.h4.cheap_lights.len(),
        data.preview.h4.light_passes.len(),
        data.preview.h4.flares.len(),
        data.preview.particles.len(),
        data.preview.particles.iter().filter(|system| system.look.h4.is_some()).count(),
        data.preview.gen3.light_volumes.len(),
        data.preview.gen3.light_volumes.iter().filter(|volume| volume.h4_material().is_some()).count(),
        data.preview.h4.tracers.len()
    );
    // Whole features left out, to see one on its own.
    if !options.dropped.is_empty() {
        let preview = Arc::make_mut(&mut data.preview);
        for feature in &options.dropped {
            match feature.as_str() {
                "particles" => preview.particles.clear(),
                "flares" => preview.h2.flares.clear(),
                "lights" => preview.h2.point_lights.clear(),
                "gels" => preview.h2.gel_lights.clear(),
                "volumes" => preview.h2.light_volumes.clear(),
                "h3_volumes" => preview.gen3.light_volumes.clear(),
                "h4_tracers" => preview.h4.tracers.clear(),
                "cheap_particles" => preview.gen3.cheap_emitters.clear(),
                "contrails" => preview.h2.contrails.clear(),
                "prt" => preview.gen3.prt = None,
                "shield" => preview.reach.shield = None,
                "antennas" => preview.antennas.clear(),
                // The sheet's own batches go with it; its vertices stay, unused.
                "cloth" => {
                    preview.cloth.clear();
                    preview.batches.retain(|batch| batch.permutation_name != "cloth");
                }
                "reach_lights" => {
                    preview.reach.lights.clear();
                    preview.reach.cheap_lights.clear();
                }
                "h4_lights" => {
                    preview.h4.lights.clear();
                    preview.h4.cheap_lights.clear();
                }
                "h4_flares" => preview.h4.flares.clear(),
                other => panic!("H2_SHOT_DROP: no feature {other}"),
            }
        }
    }
    let mut state = ModelPreviewState { h2_light_follows_camera: !options.fixed_light, ..Default::default() };
    let variant = options.variant.as_ref();
    let variant = variant.and_then(|name| data.variants.iter().position(|variant| &variant.name == name));
    reset_model_preview_selection(&mut state, &data, variant.or(default_variant_index(&data)));
    state.h2_game_shaders = shot.shaders;
    state.h2_default_lights = shot.lights;
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
    state.h2_active_camo = options.camo;
    state.h2_bloom = options.bloom;
    state.h2_self_shadow = options.shadow;
    state.h2_prt = !options.no_prt;
    if let Some(values) = options.inputs {
        state.ce_function_inputs.insert((data.geometry_id, 0), values);
    }
    for (name, value) in &options.functions {
        state.h2_function_inputs.insert((data.geometry_id, name.clone()), *value);
    }
    let background = preview_background(&if options.light_theme { egui::Visuals::light() } else { egui::Visuals::dark() });
    let mut frame = super::build_frame(&data, &state, rect, options.time, background, &options.forced);
    let preview = Arc::clone(&frame.preview);
    frame.visible_batches.retain(|&batch| {
        let material = preview.batches[batch].material_index as usize;
        !options.hidden.iter().any(|name| preview.materials[material].shader_path.ends_with(name.as_str()))
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
        gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(color));
        gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_ATTACHMENT, glow::RENDERBUFFER, Some(depth));
        assert_eq!(gl.check_framebuffer_status(glow::FRAMEBUFFER), glow::FRAMEBUFFER_COMPLETE);
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
        if let Some((play, fire)) = play.or_else(|| options.fire.as_ref().map(|fire| (fire, true))) {
            let (text, before) = play.split_once('@').expect("a play or fire is <what>@<seconds>");
            let end = frame.time;
            frame.time = end - before.parse::<f32>().unwrap();
            renderer.paint(gl, &frame);
            if fire {
                let weapon = data.preview.ce.firing.last().expect("a weapon to fire");
                let fired = state.ce_shots.entry((data.geometry_id, weapon.object)).or_default();
                let effect = fired.fire(weapon, text.parse().unwrap(), state.ce_event_scales[0], frame.time);
                eprintln!("NOTE fired, playing {:?}", effect.and_then(|at| data.preview.ce.effects[at].event.as_ref()));
                if let Some(effect) = effect {
                    frame.ce_plays.counts[effect] += 1;
                }
            }
            for (index, effect) in data.preview.ce.effects.iter().enumerate() {
                if let Some(event) = effect.event.as_ref().filter(|event| !fire && event.name.contains(text)) {
                    eprintln!("NOTE playing {}", event.name);
                    frame.ce_plays.counts[index] += 1;
                }
            }
            // Step to the shot a frame at a time, as the app would.
            let mut at = frame.time;
            while at < end {
                at = (at + 1.0 / 60.0).min(end);
                frame.time = at;
                frame.object_states = object_states(&data, &state.ce_function_inputs, &state.ce_shots, at);
                frame.ce_points = frame.ce_point_lights();
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                renderer.paint(gl, &frame);
            }
            eprintln!("NOTE states {:?}", frame.object_states.iter().map(|state| state.outputs).collect::<Vec<_>>());
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
        Rendered { pixels, geometry_id: data.geometry_id, gl_state }
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
}

/// Takes each shot in one headless context, from the kit `source` reads.
pub(in crate::app::model_preview) fn shoot(source: &TagSource, shots: &[(Shot, ShotOptions)]) -> Vec<Shown> {
    let gl = Arc::new(headless_gl());
    let mut renderer = ModelGlRenderer::new(&gl).unwrap();
    shots
        .iter()
        .map(|(shot, options)| {
            let Rendered { pixels, geometry_id, gl_state } = render(&gl, &mut renderer, source, shot, options);
            let mut simulation = String::new();
            renderer.for_each_simulation(geometry_id, |name, state| {
                super::super::gates::field_lines(&mut simulation, name, &state);
            });
            Shown { pixels, simulation, gl_state }
        })
        .collect()
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
        std::fs::write(f[1], render(&gl, &mut renderer, &source, &shot, &options).pixels).unwrap();
    }
}

/// Reach's `shadow_apply` links, and reads every input it is given.
#[test]
fn reach_shadow_apply_links() {
    let gl = headless_gl();
    let program = unsafe { super::super::gen3::pipeline::screen_program(&gl, super::super::reach::shadow_pass::APPLY_FRAGMENT) }
        .expect("the apply compiles and links");
    for name in ["scene_depth", "normals", "shadow_map", "screen_to_world", "shadow_view_projection", "hull_planes"] {
        assert!(unsafe { gl.get_uniform_location(program, name) }.is_some(), "{name} is read");
    }
}

/// Reach's shield passes, 61 and 119, translate with the preview's depth
/// read and the oscillation maps in their vertex stage, and link.
/// `BLAM_TEST_HREK` to the tags folder.
#[test]
fn reach_shield_passes_link() {
    let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
        return;
    };
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::HaloReach),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let model = load_referenced_tag_from_source(&source, "objects\\characters\\spartans\\spartans", "model", b"hlmt").unwrap();
    let shield = super::reach::shield::read_reach_shield(&source, &model).expect("the Spartan's shield");
    let gl = headless_gl();
    let passes: Vec<_> = shield.passes().collect();
    assert_eq!(passes.len(), 2, "both explicit shaders");
    for pass in passes {
        assert!(pass.pixel.contains(super::gen3::shield::DEPTH_DECODE), "the depth read is the preview's");
        assert!(!pass.pixel.contains("depth_constants.z - "), "Reach's own depth read is gone");
        assert!(!pass.vertex.contains("GlobalTexture_shield_impact_noise_texture"), "the vertex stage reads oscillation");
        unsafe {
            let stages = [(glow::VERTEX_SHADER, pass.vertex.as_str()), (glow::FRAGMENT_SHADER, pass.pixel.as_str())];
            let program = link(&gl, &stages, &[]).unwrap_or_else(|error| panic!("{error}"));
            for (name, _) in &pass.textures {
                let location = gl.get_uniform_location(program, name);
                eprintln!("NOTE shield TEXTURE {name} {}", location.is_some());
            }
        }
    }
}

/// Reach's four `light_apply` passes translate and link. `BLAM_TEST_HREK`
/// to the tags folder.
#[test]
fn reach_light_passes_link() {
    let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
        return;
    };
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::HaloReach),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    let mut sources = super::gen3::shaders::H3Sources::new(&source);
    for file in ["light_apply", "light_apply_frustum", "light_apply_specular", "light_apply_frustum_specular"] {
        let pass = sources.reach_light_pass(file).unwrap_or_else(|error| panic!("{file}: {error}"));
        for (name, _) in &pass.uniforms {
            eprintln!("NOTE {file} UNIFORM {name}");
        }
        for (name, _) in &pass.textures {
            eprintln!("NOTE {file} TEXTURE {name}");
        }
        unsafe {
            let stages = [(glow::VERTEX_SHADER, pass.vertex.as_str()), (glow::FRAGMENT_SHADER, pass.pixel.as_str())];
            link(&gl, &stages, &[]).unwrap_or_else(|error| panic!("{file}: {error}"));
        }
    }
}

/// Halo 4's screen-space light passes, explicit 187 and 204's sixteen
/// techniques each, compile and link; `BLAM_TEST_H4EK` to the tags folder.
#[test]
fn h4_light_passes_link() {
    let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
        return;
    };
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    for physical in [false, true] {
        let program = super::h4::lights::h4_light_program(&source, physical).expect("the light shader");
        for technique in 0..16 {
            let pass = super::h4::lights::h4_light_pass(&program, technique).unwrap_or_else(|error| panic!("{physical} {technique}: {error}"));
            unsafe {
                let stages = [(glow::VERTEX_SHADER, pass.vertex.as_str()), (glow::FRAGMENT_SHADER, pass.pixel.as_str())];
                link(&gl, &stages, &[]).unwrap_or_else(|error| panic!("{physical} {technique}: {error}"));
            }
        }
    }
}

/// Halo 4's lens flare pass, explicit 46, compiles and links, and takes its
/// corners and texture coordinates as `POSITION` and `TEXCOORD0`.
#[test]
fn h4_flare_pass_links() {
    let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
        return;
    };
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    let pass = super::h4::flares::h4_flare_pass(&source).expect("the flare pass");
    for (name, _) in &pass.uniforms {
        eprintln!("NOTE flare UNIFORM {name}");
    }
    assert!(pass.vertex.contains("in vec2 a_POSITION;") && pass.vertex.contains("a_TEXCOORD0"), "{}", pass.vertex);
    unsafe {
        let stages = [(glow::VERTEX_SHADER, pass.vertex.as_str()), (glow::FRAGMENT_SHADER, pass.pixel.as_str())];
        link(&gl, &stages, &[]).unwrap_or_else(|error| panic!("{error}"));
    }
}

/// Every kit effect material shader's vertex passes link with its `_default`
/// pixel shader; `BLAM_TEST_H4EK` to the tags folder.
#[test]
fn h4_effect_passes_link() {
    use super::h4::shaders::{H4_DEFINES, H4ShaderSource, translate_pass};
    use crate::core::hlsl::emit::Stage;
    let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
        return;
    };
    let folder = std::path::Path::new(&root).join("shaders/material_shaders/fx");
    let source = TagSource::LooseFolder {
        root: std::path::PathBuf::from(&root),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    let mut failures = Vec::new();
    let mut linked = 0;
    for entry in std::fs::read_dir(&folder).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".material_shader") else { continue };
        let path = format!("shaders\\material_shaders\\fx\\{stem}");
        let program = H4ShaderSource::read(&source, &path).and_then(|shader| shader.program(&H4_DEFINES)).unwrap();
        let Some(technique) = program.techniques.iter().find(|technique| &*technique.name == "_default") else { continue };
        let Ok(pixel) = translate_pass(&program, "_default", "_default", Stage::Pixel) else { continue };
        for (pass, _) in technique.passes.iter().filter(|(pass, _)| &**pass != "_default") {
            let vertex = translate_pass(&program, "_default", pass, Stage::Vertex).unwrap();
            let both = super::gen3::shaders::pass_of(vertex, pixel.clone());
            unsafe {
                let program = gl.create_program().unwrap();
                let mut ok = true;
                for (kind, text) in [(glow::VERTEX_SHADER, &both.vertex), (glow::FRAGMENT_SHADER, &both.pixel)] {
                    let shader = gl.create_shader(kind).unwrap();
                    gl.shader_source(shader, text);
                    gl.compile_shader(shader);
                    if !gl.get_shader_compile_status(shader) {
                        let log = gl.get_shader_info_log(shader);
                        let line = log.lines().next().unwrap_or("").to_owned();
                        let at = line.split(':').nth(2).and_then(|n| n.trim().parse::<usize>().ok()).unwrap_or(0);
                        let bad = text.lines().nth(at.saturating_sub(1)).unwrap_or("").to_owned();
                        failures.push(format!("{stem}/{pass} {}: {line}\n    {bad}", if kind == glow::VERTEX_SHADER { "vs" } else { "ps" }));
                        ok = false;
                    }
                    gl.attach_shader(program, shader);
                }
                if ok {
                    gl.link_program(program);
                    if gl.get_program_link_status(program) {
                        linked += 1;
                    } else {
                        failures.push(format!("{stem}/{pass} link: {}", gl.get_program_info_log(program)));
                    }
                }
            }
        }
    }
    for failure in &failures {
        eprintln!("NOTE FAIL {failure}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
    assert!(linked > 30, "linked {linked}");
}

/// What one pass reads: `H3_PASS=<family>|<template>|<entry>` and
/// `BLAM_TEST_H3EK` to the tags folder.
#[test]
fn h3_pass_inputs() {
    use super::gen3::shaders::{H3Sources, VERTEX_SKINNED, read_h3_definition};
    let Ok(spec) = std::env::var("H3_PASS") else {
        return;
    };
    let root = std::env::var("BLAM_TEST_H3EK").unwrap();
    let game = if root.contains("reach") { GameId::HaloReach } else { GameId::Halo3 };
    let source = TagSource::LooseFolder {
        root: root.clone().into(),
        game: Some(game),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    // `explicit|<file>` translates one of the explicit shaders.
    let pass = if let Some(file) = spec.strip_prefix("explicit|") {
        H3Sources::new(&source).explicit_pass(file).unwrap()
    } else {
        let [family, template, entry]: [&str; 3] = spec.split('|').collect::<Vec<_>>().try_into().unwrap();
        let definition = read_h3_definition(&source, &format!("shaders\\{family}")).unwrap();
        let options: Vec<usize> = template.split('_').filter_map(|digit| digit.parse().ok()).collect();
        Arc::new(H3Sources::new(&source).translate_entry(&definition, &options, entry, VERTEX_SKINNED).unwrap())
    };
    for (name, ty) in &pass.uniforms {
        eprintln!("NOTE UNIFORM {name} {ty:?}");
    }
    for (name, kind) in &pass.textures {
        eprintln!("NOTE TEXTURE {name} {kind:?}");
    }
    if std::env::var_os("H3_PASS_SOURCE").is_some() {
        eprintln!("{}\n-----\n{}", pass.vertex, pass.pixel);
    }
}

/// Every template of a family through translation and the driver. Set
/// `H3_SWEEP=<limit>` and `BLAM_TEST_H3EK` to the tags folder.
#[test]
fn h3_sweep() {
    use super::gen3::shaders::{H3Sources, VERTEX_RIGID, VERTEX_SKINNED, read_h3_definition};
    let Ok(limit) = std::env::var("H3_SWEEP") else {
        return;
    };
    let limit: usize = limit.parse().unwrap();
    let root = std::env::var("BLAM_TEST_H3EK").unwrap();
    let family = std::env::var("H3_FAMILY").unwrap_or_else(|_| "shader".to_owned());
    let source = TagSource::LooseFolder {
        root: root.clone().into(),
        game: Some(if root.contains("odst") {
            GameId::Halo3Odst
        } else if root.contains("reach") {
            GameId::HaloReach
        } else {
            GameId::Halo3
        }),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    let definition = read_h3_definition(&source, &format!("shaders\\{family}")).unwrap();
    let mut sources = H3Sources::new(&source);
    let folder = format!("{root}/shaders/{family}_templates");
    let mut templates: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter_map(|name| name.strip_suffix(".render_method_template").map(str::to_owned))
        .collect();
    templates.sort();
    let (mut good, mut translation, mut compile) = (0, 0, 0);
    let mut seen: HashMap<String, usize> = HashMap::new();
    for template in templates.iter().take(limit) {
        let options: Vec<usize> = template.split('_').filter_map(|digit| digit.parse().ok()).collect();
        for entry in &definition.entry_points {
            for vertex_type in [VERTEX_RIGID, VERTEX_SKINNED] {
                match sources.translate_entry(&definition, &options, entry, vertex_type) {
                    Err(error) => {
                        translation += 1;
                        let key: String = error.chars().take(140).collect();
                        if *seen.entry(key.clone()).and_modify(|count| *count += 1).or_insert(1) == 1 {
                            eprintln!("NOTE TR {template} {entry}: {key}");
                        }
                    }
                    Ok(pass) => {
                        let mut failed = false;
                        let program = unsafe { gl.create_program().unwrap() };
                        let mut shaders = Vec::new();
                        for (kind, text) in [(glow::VERTEX_SHADER, &pass.vertex), (glow::FRAGMENT_SHADER, &pass.pixel)] {
                            unsafe {
                                let shader = gl.create_shader(kind).unwrap();
                                shaders.push(shader);
                                gl.shader_source(shader, text);
                                gl.compile_shader(shader);
                                if !gl.get_shader_compile_status(shader) {
                                    failed = true;
                                    let log = gl.get_shader_info_log(shader);
                                    let first = log.lines().next().unwrap_or("").to_owned();
                                    if let Some(line) = first.split(':').nth(2).and_then(|n| n.trim().parse::<usize>().ok()) {
                                        let bad = text.lines().nth(line - 1).unwrap_or("");
                                        let key: String = first.chars().skip(8).filter(|c| !c.is_ascii_digit()).take(70).collect();
                                        if *seen.entry(key).and_modify(|count| *count += 1).or_insert(1) == 1 {
                                            eprintln!("NOTE GL {template} {entry}: {first}\n    {}", bad.chars().take(170).collect::<String>());
                                        }
                                    }
                                }
                                gl.attach_shader(program, shader);
                            }
                        }
                        // A stage built from other options than its partner can
                        // disagree on what passes between them.
                        unsafe {
                            if !failed {
                                gl.link_program(program);
                                if !gl.get_program_link_status(program) {
                                    failed = true;
                                    let log = gl.get_program_info_log(program);
                                    let key: String = log.lines().next().unwrap_or("").chars().take(120).collect();
                                    if *seen.entry(format!("LINK {key}")).and_modify(|count| *count += 1).or_insert(1) == 1 {
                                        eprintln!("NOTE LINK {template} {entry}: {key}");
                                    }
                                }
                            }
                            for shader in shaders {
                                gl.delete_shader(shader);
                            }
                            gl.delete_program(program);
                        }
                        if failed { compile += 1 } else { good += 1 }
                    }
                }
            }
        }
    }
    let mut common: Vec<_> = seen.into_iter().collect();
    common.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for (error, count) in common.iter().take(12) {
        eprintln!("NOTE TOP {count} {error}");
    }
    eprintln!("NOTE SWEEP good {good} translation {translation} compile {compile}");
}

/// Every Halo 4 material shader's object passes through translation and the
/// driver. Set `H4_SWEEP=<limit>` and `BLAM_TEST_H4EK` to the tags folder;
/// `H4_FOLDER` narrows to a folder under `shaders\material_shaders`.
#[test]
fn h4_sweep() {
    use super::h4::shaders::{H4_DEFINES, H4ShaderSource, translate_pass};
    use crate::core::hlsl::emit::Stage;
    let Ok(limit) = std::env::var("H4_SWEEP") else {
        return;
    };
    let limit: usize = limit.parse().unwrap();
    let root = std::env::var("BLAM_TEST_H4EK").unwrap();
    let folder = std::env::var("H4_FOLDER").unwrap_or_else(|_| "materials".to_owned());
    let techniques: Vec<String> = std::env::var("H4_TECHNIQUES")
        .unwrap_or_else(|_| "albedo,static_probe,single_pass_single_probe".to_owned())
        .split(',')
        .map(str::to_owned)
        .collect();
    let source = TagSource::LooseFolder {
        root: root.clone().into(),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    let mut tags = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(format!("{root}/shaders/material_shaders/{folder}"))];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "material_shader") {
                let relative = path.strip_prefix(&root).unwrap().with_extension("");
                tags.push(relative.to_string_lossy().replace('/', "\\"));
            }
        }
    }
    tags.sort();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut note = |kind: &str, tag: &str, what: &str, error: &str| {
        let key: String = format!("{kind} {}", error.chars().take(140).collect::<String>());
        if *seen.entry(key.clone()).and_modify(|count| *count += 1).or_insert(1) == 1 {
            eprintln!("NOTE {kind} {tag} {what}: {}", error.chars().take(400).collect::<String>());
        }
    };
    let compile = |kind: u32, text: &str| -> Result<glow::NativeShader, String> {
        unsafe {
            let shader = gl.create_shader(kind).unwrap();
            gl.shader_source(shader, text);
            gl.compile_shader(shader);
            if gl.get_shader_compile_status(shader) {
                return Ok(shader);
            }
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            let first = log.lines().next().unwrap_or("").to_owned();
            let bad = first
                .split(':')
                .nth(2)
                .and_then(|line| line.trim().parse::<usize>().ok())
                .and_then(|line| text.lines().nth(line - 1))
                .unwrap_or("");
            Err(format!("{first} || {}", bad.trim()))
        }
    };
    if let Ok(needle) = std::env::var("H4_DUMP") {
        let shader = H4ShaderSource::read(&source, &tags[0]).unwrap();
        let tokens = shader.tokens(&H4_DEFINES).unwrap();
        let text: Vec<&str> = tokens.iter().map(|token| &*token.text).collect();
        let at = text.iter().position(|token| *token == needle).unwrap_or(0);
        eprintln!("NOTE DUMP {}", text[at.saturating_sub(80)..(at + 40).min(text.len())].join(" "));
        return;
    }
    let (mut good, mut parse_failed, mut translation, mut driver) = (0, 0, 0, 0);
    for tag in tags.iter().take(limit) {
        let program = match H4ShaderSource::read(&source, tag).and_then(|shader| shader.program(&H4_DEFINES)) {
            Ok(program) => program,
            Err(error) => {
                parse_failed += 1;
                note("PARSE", tag, "", &error);
                continue;
            }
        };
        for technique in &techniques {
            if !program.techniques.iter().any(|candidate| &*candidate.name == technique.as_str()) {
                continue;
            }
            let pixel = match translate_pass(&program, technique, "_default", Stage::Pixel) {
                Ok(pixel) => pixel,
                Err(error) => {
                    translation += 1;
                    note("TR", tag, &format!("{technique} ps"), &error);
                    continue;
                }
            };
            let Ok(pixel_shader) = compile(glow::FRAGMENT_SHADER, &pixel.source).map_err(|error| {
                driver += 1;
                note("GL", tag, &format!("{technique} ps"), &error);
            }) else {
                continue;
            };
            for pass in ["rigid", "skinned", "rigid_boned"] {
                let vertex = match translate_pass(&program, technique, pass, Stage::Vertex) {
                    Ok(vertex) => vertex,
                    Err(error) => {
                        translation += 1;
                        note("TR", tag, &format!("{technique} {pass}"), &error);
                        continue;
                    }
                };
                let vertex_shader = match compile(glow::VERTEX_SHADER, &vertex.source) {
                    Ok(shader) => shader,
                    Err(error) => {
                        driver += 1;
                        note("GL", tag, &format!("{technique} {pass}"), &error);
                        continue;
                    }
                };
                unsafe {
                    let linked = gl.create_program().unwrap();
                    gl.attach_shader(linked, vertex_shader);
                    gl.attach_shader(linked, pixel_shader);
                    gl.link_program(linked);
                    if gl.get_program_link_status(linked) {
                        good += 1;
                    } else {
                        driver += 1;
                        note("LINK", tag, &format!("{technique} {pass}"), &gl.get_program_info_log(linked));
                    }
                    gl.delete_program(linked);
                    gl.delete_shader(vertex_shader);
                }
            }
            unsafe { gl.delete_shader(pixel_shader) };
        }
    }
    let mut common: Vec<_> = seen.into_iter().collect();
    common.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for (error, count) in common.iter().take(15) {
        eprintln!("NOTE TOP {count} {error}");
    }
    eprintln!("NOTE SWEEP {} tags: good {good} parse {parse_failed} translation {translation} driver {driver}", tags.len().min(limit));
}

/// What one Halo 4 pass reads: `H4_PASS=<material_shader>|<technique>|<vertex pass>`
/// and `BLAM_TEST_H4EK`. `H4_PASS_SOURCE` prints the GLSL too.
#[test]
fn h4_pass_inputs() {
    use super::h4::shaders::{H4_DEFINES, H4ShaderSource, translate_pass};
    use crate::core::hlsl::emit::Stage;
    let Ok(spec) = std::env::var("H4_PASS") else {
        return;
    };
    let root = std::env::var("BLAM_TEST_H4EK").unwrap();
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let [shader, technique, pass]: [&str; 3] = spec.split('|').collect::<Vec<_>>().try_into().unwrap();
    let program = H4ShaderSource::read(&source, shader).unwrap().program(&H4_DEFINES).unwrap();
    for technique in &program.techniques {
        let passes: Vec<String> = technique
            .passes
            .iter()
            .map(|(name, shaders)| format!("{name}[{}]", shaders.iter().map(|shader| &*shader.stage).collect::<Vec<_>>().join(",")))
            .collect();
        eprintln!("NOTE TECHNIQUE {} {}", technique.name, passes.join(" "));
    }
    for stage in [Stage::Vertex, Stage::Pixel] {
        let translated = translate_pass(&program, technique, pass, stage).unwrap();
        for (name, ty) in &translated.uniforms {
            eprintln!("NOTE {stage:?} UNIFORM {name} {ty:?}");
        }
        for (name, kind) in &translated.textures {
            eprintln!("NOTE {stage:?} TEXTURE {name} {kind:?}");
        }
        for attribute in &translated.attributes {
            eprintln!("NOTE {stage:?} IO {} {} {:?}", attribute.semantic, attribute.glsl, attribute.ty);
        }
        if std::env::var_os("H4_PASS_SOURCE").is_some() {
            eprintln!("{}", translated.source);
        }
    }
}

/// Every Halo 4 `.model` through the shot harness, each caught on its own,
/// or with `H4_KIT_OBJECTS=1` every object tag by its kind. Set
/// `H4_KIT=<limit>`, `H2_SHOT_H4=1` and `BLAM_TEST_H2EK` to the tags folder;
/// translation and link failures print as `h3 program failed`.
#[test]
fn h4_kit_sweep() {
    let (Ok(limit), Ok(root)) = (std::env::var("H4_KIT"), std::env::var("BLAM_TEST_H2EK")) else {
        return;
    };
    let limit: usize = limit.parse().unwrap();
    let source = TagSource::LooseFolder {
        root: root.clone().into(),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    // `H4_KIT_OBJECTS=1` takes the object tags instead, each by its kind:
    // their attachments, held weapons and effects load as well.
    let objects = std::env::var_os("H4_KIT_OBJECTS").is_some();
    const KINDS: [(&str, &str); 11] = [
        ("biped", "biped"),
        ("vehicle", "vehicle"),
        ("weapon", "weapon"),
        ("equipment", "equipment"),
        ("scenery", "scenery"),
        ("crate", "crate"),
        ("device_machine", "machine"),
        ("device_control", "control"),
        ("effect_scenery", "effect_scenery"),
        ("giant", "giant"),
        ("projectile", "projectile"),
    ];
    let mut models: Vec<(String, Option<String>)> = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(&root)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                continue;
            };
            let kind = if objects {
                match KINDS.iter().find(|(known, _)| *known == extension) {
                    Some((_, kind)) => Some((*kind).to_owned()),
                    None => continue,
                }
            } else if extension == "model" {
                None
            } else {
                continue;
            };
            let relative = path.strip_prefix(&root).unwrap().with_extension("");
            models.push((relative.to_string_lossy().replace('/', "\\"), kind));
        }
    }
    models.sort();
    let gl = Arc::new(headless_gl());
    let mut renderer = ModelGlRenderer::new(&gl).unwrap();
    let options = ShotOptions::new(GameId::Halo4);
    let (mut good, mut failed) = (0, Vec::new());
    for (model, kind) in models.iter().take(limit) {
        let shot = Shot {
            model: model.clone(),
            shaders: true,
            lights: true,
            yaw: -1.57,
            pitch: 0.3,
            kind: kind.clone(),
        };
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(&gl, &mut renderer, &source, &shot, &options)));
        match caught {
            Ok(_) => good += 1,
            Err(panic) => {
                let detail = crate::app::shell::worker::panic_detail(&*panic);
                eprintln!("NOTE FAIL {model}: {}", detail.chars().take(200).collect::<String>());
                failed.push(model.clone());
                // A panic mid-frame can leave state bound; start clean.
                renderer = ModelGlRenderer::new(&gl).unwrap();
            }
        }
    }
    eprintln!("NOTE KIT {} models: good {good}, failed {}", models.len().min(limit), failed.len());
}

/// The Chief biped, with the AR, posed at `clip`, from the kit at `root`.
fn h4_chief_with_rifle(root: &str, clip: &str) -> (ModelPreviewData, ModelPreviewState) {
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let TagSource::LooseFolder { definitions_root, .. } = &source else { unreachable!() };
    let names = TagNameIndex::load_game(definitions_root, GameId::Halo4).unwrap();
    let path = "objects\\characters\\storm_masterchief\\storm_masterchief";
    let tag = load_referenced_tag_from_source(&source, path, "biped", b"bipd").unwrap();
    let file = std::path::Path::new(&root).join("objects/characters/storm_masterchief/storm_masterchief.biped");
    let entry = TagEntry {
        key: format!("{path}.biped"),
        display_path: format!("{path}.biped"),
        group_tag: u32::from_be_bytes(*b"bipd"),
        group_name: Some("biped".to_owned()),
        location: TagEntryLocation::LooseFile(file),
    };
    let weapon = "objects\\weapons\\rifle\\storm_assault_rifle\\storm_assault_rifle".to_owned();
    let settings = PreviewLoadSettings { weapon: Some(weapon.clone()), ..Default::default() };
    let mut data = load_model_preview(&tag, &entry, &names, Some(&source), &settings).unwrap();
    let mut state = ModelPreviewState::default();
    reset_model_preview_selection(&mut state, &data, default_variant_index(&data));
    state.weapon = Some(weapon);
    animation::pose_for_shot(&source, &entry, &mut data, &mut state, clip);
    (data, state)
}

/// With `BLAM_TEST_H4EK` (the kit's tags): the Chief holding the AR in
/// `combat:rifle:idle` puts each IK set point's marker on the weapon's, the
/// left hand on `left_hand_spartan`, within a millimetre.
#[test]
fn h4_chief_left_hand_holds_the_assault_rifle() {
    let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
        return;
    };
    let (data, mut state) = h4_chief_with_rifle(&root, "combat:rifle:idle");
    let points = state.animation.pose.as_ref().unwrap().h4_ik_set.clone();
    assert!(!points.is_empty(), "the clip's IK set");
    let rows = animation::animation_skinning_rows(&data, &state).expect("a posed frame");
    let posed = |marker: &super::animation::reach_ik::ReachIkMarker| {
        let skin = &rows[marker.node * 3..marker.node * 3 + 3];
        let at = [marker.bind.m[0][3], marker.bind.m[1][3], marker.bind.m[2][3]];
        std::array::from_fn::<f32, 3, _>(|row| skin[row][0] * at[0] + skin[row][1] * at[1] + skin[row][2] * at[2] + skin[row][3])
    };
    let markers = &data.preview.gen3.ik_markers;
    let find = |holder, name: &str| markers.iter().find(|marker| marker.holder == holder && marker.name.eq_ignore_ascii_case(name));
    let mut solved = 0;
    for point in points.iter() {
        let (Some(source), Some(destination)) =
            (find(super::animation::reach_ik::ReachIkHolder::Unit, &point.source), find(point.holder, &point.destination))
        else {
            continue;
        };
        let (from, to) = (posed(source), posed(destination));
        let apart = (0..3).map(|axis| (from[axis] - to[axis]).powi(2)).sum::<f32>().sqrt();
        eprintln!("NOTE IK {} -> {} apart {apart}", point.source, point.destination);
        // A millimetre is 1/3048 of a world unit.
        assert!(apart < 1.0 / 3048.0, "{} misses {} by {apart}", point.source, point.destination);
        solved += 1;
    }
    assert!(solved > 0, "no point found both markers: {points:?}");
    // Without the set the animation alone leaves the hand off the grip.
    let mut pose = (**state.animation.pose.as_ref().unwrap()).clone();
    pose.h4_ik_set = Arc::default();
    state.animation.pose = Some(Arc::new(pose));
    let rows = animation::animation_skinning_rows(&data, &state).expect("a posed frame");
    let posed = |marker: &super::animation::reach_ik::ReachIkMarker| {
        let skin = &rows[marker.node * 3..marker.node * 3 + 3];
        let at = [marker.bind.m[0][3], marker.bind.m[1][3], marker.bind.m[2][3]];
        std::array::from_fn::<f32, 3, _>(|row| skin[row][0] * at[0] + skin[row][1] * at[1] + skin[row][2] * at[2] + skin[row][3])
    };
    let point = &points[0];
    let (from, to) = (posed(find(super::animation::reach_ik::ReachIkHolder::Unit, &point.source).unwrap()), posed(find(point.holder, &point.destination).unwrap()));
    let apart = (0..3).map(|axis| (from[axis] - to[axis]).powi(2)).sum::<f32>().sqrt();
    eprintln!("NOTE IK off: apart {apart}");
    assert!(apart > 1.0 / 3048.0, "the clip alone already reaches the grip ({apart}), so this proves nothing");
}

/// With `BLAM_TEST_H4EK`: the AR reload's chain events, whose unused data
/// indices are 0xFF, become chain tracks on the held weapon and solve.
#[test]
fn h4_chief_reload_reads_its_ik_chains() {
    let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
        return;
    };
    let (data, state) = h4_chief_with_rifle(&root, "combat:rifle:ar:reload_1");
    let pose = state.animation.pose.as_ref().unwrap();
    assert!(!pose.reach_ik.is_empty(), "the reload's chains");
    assert!(pose.reach_ik.iter().any(|track| track.holder == super::animation::reach_ik::ReachIkHolder::Weapon(1)));
    let rows = animation::animation_skinning_rows(&data, &state).expect("a posed frame");
    assert!(rows.iter().flatten().all(|lane| lane.is_finite()));
}

/// `link` leaves no shader behind, and on failure no program: a stage that
/// will not compile and a pair that will not link both come back as errors,
/// with every name they made deleted.
#[test]
fn link_cleans_up_after_a_failed_stage_and_a_failed_link() {
    const VERTEX: &str = "#version 410\nin vec2 a;\nout vec2 v;\nvoid main() { v = a; gl_Position = vec4(a, 0.0, 1.0); }\n";
    const FRAGMENT: &str = "#version 410\nin vec2 v;\nout vec4 colour;\nvoid main() { colour = vec4(v, 0.0, 1.0); }\n";
    const UNMATCHED: &str = "#version 410\nin vec3 elsewhere;\nout vec4 colour;\nvoid main() { colour = vec4(elsewhere, 1.0); }\n";
    let gl = headless_gl();
    // Shaders and programs share one run of names, from 1.
    let held = |gl: &glow::Context| -> Vec<u32> {
        (1..=32u32)
            .filter(|name| unsafe {
                let name = std::num::NonZeroU32::new(*name).unwrap();
                gl.is_shader(glow::NativeShader(name)) || gl.is_program(glow::NativeProgram(name))
            })
            .collect()
    };
    unsafe {
        let program = link(&gl, &[(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)], &[(0, "a")]).unwrap();
        assert_eq!(gl.get_attrib_location(program, "a"), Some(0));
        assert_eq!(held(&gl), [program.0.get()], "the stages are gone once linked");
        gl.delete_program(program);
        let broken = link(&gl, &[(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, "#version 410\nvoid main() { nonsense }\n")], &[]);
        assert!(broken.as_ref().is_err_and(|error| error.starts_with("fragment shader failed to compile")), "{broken:?}");
        let unlinked = link(&gl, &[(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, UNMATCHED)], &[]);
        assert!(unlinked.as_ref().is_err_and(|error| error.starts_with("failed to link")), "{unlinked:?}");
        assert_eq!(held(&gl), Vec::<u32>::new(), "nothing made on the way is still held");
    }
}

/// What [`Made`](super::super::gl::target::Made) builds goes when it is
/// dropped half done, and stays once kept.
#[test]
fn made_targets_are_deleted_unless_kept() {
    use super::super::gl::target::{Made, Sampling, Texels};
    let gl = headless_gl();
    // Each kind numbers its own names from 1.
    let held = |gl: &glow::Context| -> usize {
        (1..=32u32)
            .map(|name| unsafe {
                let name = std::num::NonZeroU32::new(name).unwrap();
                usize::from(gl.is_texture(glow::NativeTexture(name)))
                    + usize::from(gl.is_framebuffer(glow::NativeFramebuffer(name)))
                    + usize::from(gl.is_renderbuffer(glow::NativeRenderbuffer(name)))
            })
            .sum()
    };
    unsafe {
        let mut made = Made::new(&gl);
        made.color([4, 4], Texels::RGBA8, Sampling::LINEAR_CLAMPED).unwrap();
        made.depth([4, 4], Texels::DEPTH24, Sampling::NEAREST, glow::DEPTH_ATTACHMENT).unwrap();
        made.renderbuffer([4, 4], glow::DEPTH24_STENCIL8).unwrap();
        assert_eq!(held(&gl), 5);
        drop(made);
        assert_eq!(held(&gl), 0, "a set never kept is gone");
        let mut made = Made::new(&gl);
        let target = made.color([4, 4], Texels::RGBA8, Sampling::LINEAR_CLAMPED).unwrap();
        assert!(made.complete(&[target.framebuffer]));
        made.keep();
        assert_eq!(held(&gl), 2, "a kept set stays");
        target.delete(&gl);
        assert_eq!(held(&gl), 0);
    }
}

/// [`PostScope`](super::super::gl::scope::PostScope) gives back the
/// framebuffer, viewport and scissor a pass found, whatever it left.
#[test]
fn a_post_scope_restores_what_it_found() {
    use super::super::gl::scope::PostScope;
    use super::super::gl::target::{ColorTarget, Sampling, Texels};
    let gl = headless_gl();
    unsafe {
        let found = ColorTarget::new(&gl, [8, 8], Texels::RGBA8, Sampling::NEAREST).unwrap();
        let other = ColorTarget::new(&gl, [8, 8], Texels::RGBA8, Sampling::NEAREST).unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(found.framebuffer));
        gl.viewport(1, 2, 3, 4);
        gl.scissor(2, 3, 4, 5);
        gl.enable(glow::SCISSOR_TEST);
        let scope = PostScope::capture(&gl);
        assert_eq!(scope.size(), [3, 4]);
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(other.framebuffer));
        gl.viewport(0, 0, 8, 8);
        gl.scissor(0, 0, 1, 1);
        gl.disable(glow::SCISSOR_TEST);
        scope.restore(&gl);
        let (mut viewport, mut scissor) = ([0; 4], [0; 4]);
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
        gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut scissor);
        assert_eq!(gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING), Some(found.framebuffer));
        assert_eq!((viewport, scissor), ([1, 2, 3, 4], [2, 3, 4, 5]));
        assert!(gl.is_enabled(glow::SCISSOR_TEST));
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        found.delete(&gl);
        other.delete(&gl);
    }
}

/// A [`ScreenChain`](super::super::gl::chain::ScreenChain) run puts back the
/// scissor and viewport it found, binds the scene, reads its input by texel
/// again, and leaves no texture on the units its passes use.
#[test]
fn a_screen_chain_run_leaves_the_state_it_found() {
    use super::super::gl::chain::ScreenChain;
    use super::super::gl::link::link;
    use super::super::gl::target::{ColorTarget, Sampling, Texels};
    const VERTEX: &str = "#version 410\nvoid main() { gl_Position = vec4(vec2(gl_VertexID & 1, gl_VertexID >> 1) * 4.0 - 1.0, 0.0, 1.0); }\n";
    const FRAGMENT: &str = "#version 410\nuniform sampler2D source;\nuniform sampler2D added;\nout vec4 colour;\nvoid main() { colour = texelFetch(source, ivec2(0), 0) + texelFetch(added, ivec2(0), 0); }\n";
    let gl = headless_gl();
    unsafe {
        let program = link(&gl, &[(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)], &[]).unwrap();
        let chain = ScreenChain { program, nothing: None };
        let vertex_array = gl.create_vertex_array().unwrap();
        let [scene, input, target] =
            [(); 3].map(|_| ColorTarget::new(&gl, [4, 4], Texels::RGBA8, Sampling::NEAREST).unwrap());
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
        gl.viewport(1, 1, 2, 2);
        gl.enable(glow::SCISSOR_TEST);
        let run = chain.begin(&gl, vertex_array, input.texture);
        assert!(!gl.is_enabled(glow::SCISSOR_TEST));
        chain.pass(&gl, &target, input.texture, Some(scene.texture), |_| {});
        chain.end(&gl, run, scene.framebuffer);
        let mut viewport = [0; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
        assert_eq!(viewport, [1, 1, 2, 2]);
        assert!(gl.is_enabled(glow::SCISSOR_TEST));
        assert_eq!(gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING), Some(scene.framebuffer));
        for unit in [glow::TEXTURE0, glow::TEXTURE1] {
            gl.active_texture(unit);
            assert_eq!(gl.get_parameter_texture(glow::TEXTURE_BINDING_2D), None, "unit {unit:#x}");
        }
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(input.texture));
        let filter = gl.get_tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER);
        assert_eq!(filter, glow::NEAREST as i32, "the input is read by texel again");
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        for made in [scene, input, target] {
            made.delete(&gl);
        }
        gl.delete_vertex_array(vertex_array);
        gl.delete_program(program);
    }
}

/// [`Linked`](super::super::gl::program::Linked) finds a uniform the program
/// has, answers none for one it lacks, and asks GL once for each.
#[test]
fn a_linked_program_looks_each_uniform_up_once() {
    use super::super::gl::link::link;
    use super::super::gl::program::Linked;
    const VERTEX: &str = "#version 410\nvoid main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }\n";
    const FRAGMENT: &str = "#version 410\nuniform vec4 tint;\nout vec4 colour;\nvoid main() { colour = tint; }\n";
    let gl = headless_gl();
    unsafe {
        let linked = Linked::new(link(&gl, &[(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)], &[]).unwrap());
        let tint = linked.location(&gl, "tint");
        assert!(tint.is_some());
        assert_eq!(linked.location(&gl, "missing"), None);
        // Deleted, the program has no uniforms left to find: only the cache answers.
        gl.delete_program(linked.program);
        assert_eq!(linked.location(&gl, "tint"), tint, "the first answer is kept");
        assert_eq!(linked.location(&gl, "missing"), None);
    }
}

/// A gen3 tag from a source that names no game draws through Halo 3's
/// hooks on the legacy route, and so draws as a Halo 3 frame does: its
/// sprites once, from inside the gen3 pipeline, and not again after it.
#[test]
fn a_gen3_frame_with_no_game_draws_as_halo_3s_does() {
    let Ok(root) = std::env::var("BLAM_TEST_H3EK") else {
        eprintln!("skipping: BLAM_TEST_H3EK is not set");
        return;
    };
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::Halo3),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let shot = Shot {
        model: "objects\\vehicles\\warthog\\warthog".to_owned(),
        shaders: true,
        lights: true,
        yaw: 0.6,
        pitch: 0.25,
        kind: Some("vehicle".to_owned()),
    };
    let options = ShotOptions { time: 3.0, ..ShotOptions::new(GameId::Halo3) };
    let without = ShotOptions { no_game: true, ..options.clone() };
    let shown = shoot(&source, &[(shot.clone(), options), (shot, without)]);
    assert!(shown[0].pixels.iter().any(|byte| *byte != 0), "the warthog drew");
    assert!(shown[0].pixels == shown[1].pixels, "the frame with no game drew differently");
}
