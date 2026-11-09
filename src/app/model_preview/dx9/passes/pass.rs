//! A dx9 pass: its blend, depth, cull, textures and constants, and the pass as drawn.

use super::animation::H2AnimatedTint;
use super::animation::{
    H2AnimatedRow, H2Animation, H2Clock, PassAnimation, PassVisibility, animated_vector,
};
use super::globals::fill_components;
use super::{DX9_TEXTURE_ROWS, Dx9VertexStage, Image, SamplerKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum Dx9BlendFactor {
    Zero,
    One,
    SrcColor,
    OneMinusSrcColor,
    SrcAlpha,
    OneMinusSrcAlpha,
    DstColor,
    OneMinusDstColor,
    DstAlpha,
    OneMinusDstAlpha,
    SrcAlphaSaturate,
    ConstantColor,
    OneMinusConstantColor,
    ConstantAlpha,
    OneMinusConstantAlpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum Dx9BlendEquation {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app) struct Dx9Blend {
    pub(in crate::app) equation: Dx9BlendEquation,
    pub(in crate::app) source: Dx9BlendFactor,
    pub(in crate::app) destination: Dx9BlendFactor,
    /// ARGB as stored, for the `constant` factors.
    pub(in crate::app) constant: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum Dx9Compare {
    Never,
    Less,
    Equal,
    LessOrEqual,
    Greater,
    NotEqual,
    GreaterOrEqual,
    Always,
}

/// Which faces a pass draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum Dx9Cull {
    /// As the preview's Render Backfaces has it.
    Frame,
    Back,
    Front,
    /// Both sides, whatever Render Backfaces says.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) struct Dx9Depth {
    /// `None` turns the depth test off.
    pub(in crate::app) compare: Option<Dx9Compare>,
    pub(in crate::app) write: bool,
}

/// One texture unit of a pass. A `None` image draws the unit's placeholder.
#[derive(Debug, Clone)]
pub(crate) struct Dx9DrawTexture {
    pub(crate) slot: u32,
    pub(crate) kind: SamplerKind,
    pub(crate) image: Option<std::sync::Arc<Image>>,
    /// Bound to the generated normalization cube instead of an image.
    pub(crate) normalization: bool,
    /// Bound to the preview's texaccum target instead of an image.
    pub(crate) texaccum: bool,
    /// Bound to a copy of the frame so far instead of an image.
    pub(crate) frame_copy: bool,
    /// Bound to a white volume. A default light's radius puts a model at the
    /// middle of its falloff, which reads 1.
    pub(crate) falloff: bool,
    /// Bound for each light from its own `gel map`.
    pub(crate) gel: bool,
}

/// A pass ready for the renderer: resolved, translated and decoded on the
/// worker, with no GL state yet.
#[derive(Debug, Clone)]
pub(crate) struct Dx9DrawPass {
    pub(crate) stage: Dx9VertexStage,
    pub(crate) fragment: String,
    pub(crate) blend: Option<Dx9Blend>,
    pub(crate) depth: Dx9Depth,
    /// Added to its window depth: what `rasterizer_dx9_set_decal_zbias` sets.
    pub(crate) depth_bias: f32,
    pub(crate) cull: Dx9Cull,
    /// Faded out entirely on a fully camouflaged object.
    pub(crate) hidden_by_camo: bool,
    /// Drawn only while this shows: one digit of a Halo CE numeric display.
    pub(crate) numeric: Option<std::sync::Arc<dyn PassVisibility>>,
    pub(crate) channels: [bool; 4],
    pub(crate) textures: Vec<Dx9DrawTexture>,
    /// Pixel constant registers, by register index.
    pub(crate) constants: Vec<(u32, [f32; 4])>,
    pub(crate) texture_rows: [[f32; 4]; DX9_TEXTURE_ROWS],
    pub(crate) tint_facing: [f32; 4],
    pub(crate) tint_glancing: [f32; 4],
    /// Constant components filled from the object's change colors at draw
    /// time: register, component, channel.
    pub(crate) change_colors: Vec<(u32, usize, usize)>,
    /// Constant components filled from the current light: register, component.
    pub(crate) light_colors: Vec<(u32, usize)>,
    /// Registers and components of `OBJECT active camo amount`, which holds
    /// for the object as it is drawn: plenty of passes read it with no camo on.
    pub(crate) camo_amounts: Vec<(u32, usize)>,
    /// The vectors a lightmap pass lights by. Most passes keep them in the
    /// tag, each as its pixel shader expects: some negate them.
    pub(crate) light_direct: [f32; 3],
    pub(crate) light_indirect: [f32; 3],
    /// Components of `constants` the shader animates: register, component.
    pub(in crate::app::model_preview) animated_constants: Vec<(u32, usize, Vec<H2Animation>)>,
    pub(in crate::app::model_preview) animated_rows: Vec<H2AnimatedRow>,
    /// One component of a row a stage reads as a value, and the value
    /// animations that replace it (`sub_6DE0F0`): row, component.
    pub(in crate::app::model_preview) animated_row_values: Vec<(usize, usize, Vec<H2Animation>)>,
    pub(in crate::app::model_preview) animated_tint_facing: Vec<H2AnimatedTint>,
    pub(in crate::app::model_preview) animated_tint_glancing: Vec<H2AnimatedTint>,
    /// Halo CE reworks its rows and a constant by its own functions.
    pub(crate) ce_animation: Option<std::sync::Arc<dyn PassAnimation>>,
}

impl Dx9DrawPass {
    /// A pass with nothing bound, its stage's values and textures to fill.
    pub(in crate::app::model_preview) fn new(
        stage: Dx9VertexStage,
        fragment: String,
        depth: Dx9Depth,
    ) -> Self {
        Self {
            stage,
            fragment,
            blend: None,
            depth,
            depth_bias: 0.0,
            cull: Dx9Cull::Frame,
            hidden_by_camo: false,
            numeric: None,
            channels: [true; 4],
            textures: Vec::new(),
            constants: Vec::new(),
            texture_rows: [[0.0; 4]; DX9_TEXTURE_ROWS],
            tint_facing: [1.0; 4],
            tint_glancing: [1.0; 4],
            change_colors: Vec::new(),
            light_colors: Vec::new(),
            camo_amounts: Vec::new(),
            light_direct: [0.0; 3],
            light_indirect: [0.0; 3],
            animated_constants: Vec::new(),
            animated_rows: Vec::new(),
            animated_row_values: Vec::new(),
            animated_tint_facing: Vec::new(),
            animated_tint_glancing: Vec::new(),
            ce_animation: None,
        }
    }

    /// Whether this pass looks different from one moment to the next.
    pub(crate) fn moves(&self) -> bool {
        self.animated_constants
            .iter()
            .flat_map(|(_, _, animations)| animations)
            .chain(self.animated_rows.iter().flat_map(|row| &row.animations))
            .chain(
                self.animated_row_values
                    .iter()
                    .flat_map(|(_, _, animations)| animations),
            )
            .chain(self.animated_tints())
            .any(H2Animation::moves)
            || self
                .ce_animation
                .as_ref()
                .is_some_and(|animation| animation.moves())
    }

    /// Whether it moves by itself while `outputs` hold.
    pub(crate) fn moves_with(&self, outputs: &[f32; 4]) -> bool {
        self.ce_animation
            .as_ref()
            .is_some_and(|animation| animation.moves_with(outputs))
    }

    /// The object functions its animations read.
    pub(crate) fn function_inputs(&self) -> impl Iterator<Item = &str> {
        let constants = self
            .animated_constants
            .iter()
            .flat_map(|(_, _, animations)| animations);
        constants
            .chain(self.animated_rows.iter().flat_map(|row| &row.animations))
            .chain(
                self.animated_row_values
                    .iter()
                    .flat_map(|(_, _, animations)| animations),
            )
            .chain(self.animated_tints())
            .flat_map(|animation| [&animation.held_input, &animation.range_input])
            .filter_map(|input| input.as_ref().map(|(name, _)| name.as_str()))
    }

    /// Whether it changes with the object's outputs.
    pub(crate) fn reads_outputs(&self) -> bool {
        self.numeric.is_some()
            || self
                .ce_animation
                .as_ref()
                .is_some_and(|animation| animation.reads_outputs())
    }

    pub(crate) fn texture_rows_at(
        &self,
        clock: H2Clock<'_>,
        outputs: &[f32; 4],
    ) -> [[f32; 4]; DX9_TEXTURE_ROWS] {
        let mut rows = self.texture_rows;
        self.animated_rows
            .iter()
            .for_each(|row| row.apply(&mut rows, clock));
        for (row, channel, animations) in &self.animated_row_values {
            if let Some(vector) = animated_vector(animations, clock) {
                rows[*row][*channel] = vector[*channel];
            }
        }
        if let Some(animation) = &self.ce_animation {
            animation.apply_rows(&mut rows, clock.time, outputs);
        }
        rows
    }

    fn animated_tints(&self) -> impl Iterator<Item = &H2Animation> {
        self.animated_tint_facing
            .iter()
            .chain(&self.animated_tint_glancing)
            .flat_map(|(_, animations)| animations)
    }

    pub(crate) fn tints_at(&self, clock: H2Clock<'_>) -> ([f32; 4], [f32; 4]) {
        let at = |mut tint: [f32; 4], animated: &[H2AnimatedTint]| {
            for (mask, animations) in animated {
                if let Some(vector) = animated_vector(animations, clock) {
                    fill_components(&mut tint, vector, *mask);
                }
            }
            tint
        };
        (
            at(self.tint_facing, &self.animated_tint_facing),
            at(self.tint_glancing, &self.animated_tint_glancing),
        )
    }

    /// Fill what the shader animates of `value`, the constant at `register`.
    pub(crate) fn animate_constant(&self, register: u32, value: &mut [f32; 4], clock: H2Clock<'_>) {
        for (animated, component, animations) in &self.animated_constants {
            if *animated == register
                && let Some(vector) = animated_vector(animations, clock)
            {
                value[*component] = vector[*component];
            }
        }
    }
}

/// One face of the cube that normalizes a vector by sampling it, in the GL
/// face order: each texel holds its own direction, packed to 0..1.
pub(crate) fn normalization_face(face: usize, size: usize) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let direction = match face {
                0 => [1.0, -v, -u],
                1 => [-1.0, -v, u],
                2 => [u, 1.0, v],
                3 => [u, -1.0, -v],
                4 => [u, -v, 1.0],
                _ => [-u, -v, -1.0],
            };
            let length = direction
                .iter()
                .map(|axis: &f32| axis * axis)
                .sum::<f32>()
                .sqrt();
            for axis in direction {
                rgba.push(((axis / length * 0.5 + 0.5) * 255.0).round() as u8);
            }
            rgba.push(255);
        }
    }
    rgba
}

/// The GL version every translated shader and vertex stage declares.
pub(crate) const PASS_GLSL_VERSION: &str = "#version 330\n";

/// Reflection vectors in the game carry a scale of about 2.
pub(crate) const DX9_REFLECTION_SCALE: f32 = 2.0;

/// Rows of an identity 2D transform: `x` in even registers, `y` in odd ones.
pub(in crate::app::model_preview) fn identity_rows() -> [[f32; 4]; DX9_TEXTURE_ROWS] {
    std::array::from_fn(|row| {
        if row % 2 == 0 {
            [1.0, 0.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0, 0.0]
        }
    })
}
