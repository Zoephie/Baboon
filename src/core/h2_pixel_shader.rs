//! Halo 2 pixel shaders as GLSL, for the model preview.
//!
//! The kit ships each pass's pixel shader as generated HLSL
//! (`tags/rasterizer/pixel_shaders_dx9/*.psh`), and the game compiles those
//! same files at map bake. They are read from the user's kit and translated
//! when needed, so no shader source lives in this repo.
//!
//! A file is a header of `#const` bindings followed by `#case` sections, one
//! per variant. Selection follows the game's DX11 path: `DX11` is defined, and
//! `ALPHA_TEST` comes from the pass's alpha-test state.

use std::collections::HashMap;

/// How a pass's alpha test compares, as the game's shader defines name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AlphaCompare {
    Greater,
    Less,
    NotEqual,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AlphaTest {
    pub(crate) compare: AlphaCompare,
    pub(crate) reference: f32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PixelShaderOptions {
    /// `$name` values selecting a `#case` section. A name left out reads 0.
    pub(crate) cases: HashMap<String, i32>,
    pub(crate) alpha_test: Option<AlphaTest>,
}

/// `#const C1.rgb= $change_color_primary`: what fills a constant register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConstantBinding {
    pub(crate) register: u32,
    pub(crate) components: String,
    pub(crate) source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SamplerKind {
    Texture2D,
    Cube,
    Volume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SamplerBinding {
    pub(crate) slot: u32,
    pub(crate) kind: SamplerKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranslatedPixelShader {
    pub(crate) glsl: String,
    pub(crate) constants: Vec<ConstantBinding>,
    pub(crate) samplers: Vec<SamplerBinding>,
}

/// HLSL behaviour GLSL has no direct spelling for. `hlsl_any` keeps `any` of
/// a scalar, which the generated alpha tests subtract from. The `toN`
/// overloads are HLSL's implicit splat and truncation on assignment.
pub(crate) const HELPERS: &str = "\
float saturate(float x) { return clamp(x, 0.0, 1.0); }
vec2 saturate(vec2 x) { return clamp(x, 0.0, 1.0); }
vec3 saturate(vec3 x) { return clamp(x, 0.0, 1.0); }
vec4 saturate(vec4 x) { return clamp(x, 0.0, 1.0); }
float hlsl_any(float x) { return x != 0.0 ? 1.0 : 0.0; }
void clip(float x) { if (x < 0.0) discard; }
void clip(vec2 x) { if (any(lessThan(x, vec2(0.0)))) discard; }
void clip(vec3 x) { if (any(lessThan(x, vec3(0.0)))) discard; }
void clip(vec4 x) { if (any(lessThan(x, vec4(0.0)))) discard; }
float to1(float x) { return x; }
float to1(vec2 x) { return x.x; }
float to1(vec3 x) { return x.x; }
float to1(vec4 x) { return x.x; }
vec2 to2(float x) { return vec2(x); }
vec2 to2(vec2 x) { return x; }
vec2 to2(vec3 x) { return x.xy; }
vec2 to2(vec4 x) { return x.xy; }
vec3 to3(float x) { return vec3(x); }
vec3 to3(vec3 x) { return x; }
vec3 to3(vec4 x) { return x.xyz; }
vec4 to4(float x) { return vec4(x); }
vec4 to4(vec4 x) { return x; }
vec4 DX11_tex2D(sampler2D s, vec2 c) { return texture(s, c); }
vec4 DX11_tex2D(sampler2D s, vec3 c) { return texture(s, c.xy); }
vec4 DX11_tex2D(sampler2D s, vec4 c) { return texture(s, c.xy); }
vec4 DX11_tex2Dproj(sampler2D s, vec4 c) { return texture(s, c.xy / c.w); }
vec4 DX11_texCUBE(samplerCube s, vec3 c) { return texture(s, c); }
vec4 DX11_texCUBE(samplerCube s, vec4 c) { return texture(s, c.xyz); }
float hlsl_dot(float a, float b) { return a * b; }
float hlsl_lerp(float a, float b, float t) { return mix(a, b, t); }
float hlsl_max(float a, float b) { return max(a, b); }
float hlsl_pow(float a, float b) { return pow(a, b); }
";

/// `dot`, `lerp` and `max` promote scalar arguments in HLSL, in any position.
pub(crate) fn promoting_helpers() -> String {
    let mut out = String::new();
    for ty in ["vec2", "vec3", "vec4"] {
        out.push_str(&format!(
            "float hlsl_dot({ty} a, {ty} b) {{ return dot(a, b); }}\n\
             float hlsl_dot({ty} a, float b) {{ return dot(a, {ty}(b)); }}\n\
             float hlsl_dot(float a, {ty} b) {{ return dot({ty}(a), b); }}\n\
             {ty} hlsl_max({ty} a, {ty} b) {{ return max(a, b); }}\n\
             {ty} hlsl_max({ty} a, float b) {{ return max(a, {ty}(b)); }}\n\
             {ty} hlsl_max(float a, {ty} b) {{ return max({ty}(a), b); }}\n\
             {ty} hlsl_pow({ty} a, float b) {{ return pow(a, {ty}(b)); }}\n\
             {ty} hlsl_pow({ty} a, {ty} b) {{ return pow(a, b); }}\n"
        ));
        for (a, b, t) in [
            (ty, ty, ty),
            (ty, ty, "float"),
            (ty, "float", ty),
            (ty, "float", "float"),
            ("float", ty, ty),
            ("float", ty, "float"),
            ("float", "float", ty),
        ] {
            out.push_str(&format!(
                "{ty} hlsl_lerp({a} a, {b} b, {t} t) {{ return mix({ty}(a), {ty}(b), {ty}(t)); }}\n"
            ));
        }
    }
    out
}

/// What the game's vertex stage hands the pixel stage. HLSL may write to its
/// parameters and GLSL may not, so the body gets local copies.
const INPUTS: [&str; 10] = [
    "color0", "color1", "tc0", "tc1", "tc2", "tc3", "tc4", "tc5", "tc6", "tc7",
];

/// Some kit files stop before their `return` or hold only combiner source, so
/// they could not compile in the game either.
pub(crate) const INCOMPLETE: &str = "The pixel shader file is incomplete.";

pub(crate) fn translate(
    source: &str,
    version_declaration: &str,
    options: &PixelShaderOptions,
) -> Result<TranslatedPixelShader, String> {
    let source = strip_block_comments(&source.replace("\r\n", "\n"));
    let (header, section) = select_section(&source, &options.cases)?;
    if !section.contains("main(") {
        return Err(INCOMPLETE.to_owned());
    }
    let constants = header
        .lines()
        .filter_map(|line| line.strip_prefix("#const"))
        .map(parse_constant)
        .collect::<Result<Vec<_>, _>>()?;
    let lines = preprocess(section, options.alpha_test)?;

    let mut samplers = Vec::new();
    let mut uniforms = String::new();
    let mut body = String::new();
    let mut in_signature = false;
    let mut in_struct = false;
    let mut brace_depth = 0usize;
    let mut widths = HashMap::new();
    for line in &lines {
        let code = line.split("//").next().unwrap_or("").trim();
        if code.is_empty() || code.starts_with(';') {
            continue;
        }
        if in_struct {
            in_struct = !code.starts_with('}');
            continue;
        }
        if code.starts_with("struct ") {
            in_struct = true;
            continue;
        }
        if brace_depth == 0 {
            if in_signature {
                if code == "{" {
                    in_signature = false;
                    brace_depth = 1;
                }
                continue;
            }
            if code.contains("main(") {
                in_signature = true;
                continue;
            }
            if let Some(uniform) = translate_declaration(code, &mut samplers)? {
                uniforms.push_str(&uniform);
            }
            continue;
        }
        brace_depth += code.matches('{').count();
        brace_depth -= code.matches('}').count().min(brace_depth);
        if brace_depth == 0 {
            continue;
        }
        body.push_str("    ");
        let statement = translate_statement(code, options.alpha_test);
        body.push_str(&convert_assignments(&statement, &mut widths));
        body.push('\n');
    }
    if !body.contains("return ") {
        return Err(INCOMPLETE.to_owned());
    }

    let mut glsl = version_declaration.to_owned();
    for input in INPUTS {
        glsl.push_str(&format!("in vec4 v_{input};\n"));
    }
    glsl.push_str("in float v_fog;\nout vec4 out_color;\n");
    glsl.push_str(&uniforms);
    glsl.push_str(HELPERS);
    glsl.push_str(&promoting_helpers());
    glsl.push_str("vec4 pixel_main() {\n");
    for input in INPUTS {
        glsl.push_str(&format!("    vec4 {input} = v_{input};\n"));
    }
    glsl.push_str(
        "    vec4 tc[8] = vec4[8](tc0, tc1, tc2, tc3, tc4, tc5, tc6, tc7);\n\
         \x20   float fog = v_fog;\n    vec4 pos_unused = gl_FragCoord;\n",
    );
    glsl.push_str(&body);
    glsl.push_str("}\nvoid main() { out_color = pixel_main(); }\n");
    Ok(TranslatedPixelShader { glsl, constants, samplers })
}

/// Generated files can end in a `/* */` copy of their combiner source, which
/// reads as code to anything working line by line.
pub(crate) fn strip_block_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..].split_once("*/").map_or("", |(_, after)| after);
    }
    out.push_str(rest);
    out
}

