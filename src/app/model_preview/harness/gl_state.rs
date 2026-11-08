//! The GL state a paint leaves, for G-gl.

use eframe::glow;

/// The GL state a paint leaves, a line per setting. Bindings say only whether
/// something is bound (the framebuffer, whether it is the shot's own), as GL
/// names follow creation order, which a change of order alone would move.
pub(super) fn gl_state(gl: &glow::Context, target: glow::NativeFramebuffer) -> String {
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
