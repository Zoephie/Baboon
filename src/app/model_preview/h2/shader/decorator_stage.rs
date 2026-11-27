//! Halo 2's decorator vertex shaders, translated from the source their tags
//! carry: `decorator.vertex_shader` and `decorator_decal.vertex_shader` keep
//! each geometry classification's HLSL ("asm-converted vs 2.0", every
//! constant in one `float4 c[208]`) in its `code`.
//!
//! Which classification draws a decorator is its class's, not its shader's
//! (MCC `halo2_tag_test`'s `sub_94BB80`): a model draws through the rigid
//! one, a screen-facing quad the seventh, an axis-rotating quad the eighth,
//! a cross quad the first, and a decal `decorator_decal`'s first. The dx9
//! classifications are translated: the dx11 ones carry the same bodies with a
//! depth rewrite through an engine constant (`c[10]`) the preview's clip
//! position has no need of.

use super::super::super::load::TagReader;
use crate::core::hlsl::emit::{Stage, translate};
use crate::core::hlsl::parse::parse;
use crate::core::hlsl::pre::Preprocessor;
use std::sync::Arc;

/// A decorator's geometry classification, by what its class draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DecoratorSlot {
    /// A cross quad: its corners baked in place.
    World,
    /// A model, placed by `c204` to `c206`.
    Rigid,
    ScreenFacing,
    AxisRotating,
    /// A floating or projected decal, through `decorator_decal`.
    Decal,
}

impl DecoratorSlot {
    /// Its element of the vertex shader's `geometry classifications dx9`.
    fn classification(self) -> usize {
        match self {
            Self::World | Self::Decal => 0,
            Self::Rigid => 1,
            Self::ScreenFacing => 7,
            Self::AxisRotating => 8,
        }
    }
}

/// Whether `path`, a pass's `vertex shader`, is a decorator's.
pub(crate) fn is_decorator_vertex_shader(path: &str) -> bool {
    let leaf = path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    leaf == "decorator" || leaf == "decorator_decal"
}

/// The dx9 pass's varyings, as the translated pixel stage reads every one:
/// a classification's output by its semantic, and what the preview's own
/// stage writes where it writes none (`stage.vert`).
const VARYINGS: [(&str, &str, &str); 11] = [
    ("v_COLOR0", "v_color0", "vec4(1.0)"),
    ("v_COLOR1", "v_color1", "vec4(1.0)"),
    ("v_TEXCOORD0", "v_tc0", "vec4(0.0, 0.0, 0.0, 1.0)"),
    ("v_TEXCOORD1", "v_tc1", "vec4(0.0, 0.0, 0.0, 1.0)"),
    ("v_TEXCOORD2", "v_tc2", "vec4(0.0, 0.0, 0.0, 1.0)"),
    ("v_TEXCOORD3", "v_tc3", "vec4(0.0, 0.0, 0.0, 1.0)"),
    ("v_TEXCOORD4", "v_tc4", "vec4(0.0, 0.0, 0.0, 1.0)"),
    ("v_TEXCOORD5", "v_tc5", "vec4(0.0, 0.0, 0.0, 1.0)"),
    ("v_TEXCOORD6", "v_tc6", "vec4(0.0, 0.0, 0.0, 1.0)"),
    ("v_TEXCOORD7", "v_tc7", "vec4(0.0, 0.0, 0.0, 1.0)"),
    ("v_FOG", "v_fog", "1.0"),
];

/// `slot`'s classification of the vertex shader at `path`, translated, its
/// outputs renamed to the dx9 pass's varyings and those it writes none of
/// given the preview's own stage's values.
pub(crate) fn decorator_vertex_source(
    reader: &TagReader,
    path: &str,
    slot: DecoratorSlot,
) -> Result<Arc<str>, String> {
    let tag = reader
        .load(path, "vertex_shader", b"vrtx")
        .map_err(|error| format!("{path}: {error}"))?;
    let classifications = tag
        .root()
        .field("geometry classifications dx9")
        .and_then(|field| field.as_block())
        .ok_or_else(|| format!("{path} has no dx9 classifications"))?;
    let code = classifications
        .element(slot.classification())
        .and_then(|element| element.field("code")?.as_data().map(<[u8]>::to_vec))
        .ok_or_else(|| format!("{path} has no code for {slot:?}"))?;
    let text: String = String::from_utf8_lossy(&code)
        .chars()
        .filter(|&c| c != '\0' && c != '\r')
        .collect();
    let mut include = |_: &str, _: &str| None;
    let tokens = Preprocessor::new(&[], &mut include).run(&text)?;
    let translated = translate(&parse(&tokens)?, "main", Stage::Vertex)?;
    Ok(Arc::from(as_dx9_stage(&translated.source)))
}

/// A translated vertex stage writing the dx9 pass's varyings.
fn as_dx9_stage(source: &str) -> String {
    let mut source = source.to_owned();
    let mut declarations = String::new();
    let mut defaults = String::new();
    for (semantic, varying, default) in VARYINGS {
        let kind = if varying == "v_fog" { "float" } else { "vec4" };
        let written = source.contains(&format!("out vec4 {semantic};"));
        if written {
            source = source.replace(semantic, varying);
        } else {
            declarations.push_str(&format!("out {kind} {varying};\n"));
            defaults.push_str(&format!("    {varying} = {default};\n"));
        }
    }
    let main = "void main() {\n";
    source.replacen(main, &format!("{declarations}{main}{defaults}"), 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a classification writes keeps its value under the dx9 name;
    /// what it does not is written as the preview's stage writes it.
    #[test]
    fn a_translated_stage_writes_every_dx9_varying() {
        let translated = "#version 410 core\nout vec4 v_COLOR0;\nout vec4 v_TEXCOORD0;\nvoid main() {\n    v_COLOR0 = vec4(0.5);\n    v_TEXCOORD0 = vec4(1.0);\n}\n";
        let stage = as_dx9_stage(translated);
        assert!(
            stage.contains("out vec4 v_color0;") && stage.contains("v_color0 = vec4(0.5);"),
            "{stage}"
        );
        assert!(stage.contains("v_tc0 = vec4(1.0);"), "{stage}");
        assert!(
            stage.contains("out vec4 v_color1;\n") && stage.contains("out float v_fog;\n"),
            "{stage}"
        );
        let defaults = stage.find("v_color1 = vec4(1.0);").unwrap();
        let body = stage.find("v_color0 = vec4(0.5);").unwrap();
        assert!(
            defaults < body,
            "defaults first, so the body's own writes stand"
        );
        assert!(!stage.contains("v_COLOR0"));
    }
}