/// Split a file into its shared header and the section `cases` selects. A
/// file with no `#case` is one section. Sections name every value they depend
/// on, so an unselected variant is an error rather than a guess.
fn select_section<'a>(
    source: &'a str,
    cases: &HashMap<String, i32>,
) -> Result<(&'a str, &'a str), String> {
    let mut starts = source
        .match_indices("#case")
        .filter(|(at, _)| *at == 0 || source.as_bytes()[at - 1] == b'\n')
        .map(|(at, _)| at)
        .peekable();
    let Some(&first) = starts.peek() else {
        return Ok((source, source));
    };
    let starts: Vec<usize> = starts.collect();
    for (index, &start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(source.len());
        let (line, rest) = source[start..end].split_once('\n').unwrap_or((&source[start..end], ""));
        let matches = line.split_whitespace().skip(1).all(|pair| {
            let Some((name, value)) = pair.split_once('=') else {
                return false;
            };
            let wanted = cases.get(name.trim_start_matches('$')).copied().unwrap_or(0);
            value.parse::<i32>() == Ok(wanted)
        });
        if matches {
            return Ok((&source[..first], rest));
        }
    }
    Err("No #case section matches the shader's values.".to_owned())
}

fn parse_constant(text: &str) -> Result<ConstantBinding, String> {
    let bad = || format!("Unreadable #const:{text}");
    let (target, source) = text.split_once('=').ok_or_else(bad)?;
    let (register, components) = target.trim().split_once('.').ok_or_else(bad)?;
    let register = register
        .strip_prefix(['C', 'c'])
        .and_then(|number| number.parse().ok())
        .ok_or_else(bad)?;
    Ok(ConstantBinding {
        register,
        components: components.to_owned(),
        source: source.trim().trim_start_matches('$').to_owned(),
    })
}

