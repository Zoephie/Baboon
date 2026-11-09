//! CPU vertex building for Halo 2, Halo 3, ODST and Reach: billboards, frames and the per-system vertex pass.

use super::*;

/// `billboard_basis`.
fn billboard(
    style: Billboard,
    place: [f32; 3],
    velocity: [f32; 3],
    local: [[f32; 3]; 3],
    view: &ViewFrame,
) -> [[f32; 3]; 2] {
    let eye: [f32; 3] = std::array::from_fn(|axis| place[axis] - view.eye[axis]);
    match style {
        Billboard::ScreenFacing => [view.right, view.up],
        Billboard::CameraFacing => {
            let eye = clamped_unit(eye);
            let other = if eye[2].abs() < 0.99 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
            let across = clamped_unit(cross(eye, other));
            [across, clamped_unit(cross(across, eye))]
        }
        Billboard::ScreenParallel => {
            let along = clamped_unit(velocity);
            [along, clamped_unit(cross(along, eye))]
        }
        Billboard::ScreenPerpendicular => {
            let motion = clamped_unit(velocity);
            let other = if motion[2].abs() < 0.99 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
            let across = clamped_unit(cross(motion, other));
            [across, cross(motion, across)]
        }
        Billboard::ScreenVertical => {
            let up = [0.0, 0.0, 1.0];
            [up, clamped_unit(cross(up, eye))]
        }
        Billboard::LocalVertical => [local[2], clamped_unit(cross(local[2], eye))],
        Billboard::LocalHorizontal => [local[0], local[1]],
        Billboard::ScreenHorizontal | Billboard::World => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        Billboard::VelocityHorizontal => {
            let along = clamped_unit(velocity);
            let other = if along[0].abs() < 0.99 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
            [along, clamped_unit(cross(other, along))]
        }
    }
}

/// `particle_render_fast.hlsl`'s `billboard_basis(…, limited = true)`: only
/// screen and camera facing, and every other style lies flat in the world.
pub(super) fn cheap_billboard(style: Billboard) -> Billboard {
    match style {
        Billboard::ScreenFacing | Billboard::CameraFacing => style,
        _ => Billboard::World,
    }
}

/// The frame of the second sprite texcoord (`particle_render.hlsl`'s
/// `DOUBLE_SPRITE_TEXCOORD`): the next when frames blend; for Reach's
/// plasmas without, `floor(seed 1 · 16)` frames on.
pub(super) fn second_frame(frame: f32, frame_blend: bool, plasma_second_sprite: bool, seed: f32) -> f32 {
    if plasma_second_sprite && !frame_blend { frame.floor() + (seed * 16.0).floor() } else { frame + 1.0 }
}

/// `effect_lightprobe_get_lightmap_tint`, as `initial_color.rgb * exp2(a)`
/// comes out: the probe's constant term, with a brightness of at most 2.
pub(in crate::app::model_preview) fn lightmap_tint(lighting: &[[f32; 4]; 10]) -> [f32; 3] {
    let scale = (4.0 * std::f32::consts::PI).sqrt().recip();
    let tint: [f32; 3] = std::array::from_fn(|channel| lighting[0][channel] * scale);
    let brightest = tint[0].max(tint[1]).max(tint[2]).max(f32::EPSILON);
    let exponent = brightest.log2().clamp(0.0, 1.0);
    tint.map(|channel| (channel * 2f32.powf(-exponent)).clamp(0.0, 1.0) * 2f32.powf(exponent))
}

/// `surface_basis`, for a distortion: its corners lean away as a dome's would.
pub(super) fn curved_lean(lean: [[f32; 3]; 2], plane: [[f32; 3]; 2], offset: [f32; 2], curvature: f32) -> [[f32; 3]; 2] {
    let norm = (offset[0] * offset[0] + offset[1] * offset[1]).sqrt() + 0.0001;
    let [x, y] = offset.map(|side| side / norm);
    let axis = std::array::from_fn(|axis| -y * plane[0][axis] + x * plane[1][axis]);
    let rotation = axis_rotation(axis, curvature * std::f32::consts::FRAC_PI_2);
    lean.map(|side| std::array::from_fn(|column| (0..3).map(|row| side[row] * rotation[row][column]).sum()))
}

