//! A headless CGL context, and the logging GL entry points G-calls records
//! through.

use eframe::glow;
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

pub(in crate::app::model_preview) fn headless_gl() -> glow::Context {
    headless_gl_loading(false)
}

/// A headless context whose GL state calls are logged, for [`take_gl_log`].
pub(in crate::app::model_preview) fn headless_gl_recording() -> glow::Context {
    headless_gl_loading(true)
}

fn headless_gl_loading(recording: bool) -> glow::Context {
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
            if recording && let Some(shim) = gl_log::checked_shim(name) {
                return shim;
            }
            if recording && let Some(shim) = gl_log::shim(name) {
                return shim;
            }
            let name = CString::new(name).unwrap();
            dlsym(-2isize as *mut c_void, name.as_ptr()) as *const c_void
        })
    }
}

/// The draws a [`headless_gl_recording`] context has made on this thread,
/// since the last take, that sampled a texture attached to the framebuffer
/// they drew into: a feedback loop, which GL leaves undefined. One line each.
pub(in crate::app::model_preview) fn take_feedback_loops() -> Vec<String> {
    gl_log::LOOPS.with(|loops| std::mem::take(&mut *loops.borrow_mut()))
}

/// The GL state calls a [`headless_gl_recording`] context has made on this
/// thread since the last take, one line each.
pub(in crate::app::model_preview) fn take_gl_log() -> Vec<String> {
    gl_log::LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

/// Logging stand-ins for the GL entry points that set the state passes hand
/// over: each logs its name and arguments, then calls the real one.
mod gl_log {
    use std::cell::RefCell;
    use std::ffi::{CString, c_void};
    use std::sync::OnceLock;

    thread_local! {
        pub(super) static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        pub(super) static LOOPS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    /// Before draw `name`: each sampler of the current program whose unit
    /// holds a texture attached to the draw framebuffer. It asks GL through
    /// the real entry points, so nothing it does reaches the log.
    fn check_feedback(name: &str) {
        static GET_INTEGER: OnceLock<usize> = OnceLock::new();
        static ATTACHMENT: OnceLock<usize> = OnceLock::new();
        static PROGRAM_IV: OnceLock<usize> = OnceLock::new();
        static ACTIVE_UNIFORM: OnceLock<usize> = OnceLock::new();
        static LOCATION: OnceLock<usize> = OnceLock::new();
        static UNIFORM_IV: OnceLock<usize> = OnceLock::new();
        static ACTIVE_TEXTURE: OnceLock<usize> = OnceLock::new();
        unsafe {
            let get_integer: extern "system" fn(u32, *mut i32) = std::mem::transmute(real("glGetIntegerv", &GET_INTEGER));
            let attachment: extern "system" fn(u32, u32, u32, *mut i32) =
                std::mem::transmute(real("glGetFramebufferAttachmentParameteriv", &ATTACHMENT));
            let program_iv: extern "system" fn(u32, u32, *mut i32) = std::mem::transmute(real("glGetProgramiv", &PROGRAM_IV));
            let active_uniform: extern "system" fn(u32, u32, i32, *mut i32, *mut i32, *mut u32, *mut u8) =
                std::mem::transmute(real("glGetActiveUniform", &ACTIVE_UNIFORM));
            let location: extern "system" fn(u32, *const u8) -> i32 = std::mem::transmute(real("glGetUniformLocation", &LOCATION));
            let uniform_iv: extern "system" fn(u32, i32, *mut i32) = std::mem::transmute(real("glGetUniformiv", &UNIFORM_IV));
            let active_texture: extern "system" fn(u32) = std::mem::transmute(real("glActiveTexture", &ACTIVE_TEXTURE));
            let integer = |parameter: u32| {
                let mut value = 0;
                get_integer(parameter, &mut value);
                value
            };
            let framebuffer = integer(0x8CA6);
            let program = integer(0x8B8D) as u32;
            if framebuffer == 0 || program == 0 {
                return;
            }
            // What the draw framebuffer has attached as textures.
            let attached: Vec<(u32, i32)> = (0..8)
                .map(|at| 0x8CE0 + at)
                .chain([0x8D00, 0x8D20])
                .filter_map(|point| {
                    let mut kind = 0;
                    attachment(0x8CA9, point, 0x8CD0, &mut kind);
                    if kind != 0x1702 {
                        return None;
                    }
                    let mut texture = 0;
                    attachment(0x8CA9, point, 0x8CD1, &mut texture);
                    Some((point, texture))
                })
                .collect();
            if attached.is_empty() {
                return;
            }
            let mut count = 0;
            program_iv(program, 0x8B86, &mut count);
            let before = integer(0x84E0) as u32;
            for index in 0..count.max(0) as u32 {
                let (mut length, mut size, mut kind) = (0, 0, 0u32);
                let mut text = [0u8; 128];
                active_uniform(program, index, text.len() as i32, &mut length, &mut size, &mut kind, text.as_mut_ptr());
                let binding = match kind {
                    0x8B5E | 0x8B62 | 0x8DCA => 0x8069,
                    0x8B60 => 0x8514,
                    0x8B5F => 0x806A,
                    _ => continue,
                };
                let at = location(program, text.as_ptr());
                if at < 0 {
                    continue;
                }
                let mut unit = 0;
                uniform_iv(program, at, &mut unit);
                active_texture(0x84C0 + unit as u32);
                let bound = integer(binding);
                let uniform = String::from_utf8_lossy(&text[..length.max(0) as usize]).into_owned();
                if let Some((point, _)) = attached.iter().find(|(_, texture)| *texture == bound && bound != 0) {
                    let line = format!("{name}: {uniform} (unit {unit}) samples texture {bound}, attached at {point:#x} of framebuffer {framebuffer}");
                    LOOPS.with(|loops| loops.borrow_mut().push(line));
                }
            }
            active_texture(before);
        }
    }

    extern "system" fn draw_arrays_instanced(mode: u32, first: i32, count: i32, instances: i32) {
        static REAL: OnceLock<usize> = OnceLock::new();
        check_feedback("glDrawArraysInstanced");
        let real: extern "system" fn(u32, i32, i32, i32) = unsafe { std::mem::transmute(real("glDrawArraysInstanced", &REAL)) };
        real(mode, first, count, instances)
    }

    /// The stand-ins that check a draw without logging it.
    pub(super) fn checked_shim(name: &str) -> Option<*const c_void> {
        match name {
            "glDrawArraysInstanced" => Some(draw_arrays_instanced as *const c_void),
            _ => None,
        }
    }

    fn record(line: String) {
        LOG.with(|log| log.borrow_mut().push(line));
    }

    /// The real entry point `name`, looked up once.
    fn real(name: &str, cell: &OnceLock<usize>) -> usize {
        *cell.get_or_init(|| {
            let name = CString::new(name).unwrap();
            unsafe { super::dlsym(-2isize as *mut c_void, name.as_ptr()) as usize }
        })
    }

    macro_rules! logged {
        ($($name:literal => fn $shim:ident($($arg:ident: $ty:ty),*);)*) => {
            $(
                extern "system" fn $shim($($arg: $ty),*) {
                    static REAL: OnceLock<usize> = OnceLock::new();
                    let mut line = String::from($name);
                    $(line.push_str(&format!(" {:?}", $arg));)*
                    record(line);
                    if $name.starts_with("glDraw") {
                        check_feedback($name);
                    }
                    let real: extern "system" fn($($ty),*) = unsafe { std::mem::transmute(real($name, &REAL)) };
                    real($($arg),*)
                }
            )*

            /// The logging stand-in for `name`, if it has one.
            pub(super) fn shim(name: &str) -> Option<*const c_void> {
                match name {
                    $($name => Some($shim as *const c_void),)*
                    _ => None,
                }
            }
        };
    }

    logged! {
        "glEnable" => fn enable(cap: u32);
        "glDisable" => fn disable(cap: u32);
        "glBlendFunc" => fn blend_func(source: u32, destination: u32);
        "glBlendFuncSeparate" => fn blend_func_separate(source: u32, destination: u32, source_alpha: u32, destination_alpha: u32);
        "glBlendEquation" => fn blend_equation(mode: u32);
        "glBlendEquationSeparate" => fn blend_equation_separate(color: u32, alpha: u32);
        "glColorMask" => fn color_mask(red: u8, green: u8, blue: u8, alpha: u8);
        "glDepthMask" => fn depth_mask(flag: u8);
        "glDepthFunc" => fn depth_func(function: u32);
        "glCullFace" => fn cull_face(mode: u32);
        "glFrontFace" => fn front_face(mode: u32);
        "glPolygonOffset" => fn polygon_offset(factor: f32, units: f32);
        "glUseProgram" => fn use_program(program: u32);
        "glBindVertexArray" => fn bind_vertex_array(array: u32);
        "glBindBuffer" => fn bind_buffer(target: u32, buffer: u32);
        "glActiveTexture" => fn active_texture(unit: u32);
        "glBindTexture" => fn bind_texture(target: u32, texture: u32);
        "glUniform1i" => fn uniform_1_i32(location: i32, value: i32);
        "glUniform1f" => fn uniform_1_f32(location: i32, value: f32);
        "glUniform2f" => fn uniform_2_f32(location: i32, x: f32, y: f32);
        "glDrawArrays" => fn draw_arrays(mode: u32, first: i32, count: i32);
        "glDrawElements" => fn draw_elements(mode: u32, count: i32, kind: u32, indices: *const c_void);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glow::HasContext as _;

    /// A draw that samples the texture its framebuffer draws into is a
    /// feedback loop, and the recording context names it; sampling another
    /// texture is not.
    #[test]
    fn a_draw_sampling_its_own_target_is_a_feedback_loop() {
        let gl = headless_gl_recording();
        unsafe {
            let texture = |gl: &glow::Context| {
                let texture = gl.create_texture().unwrap();
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, 4, 4, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
                texture
            };
            let target = texture(&gl);
            let other = texture(&gl);
            let framebuffer = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(target), 0);
            let stages = [
                (glow::VERTEX_SHADER, "#version 330 core\nvoid main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }"),
                (glow::FRAGMENT_SHADER, "#version 330 core\nuniform sampler2D u_map;\nout vec4 color;\nvoid main() { color = texture(u_map, vec2(0.5)); }"),
            ];
            let program = crate::app::model_preview::gl::link::link(&gl, &stages, &[]).unwrap();
            gl.use_program(Some(program));
            gl.uniform_1_i32(gl.get_uniform_location(program, "u_map").as_ref(), 2);
            let vertex_array = gl.create_vertex_array().unwrap();
            gl.bind_vertex_array(Some(vertex_array));
            take_feedback_loops();
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, Some(other));
            gl.draw_arrays(glow::POINTS, 0, 1);
            assert!(take_feedback_loops().is_empty(), "another texture is no loop");
            gl.bind_texture(glow::TEXTURE_2D, Some(target));
            gl.draw_arrays(glow::POINTS, 0, 1);
            let loops = take_feedback_loops();
            assert_eq!(loops.len(), 1, "{loops:?}");
            assert!(loops[0].contains("u_map (unit 2)"), "{loops:?}");
            take_gl_log();
        }
    }
}