/// Resolve the `#ifdef` family. The files use no `#define` of their own, and
/// `#elif NAME` means "NAME is defined" as the game's compile defines it.
fn preprocess(section: &str, alpha_test: Option<AlphaTest>) -> Result<Vec<String>, String> {
    let mut defines = vec!["DX11"];
    if let Some(test) = alpha_test {
        defines.push("ALPHA_TEST");
        defines.push(match test.compare {
            AlphaCompare::Greater => "ALPHA_TEST_GREATER",
            AlphaCompare::Less => "ALPHA_TEST_LESS",
            AlphaCompare::NotEqual => "ALPHA_TEST_NOT_EQUAL",
        });
    }
    let defined = |name: &str| defines.contains(&name);

    // (branch is live, some branch of this #if has already been taken)
    let mut stack: Vec<(bool, bool)> = Vec::new();
    let mut lines = Vec::new();
    for line in section.lines() {
        let trimmed = line.trim_start();
        let word = trimmed.split_whitespace().nth(1).unwrap_or("");
        let enclosing = stack.iter().all(|(live, _)| *live);
        if trimmed.starts_with("#ifdef") {
            stack.push((defined(word), defined(word)));
        } else if trimmed.starts_with("#ifndef") {
            stack.push((!defined(word), !defined(word)));
        } else if trimmed.starts_with("#elif") {
            let (live, taken) = stack.last_mut().ok_or("#elif with no #if")?;
            *live = !*taken && defined(word);
            *taken |= *live;
        } else if trimmed.starts_with("#else") {
            let (live, taken) = stack.last_mut().ok_or("#else with no #if")?;
            *live = !*taken;
            *taken = true;
        } else if trimmed.starts_with("#endif") {
            stack.pop().ok_or("#endif with no #if")?;
        } else if !trimmed.starts_with('#') && enclosing {
            lines.push(line.to_owned());
        }
    }
    if stack.is_empty() {
        Ok(lines)
    } else {
        Err("An #if is never closed.".to_owned())
    }
}

