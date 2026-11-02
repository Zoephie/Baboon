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
//! features `H2_SHOT_DROP` names.

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
    let (extension, group) = if let Ok(kind) = std::env::var("H2_SHOT_OBJECT") { if kind == "scenery" { ("scenery", b"scen") } else if kind == "machine" { ("device_machine", b"mach") } else if kind == "crate" { ("crate", b"bloc") } else if kind == "giant" { ("giant", b"gint") } else if kind == "equipment" { ("equipment", b"eqip") } else if kind == "weapon" { ("weapon", b"weap") } else if kind == "vehicle" { ("vehicle", b"vehi") } else { ("biped", b"bipd") } } else if std::env::var_os("H2_SHOT_BIPED").is_some() {
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
    }
    data.textures = Some(Arc::new(textures));
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
                "contrails" => preview.h2_contrails.clear(),
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
    // `H2_SHOT_HIDE=a,b` leaves out the materials whose shaders end in these.
    if let Ok(hidden) = std::env::var("H2_SHOT_HIDE") {
        visible_batches.retain(|&batch| {
            let material = preview.batches[batch].material_index as usize;
            !hidden.split(',').any(|name| preview.materials[material].shader_path.ends_with(name))
        });
    }
    let time = std::env::var("H2_SHOT_TIME").map_or(0.0, |time| time.parse().unwrap());
    let frame = ModelGpuFrame {
        preview,
        geometry_id: data.geometry_id,
        textures_id: data.textures_id,
        visible_batches,
        camera: camera.gpu_uniforms(),
        render_mode: state.render_mode,
        background: preview_background(&egui::Visuals::dark()),
        show_backfaces: state.show_backfaces,
        show_grid: false,
        textures: data.textures.clone(),
        bones: animation_skinning_rows(&data, &state).map(Arc::new),
        game_shaders: state.h2_game_shaders,
        default_lights: state.h2_default_lights,
        active_camo: std::env::var_os("H2_SHOT_CAMO").is_some(),
        bloom: std::env::var_os("H2_SHOT_BLOOM").is_some(),
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
            object_states(&data, &set, time)
        },
        object_moves: object_functions(&data).any(|functions| functions.move_at_rest()),
        h2_attached_values: super::h2_attached_values(&data, time),
        h3_flares_shown: super::h3_flares_shown(&data.preview, &state),
        h3_lights_shown: super::h3_lights_shown(&data.preview, &state),
        h3_volumes_shown: super::h3_volumes_shown(&data.preview, &state),
        h3_systems_shown: super::h3_systems_shown(&data.preview, &state),
        h3_strips_shown: super::h3_strips_shown(&data.preview, &state),
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
            h2_function_values_for(&data, &set, time)
        },
        ce_points: Vec::new(),
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
        game: Some(if root.contains("odst") { GameId::Halo3Odst } else { GameId::Halo3 }),
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
                    Ok((vertex, pixel)) => {
                        let mut failed = false;
                        for (kind, text) in
                            [(glow::VERTEX_SHADER, &vertex.source), (glow::FRAGMENT_SHADER, &pixel.source)]
                        {
                            unsafe {
                                let shader = gl.create_shader(kind).unwrap();
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
                                gl.delete_shader(shader);
                            }
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
