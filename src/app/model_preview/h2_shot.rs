//! Renders a real model through the real renderer with no window, so a pass
//! can be looked at without opening the app. macOS only, through CGL.
//!
//! `H2_SHOT="<model path>|<out file>|<shaders 0/1>|<lights 0/1>|<yaw>|<pitch>"`,
//! several separated by `;`, with `BLAM_TEST_H2EK` naming the kit's tags.
//! With `H2_SHOT_CE` set, that kit is Halo CE's and the models gbxmodels, or
//! bipeds with `H2_SHOT_BIPED`, whose function inputs `H2_SHOT_INPUTS` can set.

use super::*;
use crate::core::format::TagNameIndex;
use crate::core::game::GameId;
use crate::core::source::{TagEntry, TagEntryLocation, TagSource};
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
    if std::env::var_os("H2_SHOT_CE").is_some() { GameId::HaloCe } else { GameId::Halo2 }
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
    let (extension, group) = if std::env::var_os("H2_SHOT_BIPED").is_some() {
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
    let settings = PreviewLoadSettings::default();
    let mut data = load_model_preview(&tag, &entry, &names, Some(source), &settings).unwrap();
    let textures = materials::resolve_model_textures(source, &data.preview.materials);
    let with_passes = textures.iter().filter(|t| t.h2_layers.iter().any(|l| !l.is_empty())).count();
    eprintln!("{}: {} materials, {with_passes} with passes", shot.model, textures.len());
    data.textures = Some(Arc::new(textures));
    let mut state = ModelPreviewState::default();
    reset_model_preview_selection(&mut state, &data, default_variant_index(&data));
    state.h2_game_shaders = shot.shaders;
    state.h2_default_lights = shot.lights;
    state.yaw = shot.yaw;
    state.pitch = shot.pitch;
    state.perspective = true;

    let (width, height) = (1000i32, 760i32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, height as f32));
    let camera = PreviewCamera::new(&data, &state, rect);
    let preview = Arc::clone(&data.preview);
    let visible_batches = visible_batch_indices(&preview, &state);
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
        bones: None,
        game_shaders: state.h2_game_shaders,
        default_lights: state.h2_default_lights,
        active_camo: std::env::var_os("H2_SHOT_CAMO").is_some(),
        bloom: std::env::var_os("H2_SHOT_BLOOM").is_some(),
        self_shadow: std::env::var_os("H2_SHOT_SHADOW").is_some(),
        light_follows_camera: state.h2_light_follows_camera,
        time,
        h2_prt_clusters: None,
        object_outputs: data
            .ce_object_functions
            .as_ref()
            .map_or([0.0; 4], |functions| {
                let mut inputs = functions.inputs_for(None, data.geometry_id);
                if let Ok(set) = std::env::var("H2_SHOT_INPUTS") {
                    let values = set.split(',').map(|value| value.parse().unwrap());
                    inputs.iter_mut().zip(values).for_each(|(input, value)| *input = value);
                }
                functions.outputs(&inputs, time)
            }),
        object_moves: data
            .ce_object_functions
            .as_ref()
            .is_some_and(|functions| functions.move_at_rest()),
        change_colors: h2_change_colors_for(
            &data.h2_change_colors,
            state
                .selected_variant
                .and_then(|index| data.variants.get(index))
                .map_or("", |variant| variant.name.as_str()),
        ),
    };
    let mut frame = frame;
    frame.h2_prt_clusters = frame.h2_object_prt_clusters();
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