/// `matrix3x3_rotation_from_axis_and_angle`.
pub(super) fn axis_rotation(axis: [f32; 3], angle: f32) -> [[f32; 3]; 3] {
    let (sin, cos) = angle.sin_cos();
    let [x, y, z] = axis;
    let rest = axis.map(|value| value * (1.0 - cos));
    [
        [rest[0] * x + cos, rest[1] * x + sin * z, rest[2] * x - sin * y],
        [rest[0] * y - sin * z, rest[1] * y + cos, rest[2] * y + sin * x],
        [rest[0] * z + sin * y, rest[1] * z - sin * x, rest[2] * z + cos],
    ]
}

/// `frame_texcoord`.
fn sprite_uv(frame: [f32; 4], uv: [f32; 2], scroll: [f32; 2], scale: f32) -> [f32; 2] {
    let [left, top, width, height] = frame;
    let shrink = 0.5 * (1.0 - scale);
    [
        uv[0] * width * scale + scroll[0] * width.abs() + left + shrink * width,
        uv[1] * height * scale + scroll[1] * height.abs() + top + shrink * height,
    ]
}

impl ParticleSystem {
    /// A location's quads, at the end of `vertices`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn vertices_at(
        &self,
        running: &[RunningEmitter],
        (origin, axes, onset): ([f32; 3], [[f32; 3]; 3], f32),
        values: Option<&FunctionValues>,
        view: &ViewFrame,
        lighting: &[[f32; 4]; 10],
        time: f32,
        stops: f32,
        vertices: &mut Vec<ParticleCorner>,
    ) {
        let system = self;
        let look = &*system.look;
        // Halo 4's draw through their material, from packed state.
        if look.h4.is_some() {
            return;
        }
        let light = particle_exposure(look.shader.blend, look.exposed, look.reach, stops);
        let initial = match (look.tint_from_lightmap, look.reach) {
            (false, _) => [1.0; 3],
            (true, false) => lightmap_tint(lighting),
            (true, true) => reach_lightmap_tint(),
        };
        let ambient = reach_particle_light().ambient;
        // `particle_render_fast.hlsl`: no aspect, no blur, no quarter turn.
        let style = if look.cheap { cheap_billboard(look.billboard) } else { look.billboard };
        for (emitter, running) in system.emitters.iter().zip(running) {
            let base = ParticleState {
                system_age: system.times.age(system.event, time - onset).unwrap_or(1.0),
                game_time: particle_game_time(look.reach, time),
                effect_a: system_state(system, values, time, time - onset).map_or(0.0, |state| state.effect_a),
                effect_b: system_state(system, values, time, time - onset).map_or(1.0, |state| state.effect_b),
                location_seed: system.location_seed,
                lod: {
                    let away = minus(origin, view.eye);
                    lod_amount(system.lod, dot(away, away).sqrt())
                },
                ..Default::default()
            };
            // `calc_matrix`, or none for what already lies in the world.
            const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            let (offset, turned) = if system.world { ([0.0; 3], IDENTITY) } else { emitter_frame(emitter, origin, axes, &base) };
            for particle in &running.particles {
                let state = particle.state(base);
                let to_world = |vector: [f32; 3]| -> [f32; 3] {
                    std::array::from_fn(|axis| (0..3).map(|at| turned[at][axis] * vector[at]).sum())
                };
                let place: [f32; 3] = {
                    let moved = to_world(particle.position);
                    std::array::from_fn(|axis| offset[axis] + moved[axis])
                };
                let velocity = to_world(particle.velocity);
                let size = emitter.size.scalar(&state) * emitter.scale.scalar(&state);
                let mut aspect = if look.cheap { 1.0 } else { look.aspect.scalar(&state) };
                if let Some(stretch) = look.blur_aspect.filter(|_| !look.cheap) {
                    // A baseline of 30, not the frame rate. The preview's observer is still.
                    aspect += dot(velocity, velocity).sqrt() * stretch / (30.0 * size);
                }
                let basis = billboard(style, place, velocity, turned, view);
                let manual = emitter.rotation.scalar(&state).rem_euclid(1.0);
                let turned_up = if look.vertical && !look.cheap { 0.25 } else { 0.0 };
                let spun = if emitter.rotation_given { 0.0 } else { particle.rotation };
                let (sin, cos) = (std::f32::consts::TAU * (spun + manual + turned_up)).sin_cos();
                let tint = emitter.tint.color(&state);
                let color = look.color.color(&state);
                let intensity = look.intensity.scalar(&state);
                let mut alpha = (emitter.alpha.scalar(&state) * look.alpha.scalar(&state)).clamp(0.0, 1.0);
                // `default_vs`: wholly transparent is dead, whatever the blend.
                if alpha == 0.0 {
                    continue;
                }
                if look.intensity_fades {
                    alpha *= intensity;
                }
                let eye: [f32; 3] = std::array::from_fn(|axis| place[axis] - view.eye[axis]);
                let near = system.near_range * (dot(view.forward, eye) - system.near_cutoff);
                alpha *= if near.is_nan() { 0.0 } else { near.clamp(0.0, 1.0) };
                if alpha == 0.0 && look.model.is_none() {
                    continue;
                }
                let rgb: [f32; 3] = std::array::from_fn(|axis| {
                    (tint[axis] * color[axis]).clamp(0.0, 1.0) * initial[axis] * intensity * light
                });
                let shading = look.shader.lighting;
                let normal_at = |planar: [f32; 2], spun: [[f32; 3]; 2]| -> [f32; 3] {
                    let normal = clamped_unit(cross(spun[0], spun[1]));
                    if shading == ParticleLighting::None {
                        return normal;
                    }
                    let tangent = (look.curvature * std::f32::consts::FRAC_PI_2).tan();
                    let bent: [f32; 3] = if tangent > 0.0 {
                        let plane: [f32; 3] = std::array::from_fn(|axis| spun[0][axis] * planar[0] + spun[1][axis] * planar[1]);
                        let plane = clamped_unit(plane);
                        std::array::from_fn(|axis| normal[axis] + plane[axis])
                    } else {
                        normal
                    };
                    clamped_unit(bent)
                };
                // `per_vertex_ravi_order_0` on the vertex; order 3 on the pixel.
                // Reach's `per_vertex_ambient` is the probe's ambient, whatever
                // the normal.
                let lit_by = |normal: [f32; 3]| match shading {
                    ParticleLighting::Ambient => ravi(shading, normal, lighting),
                    ParticleLighting::ReachAmbient => ambient,
                    _ => [1.0; 3],
                };
                let (frames, corner) = match look.sequences.get(particle.sequence) {
                    Some((frames, corner)) => (frames, *corner),
                    None => (&look.frames, look.corner),
                };
                let count = frames.len();
                let frame_at = particle.frame + look.frame.scalar(&state).rem_euclid(1.0);
                let frame = frame_variant(frame_at, count, look.one_shot, look.backwards && particle.flip[2]);
                let first = frames[frame.floor().rem_euclid(count as f32) as usize % count];
                let shader = &look.shader;
                let next = second_frame(frame, shader.frame_blend, shader.plasma_second_sprite, particle.seeds[0]);
                let second = frames[next.floor().rem_euclid(count as f32) as usize % count];
                let blend = if shader.frame_blend && !look.cheap { frame.fract() } else { 0.0 };
                let scroll = emitter.scroll.map(|rate| rate * time);
                let [from, to] = look.shader.uv_scale;
                let uv_scale = |at: f32| (from + (to - from) * at / 2.0).recip();
                let (first_scale, second_scale) =
                    if look.shader.frame_blend { (uv_scale(1.0 + blend), uv_scale(blend)) } else { (1.0, 1.0) };
                let black = emitter.black_point.scalar(&state).clamp(0.0, 0.9999);
                // The cheap shader's `palette_v` is a literal half.
                let palette = if look.cheap { 0.5 } else { look.palette.scalar(&state).clamp(0.0, 0.9999) };
                let [x, y, width, height] = corner;
                let at_spot = |spot: [f32; 3], mut uv: [f32; 2], normal: [f32; 3], lit: [f32; 3], lean: [[f32; 3]; 2]| {
                    let mut faded = alpha;
                    if let Some([range, cutoff]) = look.edge_fade {
                        let toward = clamped_unit(minus(spot, view.eye));
                        let angle = std::f32::consts::FRAC_PI_2 - dot(toward, normal).abs().min(1.0).acos();
                        faded *= (range * (angle - cutoff)).clamp(0.0, 1.0);
                    }
                    if look.flip_u && particle.flip[0] && !look.cheap {
                        uv[0] = 1.0 - uv[0];
                    }
                    if look.flip_v && particle.flip[1] && !look.cheap {
                        uv[1] = 1.0 - uv[1];
                    }
                    // The cheap shader samples every map at the quad's own uv.
                    let (sprite, next_sprite) = if look.cheap {
                        (uv, uv)
                    } else {
                        (sprite_uv(first, uv, scroll, first_scale), sprite_uv(second, uv, scroll, second_scale))
                    };
                    ParticleCorner {
                        place: spot,
                        sprite,
                        next_sprite,
                        billboard: uv,
                        color: [rgb[0] * lit[0], rgb[1] * lit[1], rgb[2] * lit[2], faded],
                        extra: [blend, palette, black],
                        lean,
                        normal,
                    }
                };
                if let Some(meshes) = &look.model {
                    let angle = std::f32::consts::TAU * (particle.rotation + manual);
                    let variant = frame_variant(frame_at, meshes.len(), look.one_shot, look.backwards && particle.flip[2]);
                    let mesh = &meshes[variant.floor().rem_euclid(meshes.len() as f32) as usize % meshes.len()];
                    let plane = [basis[0], basis[1], cross(basis[0], basis[1])];
                    let plane = axis_rotation(particle.axis, angle).map(|row| -> [f32; 3] {
                        std::array::from_fn(|axis| (0..3).map(|at| row[at] * plane[at][axis]).sum())
                    });
                    let along = |vector: [f32; 3]| -> [f32; 3] {
                        std::array::from_fn(|axis| (0..3).map(|at| vector[at] * plane[at][axis]).sum())
                    };
                    for triangle in mesh.chunks_exact(3) {
                        // Backfaces cull among the transparents.
                        let turned_away = triangle.iter().all(|vertex| {
                            let normal = along([vertex.normal.i, vertex.normal.j, vertex.normal.k]);
                            dot(normal, minus(view.eye, place)) < 0.0
                        });
                        if turned_away && !look.double_sided {
                            continue;
                        }
                        for vertex in triangle {
                        let at = [vertex.position.x * aspect, vertex.position.y, vertex.position.z];
                        let moved = along(at);
                        let spot = std::array::from_fn(|axis| place[axis] + moved[axis] * size);
                        let normal = clamped_unit(along([vertex.normal.i, vertex.normal.j, vertex.normal.k]));
                        let lit = lit_by(normal);
                        let lean = [plane[0], plane[1]].map(|side| side.map(|axis| axis * size * width));
                        vertices.push(at_spot(spot, [vertex.texcoord.x, vertex.texcoord.y], normal, lit, lean));
                        }
                    }
                    continue;
                }
                let corner = |shift: [f32; 2]| {
                    let planar = [(shift[0] * width + x) * aspect, shift[1] * height + y];
                    let spun: [[f32; 3]; 2] = [
                        std::array::from_fn(|axis| cos * basis[0][axis] + sin * basis[1][axis]),
                        std::array::from_fn(|axis| -sin * basis[0][axis] + cos * basis[1][axis]),
                    ];
                    let spot: [f32; 3] = std::array::from_fn(|axis| {
                        place[axis] + (spun[0][axis] * planar[0] + spun[1][axis] * planar[1]) * size
                    });
                    let lean = [spun[0].map(|axis| axis * size * width), spun[1].map(|axis| axis * size * height)];
                    let lean = if look.shader.distortion.is_some() { curved_lean(lean, spun, planar, look.curvature) } else { lean };
                    let normal = normal_at(planar, spun);
                    at_spot(spot, shift, normal, lit_by(normal), lean)
                };
                let quad = [corner([0.0, 0.0]), corner([1.0, 0.0]), corner([1.0, 1.0]), corner([0.0, 1.0])];
                vertices.extend([quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
            }
        }
    }
}
