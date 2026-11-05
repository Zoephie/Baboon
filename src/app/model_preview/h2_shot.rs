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

struct Shot {
    model: String,
    out: String,
    shaders: bool,
    lights: bool,
    yaw: f32,
    pitch: f32,
}

fn render(gl: &Arc<glow::Context>, renderer: &mut ModelGlRenderer, source: &TagSource, shot: &Shot) {
    let names = match source {
        TagSource::LooseFolder { definitions_root, .. } => {
            TagNameIndex::load_game(definitions_root, shot_game()).unwrap()
        }
        _ => unreachable!(),
    };
    let TagSource::LooseFolder { root, .. } = source else { unreachable!() };
    // A sky has a render_model and nothing above it.
    let file = root.join(format!("{}.model", shot.model.replace('\\', "/")));
    let (extension, group) = if let Ok(kind) = std::env::var("H2_SHOT_OBJECT") { if kind == "scenery" { ("scenery", b"scen") } else if kind == "machine" { ("device_machine", b"mach") } else if kind == "crate" { ("crate", b"bloc") } else if kind == "giant" { ("giant", b"gint") } else if kind == "equipment" { ("equipment", b"eqip") } else if kind == "weapon" { ("weapon", b"weap") } else if kind == "vehicle" { ("vehicle", b"vehi") } else if kind == "effect_scenery" { ("effect_scenery", b"efsc") } else { ("biped", b"bipd") } } else if std::env::var_os("H2_SHOT_BIPED").is_some() {
        ("biped", b"bipd")
    } else if shot_game() == GameId::HaloCe {
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
        weapon: std::env::var("H2_SHOT_WEAPON").ok(),
        secondary_weapon: std::env::var("H2_SHOT_SECONDARY").ok(),
        h2_equippable: None,
        ..Default::default()
    };
    let mut data = load_model_preview(&tag, &entry, &names, Some(source), &settings).unwrap();
    let textures = materials::resolve_model_textures(source, &data.preview.materials);
    let with_passes = textures.iter().filter(|t| t.h2_layers.iter().any(|l| !l.is_empty())).count();
    eprintln!("{}: {} materials, {with_passes} with passes", shot.model, textures.len());
    for (material, texture) in data.preview.materials.iter().zip(textures.iter()) {
        eprintln!("NOTE MAT {} h3 {} {:?}", material.shader_path, texture.h3.is_some(), texture.error);
        // `H4_TEXTURES=1`: each pass's samplers, and whether one is bound.
        if std::env::var_os("H4_TEXTURES").is_some()
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
    eprintln!("NOTE {} cheap particle emitters", data.preview.reach_cheap_emitters.len());
    eprintln!(
        "NOTE H4 {} lights, {} cheap lights, {} light passes, {} flares, {} particle systems ({} on materials), {} light volumes ({} on materials), {} tracers",
        data.preview.h4_lights.len(),
        data.preview.h4_cheap_lights.len(),
        data.preview.h4_light_passes.len(),
        data.preview.h4_flares.len(),
        data.preview.h3_particles.len(),
        data.preview.h3_particles.iter().filter(|system| system.look.h4.is_some()).count(),
        data.preview.h3_light_volumes.len(),
        data.preview.h3_light_volumes.iter().filter(|volume| volume.h4_material().is_some()).count(),
        data.preview.h4_tracers.len()
    );
    // `H2_SHOT_DROP=a,b` leaves out whole features, to see one on its own.
    if let Ok(dropped) = std::env::var("H2_SHOT_DROP") {
        let preview = Arc::make_mut(&mut data.preview);
        for feature in dropped.split(',') {
            match feature {
                "particles" => preview.h3_particles.clear(),
                "flares" => preview.h2_flares.clear(),
                "lights" => preview.h2_point_lights.clear(),
                "gels" => preview.h2_gel_lights.clear(),
                "volumes" => preview.h2_light_volumes.clear(),
                "h3_volumes" => preview.h3_light_volumes.clear(),
                "h4_tracers" => preview.h4_tracers.clear(),
                "cheap_particles" => preview.reach_cheap_emitters.clear(),
                "contrails" => preview.h2_contrails.clear(),
                "prt" => preview.h3_prt = None,
                "shield" => preview.reach_shield = None,
                "antennas" => preview.h2_antennas.clear(),
                // The sheet's own batches go with it; its vertices stay, unused.
                "cloth" => {
                    preview.h3_cloth.clear();
                    preview.batches.retain(|batch| batch.permutation_name != "cloth");
                }
                "reach_lights" => {
                    preview.reach_lights.clear();
                    preview.reach_cheap_lights.clear();
                }
                "h4_lights" => {
                    preview.h4_lights.clear();
                    preview.h4_cheap_lights.clear();
                }
                "h4_flares" => preview.h4_flares.clear(),
                other => panic!("H2_SHOT_DROP: no feature {other}"),
            }
        }
    }
    let mut state = ModelPreviewState::default();
    state.h2_light_follows_camera = std::env::var_os("H2_SHOT_FIXED_LIGHT").is_none();
    let variant = std::env::var("H2_SHOT_VARIANT").ok();
    let variant = variant.and_then(|name| data.variants.iter().position(|variant| variant.name == name));
    reset_model_preview_selection(&mut state, &data, variant.or(default_variant_index(&data)));
    state.h2_game_shaders = shot.shaders;
    state.h2_default_lights = shot.lights;
    state.yaw = shot.yaw;
    state.pitch = shot.pitch;
    state.perspective = true;
    // `H2_SHOT_FOCUS=x,y,z` orbits that point, at `H2_SHOT_SCALE` times the fit.
    let (center, radius) = preview_center_radius(&data.preview);
    eprintln!("NOTE center {center:?} radius {radius}");
    if let Ok(focus) = std::env::var("H2_SHOT_FOCUS") {
        let point: Vec<f32> = focus.split(',').map(|axis| axis.parse().unwrap()).collect();
        let (center, _) = preview_center_radius(&data.preview);
        state.focus = std::array::from_fn(|axis| point[axis] - center[axis]);
    }
    if let Ok(scale) = std::env::var("H2_SHOT_SCALE") {
        state.scale = scale.parse().unwrap();
    }
    state.weapon = settings.weapon.clone();
    state.secondary_weapon = settings.secondary_weapon.clone();
    if let Ok(clip) = std::env::var("H2_SHOT_CLIP") {
        animation::pose_for_shot(source, &entry, &mut data, &mut state, &clip);
    }

    let (width, height) = (1000i32, 760i32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, height as f32));
    let camera = PreviewCamera::new(&data, &state, rect);
    let preview = Arc::clone(&data.preview);
    let mut visible_batches = visible_batch_indices(&preview, &state);
    let shield_batches = visible_batches.clone();
    without_shield_parts(&mut visible_batches, &preview, data.textures.as_deref().map(Vec::as_slice));
    // `H2_SHOT_HIDE=a,b` leaves out the materials whose shaders end in these.
    if let Ok(hidden) = std::env::var("H2_SHOT_HIDE") {
        visible_batches.retain(|&batch| {
            let material = preview.batches[batch].material_index as usize;
            !hidden.split(',').any(|name| preview.materials[material].shader_path.ends_with(name))
        });
    }
    let time = std::env::var("H2_SHOT_TIME").map_or(0.0, |time| time.parse().unwrap());
    // `H2_SHOT_MODE=solid|wire|solid_wire|textured|textured_wire`.
    let render_mode = match std::env::var("H2_SHOT_MODE").as_deref() {
        Ok("solid") => ModelRenderMode::Solid,
        Ok("wire") => ModelRenderMode::Wireframe,
        Ok("solid_wire") => ModelRenderMode::SolidWireframe,
        Ok("textured_wire") => ModelRenderMode::TexturedWireframe,
        _ => state.render_mode,
    };
    let frame = ModelGpuFrame {
        preview,
        geometry_id: data.geometry_id,
        mesh_id: data.mesh_id,
        textures_id: data.textures_id,
        visible_batches,
        shield_batches,
        camera: camera.gpu_uniforms(),
        render_mode,
        // `H2_SHOT_LIGHT` takes the light theme's backdrop.
        background: preview_background(&if std::env::var_os("H2_SHOT_LIGHT").is_some() {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        }),
        show_backfaces: state.show_backfaces,
        show_grid: std::env::var_os("H2_SHOT_GRID").is_some(),
        textures: render_mode.uses_textures().then(|| data.textures.clone()).flatten(),
        bones: animation_skinning_rows(&data, &state).map(Arc::new),
        game_shaders: state.h2_game_shaders,
        default_lights: state.h2_default_lights,
        active_camo: std::env::var_os("H2_SHOT_CAMO").is_some(),
        bloom: std::env::var_os("H2_SHOT_BLOOM").is_some() && !data.halo_ce,
        self_shadow: std::env::var_os("H2_SHOT_SHADOW").is_some(),
        prt: std::env::var_os("H2_SHOT_NO_PRT").is_none(),
        light_follows_camera: state.h2_light_follows_camera,
        time,
        h2_prt_clusters: None,
        object_states: {
            let mut set = state.ce_function_inputs.clone();
            if let Ok(values) = std::env::var("H2_SHOT_INPUTS") {
                let values: Vec<f32> = values.split(',').map(|value| value.parse().unwrap()).collect();
                set.insert((data.geometry_id, 0), std::array::from_fn(|index| values[index]));
            }
            object_states(&data, &set, &state.ce_shots, time)
        },
        object_moves: object_functions(&data).any(|functions| functions.move_at_rest()),
        h2_attached_values: super::h2_attached_values(&data, time),
        h2_attached_colors: super::h2_attached_colors(&data),
        h3_flares_shown: super::h3_flares_shown(&data.preview, &state),
        h3_lights_shown: super::h3_lights_shown(&data.preview, &state),
        reach_lights_shown: super::reach_lights_shown(&data.preview, &state),
        h3_volumes_shown: super::h3_volumes_shown(&data.preview, &state),
        h4_tracers_shown: super::h4_tracers_shown(&data.preview, &state),
        h3_systems_shown: super::h3_systems_shown(&data.preview, &state),
        h3_strips_shown: super::h3_strips_shown(&data.preview, &state),
        reach_cheap_shown: super::reach_cheap_shown(&data.preview, &state),
        h2_flares_shown: super::h2_flares_shown(&data.preview, &state),
        h2_shown: super::H2Shown::of(&data.preview, &state),
        h2_values: {
            let mut set = state.h2_function_inputs.clone();
            if let Ok(pairs) = std::env::var("H2_SHOT_FUNCTIONS") {
                for pair in pairs.split(',') {
                    let (name, value) = pair.split_once('=').unwrap();
                    set.insert((data.geometry_id, name.to_owned()), value.parse().unwrap());
                }
            }
            let mut values = h2_function_values_for(&data, &set, time);
            // `H2_SHOT_FORCE=name=value,..` sets what a function exports, which
            // its inputs cannot reach: a scripted value, say.
            if let (Ok(pairs), Some(values)) = (std::env::var("H2_SHOT_FORCE"), values.as_mut()) {
                for pair in pairs.split(',') {
                    let (name, value) = pair.split_once('=').unwrap();
                    values.insert(name.to_owned(), value.parse().unwrap());
                }
            }
            values
        },
        ce_points: Vec::new(),
        ce_plays: super::ce_event_plays(&data, &state.ce_event_counts, state.ce_event_scales),
        change_colors: h2_change_colors_for(
            &data.h2_change_colors,
            state
                .selected_variant
                .and_then(|index| data.variants.get(index))
                .map_or("", |variant| variant.name.as_str()),
        ),
    };
    let mut frame = frame;
    frame.object_moves =
        super::object_moves(&data, &state.h2_function_inputs, time, frame.h2_values.as_ref());
    eprintln!("NOTE moves {}", frame.h2_moves());
    let mut frame = frame;
    frame.h2_prt_clusters = frame.h2_object_prt_clusters();
    frame.ce_points = frame.ce_point_lights();
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
        if std::env::var_os("H2_SHOT_VISION").is_some() {
            let end = std::mem::replace(&mut frame.time, 0.0);
            renderer.paint(gl, &frame);
            frame.time = end;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }
        let play = std::env::var("H2_SHOT_PLAY").ok().map(|play| (play, false));
        if let Some((play, fire)) = play.or_else(|| std::env::var("H2_SHOT_FIRE").ok().map(|fire| (fire, true))) {
            let (text, before) = play.split_once('@').expect("H2_SHOT_PLAY or H2_SHOT_FIRE is <what>@<seconds>");
            let end = frame.time;
            frame.time = end - before.parse::<f32>().unwrap();
            renderer.paint(gl, &frame);
            if fire {
                let weapon = data.preview.ce_firing.last().expect("a weapon to fire");
                let fired = state.ce_shots.entry((data.geometry_id, weapon.object)).or_default();
                let effect = fired.fire(weapon, text.parse().unwrap(), state.ce_event_scales[0], frame.time);
                eprintln!("NOTE fired, playing {:?}", effect.and_then(|at| data.preview.ce_effects[at].event.as_ref()));
                if let Some(effect) = effect {
                    frame.ce_plays.counts[effect] += 1;
                }
            }
            for (index, effect) in data.preview.ce_effects.iter().enumerate() {
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
        std::fs::write(&shot.out, &pixels).unwrap();
    }
}

#[test]
fn h2_shot() {
    let (Ok(shots), Ok(root)) = (std::env::var("H2_SHOT"), std::env::var("BLAM_TEST_H2EK")) else {
        return;
    };
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(shot_game()),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
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
            out: f[1].to_owned(),
            shaders: f[2] == "1",
            lights: f[3] == "1",
            yaw: f[4].parse().unwrap(),
            pitch: f[5].parse().unwrap(),
        };
        render(&gl, &mut renderer, &source, &shot);
    }
}

