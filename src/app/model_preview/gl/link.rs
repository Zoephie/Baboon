//! One way to build a program from its stages.

use eframe::glow::{self, HasContext};

/// Compile each of `stages` (a shader kind and its source), bind each of
/// `attributes` (a location and its name), and link them into a program.
/// Whatever fails first is the error, and nothing made on the way is left
/// behind: no shader outlives the call, and a failed program is deleted.
///
/// # Safety
///
/// `gl` must be current on this thread.
pub(in crate::app::model_preview) unsafe fn link(
    gl: &glow::Context,
    stages: &[(u32, &str)],
    attributes: &[(u32, &str)],
) -> Result<glow::NativeProgram, String> {
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::with_capacity(stages.len());
        let release = |shaders: &[glow::NativeShader]| {
            for shader in shaders {
                gl.detach_shader(program, *shader);
                gl.delete_shader(*shader);
            }
        };
        for (kind, source) in stages {
            let shader = match gl.create_shader(*kind) {
                Ok(shader) => shader,
                Err(error) => {
                    release(&shaders);
                    gl.delete_program(program);
                    return Err(error);
                }
            };
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                release(&shaders);
                gl.delete_program(program);
                return Err(format!("{} failed to compile: {log}", stage_name(*kind)));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        for (location, name) in attributes {
            gl.bind_attrib_location(program, *location, name);
        }
        gl.link_program(program);
        release(&shaders);
        if gl.get_program_link_status(program) {
            Ok(program)
        } else {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            Err(format!("failed to link: {log}"))
        }
    }
}

fn stage_name(kind: u32) -> &'static str {
    match kind {
        glow::VERTEX_SHADER => "vertex shader",
        glow::FRAGMENT_SHADER => "fragment shader",
        _ => "shader",
    }
}
