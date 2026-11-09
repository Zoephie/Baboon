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
            #[cfg(test)]
            super::glsl::record_source(source);
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

#[cfg(all(test, target_os = "macos"))]
mod gl_tests {
    use super::link;
    use crate::app::model_preview::harness::gl::headless_gl;
    use eframe::glow::{self, HasContext as _};

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
}