/// Reach's `shadow_apply` links, and reads every input it is given.
#[test]
fn reach_shadow_apply_links() {
    let gl = headless_gl();
    let program = unsafe { super::h3_draw::screen_program(&gl, super::reach_shadow_pass::APPLY_FRAGMENT) }
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
    let shield = super::reach_shield::read_reach_shield(&source, &model).expect("the Spartan's shield");
    let gl = headless_gl();
    let passes: Vec<_> = shield.passes().collect();
    assert_eq!(passes.len(), 2, "both explicit shaders");
    for pass in passes {
        assert!(pass.pixel.contains(super::reach_shield::DEPTH_DECODE), "the depth read is the preview's");
        assert!(!pass.pixel.contains("depth_constants.z - "), "Reach's own depth read is gone");
        assert!(!pass.vertex.contains("GlobalTexture_shield_impact_noise_texture"), "the vertex stage reads oscillation");
        unsafe {
            let program = gl.create_program().unwrap();
            for (kind, text) in [(glow::VERTEX_SHADER, &pass.vertex), (glow::FRAGMENT_SHADER, &pass.pixel)] {
                let shader = gl.create_shader(kind).unwrap();
                gl.shader_source(shader, text);
                gl.compile_shader(shader);
                assert!(gl.get_shader_compile_status(shader), "{}", gl.get_shader_info_log(shader));
                gl.attach_shader(program, shader);
            }
            gl.link_program(program);
            assert!(gl.get_program_link_status(program), "{}", gl.get_program_info_log(program));
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
    let mut sources = super::h3_shaders::H3Sources::new(&source);
    for file in ["light_apply", "light_apply_frustum", "light_apply_specular", "light_apply_frustum_specular"] {
        let pass = sources.reach_light_pass(file).unwrap_or_else(|error| panic!("{file}: {error}"));
        for (name, _) in &pass.uniforms {
            eprintln!("NOTE {file} UNIFORM {name}");
        }
        for (name, _) in &pass.textures {
            eprintln!("NOTE {file} TEXTURE {name}");
        }
        unsafe {
            let program = gl.create_program().unwrap();
            for (kind, text) in [(glow::VERTEX_SHADER, &pass.vertex), (glow::FRAGMENT_SHADER, &pass.pixel)] {
                let shader = gl.create_shader(kind).unwrap();
                gl.shader_source(shader, text);
                gl.compile_shader(shader);
                assert!(gl.get_shader_compile_status(shader), "{file}: {}", gl.get_shader_info_log(shader));
                gl.attach_shader(program, shader);
            }
            gl.link_program(program);
            assert!(gl.get_program_link_status(program), "{file}: {}", gl.get_program_info_log(program));
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
        let program = super::h4_lights::h4_light_program(&source, physical).expect("the light shader");
        for technique in 0..16 {
            let pass = super::h4_lights::h4_light_pass(&program, technique).unwrap_or_else(|error| panic!("{physical} {technique}: {error}"));
            unsafe {
                let linked = gl.create_program().unwrap();
                for (kind, text) in [(glow::VERTEX_SHADER, &pass.vertex), (glow::FRAGMENT_SHADER, &pass.pixel)] {
                    let shader = gl.create_shader(kind).unwrap();
                    gl.shader_source(shader, text);
                    gl.compile_shader(shader);
                    assert!(gl.get_shader_compile_status(shader), "{physical} {technique}: {}", gl.get_shader_info_log(shader));
                    gl.attach_shader(linked, shader);
                }
                gl.link_program(linked);
                assert!(gl.get_program_link_status(linked), "{physical} {technique}: {}", gl.get_program_info_log(linked));
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
    let pass = super::h4_flares::h4_flare_pass(&source).expect("the flare pass");
    for (name, _) in &pass.uniforms {
        eprintln!("NOTE flare UNIFORM {name}");
    }
    assert!(pass.vertex.contains("in vec2 a_POSITION;") && pass.vertex.contains("a_TEXCOORD0"), "{}", pass.vertex);
    unsafe {
        let linked = gl.create_program().unwrap();
        for (kind, text) in [(glow::VERTEX_SHADER, &pass.vertex), (glow::FRAGMENT_SHADER, &pass.pixel)] {
            let shader = gl.create_shader(kind).unwrap();
            gl.shader_source(shader, text);
            gl.compile_shader(shader);
            assert!(gl.get_shader_compile_status(shader), "{}", gl.get_shader_info_log(shader));
            gl.attach_shader(linked, shader);
        }
        gl.link_program(linked);
        assert!(gl.get_program_link_status(linked), "{}", gl.get_program_info_log(linked));
    }
}

/// Every kit effect material shader's vertex passes link with its `_default`
/// pixel shader; `BLAM_TEST_H4EK` to the tags folder.
#[test]
fn h4_effect_passes_link() {
    use super::h4_shaders::{H4_DEFINES, H4ShaderSource, translate_pass};
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
            let both = super::h3_shaders::pass_of(vertex, pixel.clone());
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
    use super::h3_shaders::{H3Sources, VERTEX_SKINNED, read_h3_definition};
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
    use super::h3_shaders::{H3Sources, VERTEX_RIGID, VERTEX_SKINNED, read_h3_definition};
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
    use super::h4_shaders::{H4_DEFINES, H4ShaderSource, translate_pass};
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
    use super::h4_shaders::{H4_DEFINES, H4ShaderSource, translate_pass};
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

/// Every Halo 4 `.model` through the shot harness, each caught on its own.
/// Set `H4_KIT=<limit>`, `H2_SHOT_H4=1` and `BLAM_TEST_H2EK` to the tags
/// folder; translation and link failures print as `h3 program failed`.
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
    let mut models = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(&root)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "model") {
                let relative = path.strip_prefix(&root).unwrap().with_extension("");
                models.push(relative.to_string_lossy().replace('/', "\\"));
            }
        }
    }
    models.sort();
    let gl = Arc::new(headless_gl());
    let mut renderer = ModelGlRenderer::new(&gl).unwrap();
    let out = std::env::temp_dir().join("baboon_h4_kit_sweep.raw");
    let (mut good, mut failed) = (0, Vec::new());
    for model in models.iter().take(limit) {
        let shot = Shot { model: model.clone(), out: out.to_string_lossy().into_owned(), shaders: true, lights: true, yaw: -1.57, pitch: 0.3 };
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(&gl, &mut renderer, &source, &shot)));
        match caught {
            Ok(()) => good += 1,
            Err(panic) => {
                let detail = crate::app::shell::worker::panic_detail(&*panic);
                eprintln!("NOTE FAIL {model}: {}", detail.chars().take(200).collect::<String>());
                failed.push(model.clone());
                // A panic mid-frame can leave state bound; start clean.
                renderer = ModelGlRenderer::new(&gl).unwrap();
            }
        }
    }
    let _ = std::fs::remove_file(&out);
    eprintln!("NOTE KIT {} models: good {good}, failed {}", models.len().min(limit), failed.len());
}

/// The Chief biped, with the AR, posed at `clip`, from the kit at `root`.
fn h4_chief_with_rifle(root: &str, clip: &str) -> (ModelPreviewData, ModelPreviewState) {
    let source = TagSource::LooseFolder {
        root: root.clone().into(),
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
    let posed = |marker: &super::reach_ik::ReachIkMarker| {
        let skin = &rows[marker.node * 3..marker.node * 3 + 3];
        let at = [marker.bind.m[0][3], marker.bind.m[1][3], marker.bind.m[2][3]];
        std::array::from_fn::<f32, 3, _>(|row| skin[row][0] * at[0] + skin[row][1] * at[1] + skin[row][2] * at[2] + skin[row][3])
    };
    let markers = &data.preview.reach_ik_markers;
    let find = |holder, name: &str| markers.iter().find(|marker| marker.holder == holder && marker.name.eq_ignore_ascii_case(name));
    let mut solved = 0;
    for point in points.iter() {
        let (Some(source), Some(destination)) =
            (find(super::reach_ik::ReachIkHolder::Unit, &point.source), find(point.holder, &point.destination))
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
    let posed = |marker: &super::reach_ik::ReachIkMarker| {
        let skin = &rows[marker.node * 3..marker.node * 3 + 3];
        let at = [marker.bind.m[0][3], marker.bind.m[1][3], marker.bind.m[2][3]];
        std::array::from_fn::<f32, 3, _>(|row| skin[row][0] * at[0] + skin[row][1] * at[1] + skin[row][2] * at[2] + skin[row][3])
    };
    let point = &points[0];
    let (from, to) = (posed(find(super::reach_ik::ReachIkHolder::Unit, &point.source).unwrap()), posed(find(point.holder, &point.destination).unwrap()));
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
    assert!(pose.reach_ik.iter().any(|track| track.holder == super::reach_ik::ReachIkHolder::Weapon(1)));
    let rows = animation::animation_skinning_rows(&data, &state).expect("a posed frame");
    assert!(rows.iter().flatten().all(|lane| lane.is_finite()));
}
