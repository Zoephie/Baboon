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
            if recording && let Some(shim) = gl_log::shim(name) {
                return shim;
            }
            let name = CString::new(name).unwrap();
            dlsym(-2isize as *mut c_void, name.as_ptr()) as *const c_void
        })
    }
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