/// A global declaration: textures and constant registers become uniforms, and
/// `SamplerState` has no GLSL counterpart.
fn translate_declaration(
    code: &str,
    samplers: &mut Vec<SamplerBinding>,
) -> Result<Option<String>, String> {
    for (prefix, kind, glsl_type) in [
        ("DX11_sampler2D(", SamplerKind::Texture2D, "sampler2D"),
        ("DX11_samplerCUBE(", SamplerKind::Cube, "samplerCube"),
    ] {
        if let Some(rest) = code.strip_prefix(prefix) {
            let slot = rest
                .split(')')
                .next()
                .and_then(|number| number.trim().parse().ok())
                .ok_or_else(|| format!("Unreadable sampler: {code}"))?;
            samplers.push(SamplerBinding { slot, kind });
            return Ok(Some(format!("uniform {glsl_type} s{slot};\n")));
        }
    }
    let declaration = code.split(':').next().unwrap_or(code).trim();
    let mut words = declaration.trim_end_matches(';').split_whitespace();
    let (Some(kind), Some(name)) = (words.next(), words.next()) else {
        return Ok(None);
    };
    let texture = match kind {
        "SamplerState" => return Ok(None),
        "Texture2D" => Some((SamplerKind::Texture2D, "sampler2D")),
        "TextureCube" => Some((SamplerKind::Cube, "samplerCube")),
        "Texture3D" => Some((SamplerKind::Volume, "sampler3D")),
        _ => None,
    };
    if let Some((kind, glsl_type)) = texture {
        let slot = name
            .strip_prefix('s')
            .and_then(|rest| rest.strip_suffix("tex"))
            .and_then(|number| number.parse().ok())
            .ok_or_else(|| format!("Unreadable texture name: {name}"))?;
        samplers.push(SamplerBinding { slot, kind });
        return Ok(Some(format!("uniform {glsl_type} s{slot};\n")));
    }
    match kind {
        "float4" => Ok(Some(format!("uniform vec4 {name};\n"))),
        "float3" => Ok(Some(format!("uniform vec3 {name};\n"))),
        "float2" => Ok(Some(format!("uniform vec2 {name};\n"))),
        "float" => Ok(Some(format!("uniform float {name};\n"))),
        // Its members follow as ordinary declarations.
        "cbuffer" => Ok(None),
        _ if code.contains('$') => Ok(None),
        _ => Err(format!("Unsupported declaration: {code}")),
    }
}

