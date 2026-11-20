//! A headless WGL context on Windows: a hidden window's OpenGL context, which
//! the real renderer paints into an offscreen framebuffer. Nothing is shown,
//! so it works with the screen off. It records no GL calls; the gates that
//! read them run on macOS.

use eframe::glow;
use std::ffi::{CString, c_void};

#[repr(C)]
struct PixelFormatDescriptor {
    size: u16,
    version: u16,
    flags: u32,
    pixel_type: u8,
    color_bits: u8,
    color: [u8; 6],
    alpha_bits: u8,
    alpha_shift: u8,
    accum: [u8; 5],
    depth_bits: u8,
    stencil_bits: u8,
    aux_buffers: u8,
    layer_type: u8,
    reserved: u8,
    masks: [u32; 3],
}

#[link(name = "user32")]
unsafe extern "system" {
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: *mut c_void,
        menu: *mut c_void,
        instance: *mut c_void,
        param: *mut c_void,
    ) -> *mut c_void;
    fn GetDC(window: *mut c_void) -> *mut c_void;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn ChoosePixelFormat(dc: *mut c_void, format: *const PixelFormatDescriptor) -> i32;
    fn SetPixelFormat(dc: *mut c_void, format: i32, descriptor: *const PixelFormatDescriptor)
    -> i32;
}

#[link(name = "opengl32")]
unsafe extern "system" {
    fn wglCreateContext(dc: *mut c_void) -> *mut c_void;
    fn wglMakeCurrent(dc: *mut c_void, context: *mut c_void) -> i32;
    fn wglGetProcAddress(name: *const i8) -> *const c_void;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryA(name: *const i8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const i8) -> *const c_void;
}

pub(in crate::app) fn headless_gl() -> glow::Context {
    const PFD_DRAW_TO_WINDOW: u32 = 0x04;
    const PFD_SUPPORT_OPENGL: u32 = 0x20;
    const PFD_DOUBLEBUFFER: u32 = 0x01;
    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    unsafe {
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            16,
            16,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        assert!(!window.is_null(), "no hidden window");
        let dc = GetDC(window);
        let descriptor = PixelFormatDescriptor {
            size: size_of::<PixelFormatDescriptor>() as u16,
            version: 1,
            flags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
            pixel_type: 0,
            color_bits: 32,
            color: [0; 6],
            alpha_bits: 8,
            alpha_shift: 0,
            accum: [0; 5],
            depth_bits: 24,
            stencil_bits: 8,
            aux_buffers: 0,
            layer_type: 0,
            reserved: 0,
            masks: [0; 3],
        };
        let format = ChoosePixelFormat(dc, &descriptor);
        assert!(format != 0 && SetPixelFormat(dc, format, &descriptor) != 0);
        let context = wglCreateContext(dc);
        assert!(!context.is_null() && wglMakeCurrent(dc, context) != 0);
        let opengl32 = LoadLibraryA(c"opengl32.dll".as_ptr());
        glow::Context::from_loader_function(|name| {
            let name = CString::new(name).unwrap();
            let found = wglGetProcAddress(name.as_ptr());
            // GL 1.1's entry points come from opengl32 itself.
            if matches!(found as isize, -1..=3) {
                GetProcAddress(opengl32, name.as_ptr())
            } else {
                found
            }
        })
    }
}

/// [`headless_gl`]: this context records nothing.
pub(in crate::app::model_preview) fn headless_gl_recording() -> glow::Context {
    headless_gl()
}

pub(in crate::app::model_preview) fn take_feedback_loops() -> Vec<String> {
    Vec::new()
}

pub(in crate::app::model_preview) fn take_gl_log() -> Vec<String> {
    Vec::new()
}