/// Rewrite one statement token by token. Integer literals become floats so
/// the result does not lean on implicit conversions, except a loop's counter.
pub(crate) fn translate_statement(code: &str, alpha_test: Option<AlphaTest>) -> String {
    let bytes = code.as_bytes();
    let integers = code.starts_with("for");
    let mut out = String::with_capacity(code.len());
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let end = word_end(bytes, at);
            let word = &code[at..end];
            let after_dot = at > 0 && bytes[at - 1] == b'.';
            if !after_dot
                && word.ends_with("tex")
                && code[end..].starts_with(".Sample(")
            {
                let sampler = word.trim_end_matches("tex");
                let arguments = end + ".Sample(".len();
                let comma = code[arguments..].find(',').map_or(arguments, |at| arguments + at + 1);
                out.push_str("texture(");
                out.push_str(sampler);
                out.push(',');
                at = comma;
                continue;
            }
            match word {
                _ if after_dot => out.push_str(word),
                "float4" | "half4" => out.push_str("vec4"),
                "float3" | "half3" => out.push_str("vec3"),
                "float2" | "half2" => out.push_str("vec2"),
                "half" => out.push_str("float"),
                "frac" => out.push_str("fract"),
                "lerp" => out.push_str("hlsl_lerp"),
                "dot" => out.push_str("hlsl_dot"),
                "max" => out.push_str("hlsl_max"),
                "pow" => out.push_str("hlsl_pow"),
                "any" => out.push_str("hlsl_any"),
                "ALPHA_TEST" => out.push_str(&float_literal(
                    alpha_test.map_or(0.0, |test| test.reference),
                )),
                _ => out.push_str(word),
            }
            at = end;
        } else if byte.is_ascii_digit() || (byte == b'.' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit)) {
            let mut end = at;
            // A `.` before a letter is a swizzle: `tc[0].xy`.
            while end < bytes.len()
                && (bytes[end].is_ascii_digit()
                    || (bytes[end] == b'.'
                        && !bytes.get(end + 1).is_some_and(|next| b"rgbaxyzw".contains(next))))
            {
                end += 1;
            }
            let number = &code[at..end];
            let index = out.ends_with('[');
            out.push_str(number);
            if !number.contains('.') && !integers && !index {
                out.push_str(".0");
            }
            if bytes.get(end) == Some(&b'f') {
                end += 1;
            }
            at = end;
        } else {
            out.push(byte as char);
            at += 1;
        }
    }
    out
}

/// The component count of a GLSL type name.
fn type_width(word: &str) -> Option<usize> {
    match word {
        "float" => Some(1),
        "vec2" => Some(2),
        "vec3" => Some(3),
        "vec4" => Some(4),
        _ => None,
    }
}

/// Split at each `=` that assigns, outside any parentheses.
fn split_assignments(statement: &str) -> Vec<&str> {
    let bytes = statement.as_bytes();
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0usize, 0);
    for (at, &byte) in bytes.iter().enumerate() {
        match byte {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b'=' if depth == 0
                && !matches!(bytes.get(at + 1), Some(b'='))
                && !matches!(bytes[..at].last(), Some(b'=' | b'<' | b'>' | b'!')) =>
            {
                parts.push(&statement[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(&statement[start..]);
    parts
}

/// The width of an assignment target: its swizzle, or its declared type.
fn target_width(target: &str, widths: &HashMap<String, usize>) -> Option<usize> {
    let target = target.trim();
    match target.split_once('.') {
        Some((_, swizzle)) if swizzle.bytes().all(|byte| b"rgbaxyzw".contains(&byte)) => {
            Some(swizzle.len())
        }
        Some(_) => None,
        None => widths.get(target).copied(),
    }
}

/// Record declarations, and give each assignment HLSL's implicit conversion.
pub(crate) fn convert_assignments(statement: &str, widths: &mut HashMap<String, usize>) -> String {
    let Some(code) = statement.trim_end().strip_suffix(';') else {
        return statement.to_owned();
    };
    let mut parts = split_assignments(code);
    let first = parts[0].trim();
    let mut declaration = None;
    if let Some((word, names)) = first.split_once(char::is_whitespace)
        && let Some(width) = type_width(word)
    {
        for name in names.split(',') {
            widths.insert(name.trim().to_owned(), width);
        }
        declaration = Some((word, names.trim(), width));
    }
    if parts.len() == 1 {
        return statement.to_owned();
    }
    let value = parts.pop().unwrap_or("").trim();
    let mut converted = value.to_owned();
    for target in parts.iter().rev() {
        let width = match declaration {
            Some((_, name, width)) if std::ptr::eq(*target, parts[0]) && !name.contains(',') => Some(width),
            _ => target_width(target, widths),
        };
        converted = match width {
            Some(width) => format!("{}= to{width}({converted})", target.trim()),
            None => format!("{}= {converted}", target.trim()),
        };
    }
    format!("{converted};")
}

fn word_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    end
}

fn float_literal(value: f32) -> String {
    format!("{value:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-written in the shape of a generated file, with both API branches.
    const FIXTURE: &str = "; generated\n\
        #const C1.rgb= $tint\n\
        #case $variant=0\n\
        #ifndef DX11\n\
        sampler2D s0 : register(s0);\n\
        #else\n\
        SamplerState s0 : register(s0);\n\
        Texture2D s0tex : register(t0);\n\
        #endif\n\
        float4 c1 : register(C1);\n\
        struct MRT_return\n{\n    float4 color :COLOR0;\n};\n\
        float4 main(\n    float4 tc0 : TEXCOORD0\n)\n    : COLOR\n{\n\
        \x20   float4 r0;\n\
        \x20   r0= s0tex.Sample(s0, tc0.xy/tc0.w);\n\
        \x20   r0.rgb= frac(saturate(r0.rgb)*2) * c1.rgb; // scale\n\
        #ifdef ALPHA_TEST\n\
        \x20   clip(r0.a - ALPHA_TEST);\n\
        #endif\n\
        \x20   return r0;\n}\n\
        #case $variant=1\n\
        float4 main()\n{\n    return 1;\n}\n";

    #[test]
    fn the_dx11_branch_of_the_selected_case_is_translated() {
        let shader = translate(FIXTURE, "#version 330\n", &Default::default()).unwrap();
        assert!(shader.glsl.contains("texture(s0, tc0.xy/tc0.w)"), "{}", shader.glsl);
        assert!(shader.glsl.contains("fract(saturate(r0.rgb)*2.0)"), "{}", shader.glsl);
        assert!(!shader.glsl.contains("sampler2D s0 :"), "{}", shader.glsl);
        assert!(!shader.glsl.contains("clip(r0.a"), "no alpha test was asked for");
        assert_eq!(shader.samplers, [SamplerBinding { slot: 0, kind: SamplerKind::Texture2D }]);
        assert_eq!(
            shader.constants,
            [ConstantBinding { register: 1, components: "rgb".into(), source: "tint".into() }]
        );
    }

    #[test]
    fn cases_select_a_section_and_an_unknown_value_is_an_error() {
        let pick = |value| {
            let cases = HashMap::from([("variant".to_owned(), value)]);
            translate(FIXTURE, "", &PixelShaderOptions { cases, alpha_test: None })
        };
        assert!(pick(1).unwrap().glsl.contains("return 1.0;"));
        assert!(pick(0).unwrap().glsl.contains("return r0;"));
        assert!(pick(2).is_err());
    }

    #[test]
    fn alpha_test_uses_the_pass_reference() {
        let options = PixelShaderOptions {
            alpha_test: Some(AlphaTest { compare: AlphaCompare::Greater, reference: 0.5 }),
            ..Default::default()
        };
        let shader = translate(FIXTURE, "", &options).unwrap();
        assert!(shader.glsl.contains("clip(r0.a - 0.5);"), "{}", shader.glsl);
    }

    #[test]
    fn assignments_convert_as_hlsl_does() {
        let mut widths = HashMap::new();
        let convert = |text: &str, widths: &mut _| convert_assignments(text, widths);
        assert_eq!(convert("vec3 a, b;", &mut widths), "vec3 a, b;");
        assert_eq!(convert("a= 1.0;", &mut widths), "a= to3(1.0);");
        assert_eq!(convert("vec4 c= b.r;", &mut widths), "vec4 c= to4(b.r);");
        assert_eq!(
            convert("a.r= b.g= 0.0;", &mut widths),
            "a.r= to1(b.g= to1(0.0));"
        );
        assert_eq!(convert("a.rg= f(x == 1.0);", &mut widths), "a.rg= to2(f(x == 1.0));");
    }

    #[test]
    fn block_comments_do_not_reach_the_body() {
        assert_eq!(strip_block_comments("a/*\nb\n*/c/* open"), "ac");
        let source = format!("{FIXTURE}/*\nR0a= C0a*C1a\n*/");
        assert!(translate(&source, "", &Default::default()).is_ok());
    }

    #[test]
    fn array_indices_and_loop_counters_stay_integers() {
        assert_eq!(translate_statement("r0= tc[1].xy;", None), "r0= tc[1].xy;");
        assert_eq!(translate_statement("r0= 1.5 + 2;", None), "r0= 1.5 + 2.0;");
        assert_eq!(translate_statement("r0= 1.f / 2.f;", None), "r0= 1. / 2.;");
        assert_eq!(
            translate_statement("for (int i = 0; i < 3; i++)", None),
            "for (int i = 0; i < 3; i++)"
        );
    }

    #[test]
    fn unbalanced_conditionals_are_errors() {
        assert!(preprocess("#ifdef DX11\n", None).is_err());
        assert!(preprocess("#endif\n", None).is_err());
    }

    /// Every `#case` of a file as option sets.
    fn variants(source: &str) -> Vec<HashMap<String, i32>> {
        let found: Vec<_> = source
            .lines()
            .filter(|line| line.starts_with("#case"))
            .map(|line| {
                line.split_whitespace()
                    .skip(1)
                    .filter_map(|pair| {
                        let (name, value) = pair.split_once('=')?;
                        Some((name.trim_start_matches('$').to_owned(), value.parse().ok()?))
                    })
                    .collect()
            })
            .collect();
        if found.is_empty() { vec![HashMap::new()] } else { found }
    }

    /// With `BLAM_TEST_H2EK` and `glslangValidator`, compile every variant of
    /// every shader the kit ships.
    #[test]
    fn every_kit_pixel_shader_compiles_under_glslang() {
        let Ok(root) = std::env::var("BLAM_TEST_H2EK") else {
            eprintln!("skipping: BLAM_TEST_H2EK is not set");
            return;
        };
        if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
            eprintln!("skipping: glslangValidator is not on PATH");
            return;
        }
        let shaders = std::path::Path::new(&root).join("rasterizer/pixel_shaders_dx9");
        let dir = crate::core::test_kits::unique_temp_dir("h2_pixel_shaders");
        let (mut compiled, mut incomplete, mut failures) = (0usize, 0usize, Vec::new());
        for entry in std::fs::read_dir(&shaders).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "psh") {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let source = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
            let tests = [
                None,
                Some(AlphaTest { compare: AlphaCompare::Greater, reference: 0.5 }),
                Some(AlphaTest { compare: AlphaCompare::NotEqual, reference: 0.0 }),
            ];
            let variants = variants(&source);
            let runs = variants.iter().enumerate().flat_map(|(index, cases)| {
                tests.iter().map(move |alpha_test| (index, cases.clone(), *alpha_test))
            });
            for (index, cases, alpha_test) in runs {
                let options = PixelShaderOptions { cases, alpha_test };
                let glsl = match translate(&source, "#version 330\n", &options) {
                    Ok(shader) => shader.glsl,
                    Err(error) if error == INCOMPLETE => {
                        incomplete += 1;
                        continue;
                    }
                    Err(error) => {
                        failures.push(format!("{name}#{index}: {error}"));
                        continue;
                    }
                };
                let file = dir.join("shader.frag");
                std::fs::write(&file, glsl).unwrap();
                let output = std::process::Command::new("glslangValidator").arg(&file).output().unwrap();
                if output.status.success() {
                    compiled += 1;
                } else {
                    let log = String::from_utf8_lossy(&output.stdout).into_owned();
                    let first = log.lines().find(|line| line.starts_with("ERROR")).unwrap_or("").to_owned();
                    failures.push(format!("{name}#{index}: {first}"));
                }
            }
        }
        eprintln!("{compiled} variants compiled, {incomplete} incomplete in the kit");
        assert!(compiled > 0, "no shaders under {}", shaders.display());
        assert!(
            failures.is_empty(),
            "{compiled} compiled, {incomplete} incomplete, {} failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
