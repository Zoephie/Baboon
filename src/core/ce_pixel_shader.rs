//! Halo CE pixel shaders as GLSL, for the model preview.
//!
//! MCC's kit ships the game's shaders as HLSL under `shaders/fx`, beside the
//! `tags` folder. They are read from the user's kit and translated when
//! needed, so no shader source lives in this repo. Where Halo 2's files are
//! one generated `main`, these hold ordinary functions a technique calls with
//! compile-time arguments, so the caller names that call and the defines.

use std::collections::{HashMap, HashSet};

use super::h2_pixel_shader::{
    HELPERS, SamplerBinding, SamplerKind, convert_assignments, promoting_helpers,
    strip_block_comments, translate_statement,
};

/// `Texture0`, or any other name ending in its register.
const MAX_TEXTURES: u32 = 8;
/// Constants bound to a register land here, so an index may be computed.
const REGISTER_ARRAY: &str = "ce_registers";
const REGISTER_COUNT: usize = 32;
/// HLSL's `dot` of a `float4` and a `float3` drops the fourth component.
const DOT_TRUNCATING: &str = "float hlsl_dot(vec4 a, vec3 b) { return dot(a.xyz, b); }\n";
/// Names GLSL keeps for itself that the kit's files use.
const RENAMED: [(&str, &str); 2] = [("main", "pixel_main"), ("input", "ps_input")];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranslatedFxShader {
    /// Everything but the entry point, which `TranslatedFxShader::entry` adds.
    glsl: String,
    input: String,
    /// `cbuffer` members in order: each is the uniform `c{index}`.
    pub(crate) constants: Vec<String>,
    pub(crate) samplers: Vec<SamplerBinding>,
}

impl TranslatedFxShader {
    /// The whole fragment shader, returning `call`.
    pub(crate) fn entry(&self, call: &str) -> String {
        self.entry_tested(call, None)
    }

    /// `entry`, with the render state's alpha test the kit's file leaves out.
    pub(crate) fn entry_tested(&self, call: &str, alpha_reference: Option<f32>) -> String {
        let test = alpha_reference
            .map(|reference| format!("    clip(out_color.a - {reference:?});\n"))
            .unwrap_or_default();
        format!(
            "{}void main() {{\n    PS_INPUT i = {};\n    out_color = {call};\n{test}}}\n",
            self.glsl, self.input
        )
    }

    /// The uniform register of a `cbuffer` member.
    pub(crate) fn register(&self, name: &str) -> Option<u32> {
        self.constants.iter().position(|constant| constant == name).map(|at| at as u32)
    }
}

/// Inline `#include`s, each once.
fn expand(
    file: &str,
    read: &impl Fn(&str) -> Result<String, String>,
    seen: &mut HashSet<String>,
    out: &mut String,
) -> Result<(), String> {
    if !seen.insert(file.to_owned()) {
        return Ok(());
    }
    let text = strip_block_comments(&read(file)?.replace("\r\n", "\n")).replace("\\\n", " ");
    for line in text.lines() {
        match line.trim().strip_prefix("#include") {
            Some(rest) => expand(rest.trim().trim_matches('"'), read, seen, out)?,
            None => {
                out.push_str(line.split("//").next().unwrap_or(""));
                out.push('\n');
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct Macro {
    parameters: Option<Vec<String>>,
    body: String,
}

fn word_at(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    end
}

/// Split at commas outside any brackets.
fn split_arguments(text: &str) -> Vec<String> {
    let (mut parts, mut depth, mut start) = (Vec::new(), 0usize, 0);
    for (at, byte) in text.bytes().enumerate() {
        match byte {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(text[start..at].trim().to_owned());
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(text[start..].trim().to_owned());
    parts
}

/// The end of the bracket opening at `open`.
fn closing(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (at, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

fn substitute(body: &str, parameters: &[String], arguments: &[String]) -> String {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
            let end = word_at(bytes, at);
            let word = &body[at..end];
            let swizzle = at > 0 && bytes[at - 1] == b'.';
            match parameters.iter().position(|name| name == word).filter(|_| !swizzle) {
                Some(index) => out.push_str(arguments.get(index).map_or("", String::as_str)),
                None => out.push_str(word),
            }
            at = end;
        } else if bytes[at].is_ascii_digit() {
            let end = word_at(bytes, at);
            out.push_str(&body[at..end]);
            at = end;
        } else {
            out.push(bytes[at] as char);
            at += 1;
        }
    }
    out
}

fn expand_macros(text: &str, macros: &HashMap<String, Macro>) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < bytes.len() {
        if !(bytes[at].is_ascii_alphabetic() || bytes[at] == b'_') {
            if bytes[at].is_ascii_digit() {
                let end = word_at(bytes, at);
                out.push_str(&text[at..end]);
                at = end;
            } else {
                out.push(bytes[at] as char);
                at += 1;
            }
            continue;
        }
        let end = word_at(bytes, at);
        let word = &text[at..end];
        let swizzle = at > 0 && bytes[at - 1] == b'.';
        let Some(found) = macros.get(word).filter(|_| !swizzle) else {
            out.push_str(word);
            at = end;
            continue;
        };
        let Some(parameters) = &found.parameters else {
            out.push_str(&expand_macros(&found.body, macros)?);
            at = end;
            continue;
        };
        let open = end + text[end..].len() - text[end..].trim_start().len();
        if bytes.get(open) != Some(&b'(') {
            out.push_str(word);
            at = end;
            continue;
        }
        let close = closing(bytes, open).ok_or_else(|| format!("Unclosed call of {word}"))?;
        // An argument can itself expand to the whole list.
        let inner = expand_macros(&text[open + 1..close], macros)?;
        let body = substitute(&found.body, parameters, &split_arguments(&inner));
        out.push_str(&expand_macros(&body, macros)?);
        at = close + 1;
    }
    Ok(out)
}

/// Follow the conditionals and macros, leaving plain code.
fn preprocess(source: &str, defines: &[(&str, &str)]) -> Result<String, String> {
    let mut macros: HashMap<String, Macro> = defines
        .iter()
        .map(|(name, value)| {
            ((*name).to_owned(), Macro { parameters: None, body: (*value).to_owned() })
        })
        .collect();
    // (branch is live, some branch of this #if has already been taken)
    let mut stack: Vec<(bool, bool)> = Vec::new();
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();
        let live = stack.iter().all(|(live, _)| *live);
        let mut words = trimmed.split_whitespace();
        let directive = words.next().unwrap_or("");
        if let Some(rest) = directive.strip_prefix('#') {
            let name = words.next().unwrap_or("");
            match rest {
                "ifdef" => {
                    let on = macros.contains_key(name);
                    stack.push((on, on));
                }
                "ifndef" => {
                    let on = !macros.contains_key(name);
                    stack.push((on, on));
                }
                "if" => {
                    // Only `#if NAME != 1` is used, and a name left out reads 0.
                    let value = macros.get(name).map_or("0", |found| found.body.trim());
                    let (operator, other) = (words.next(), words.next());
                    let on = match operator {
                        Some("!=") => Some(value) != other,
                        Some("==") => Some(value) == other,
                        _ => return Err(format!("Unsupported condition: {trimmed}")),
                    };
                    stack.push((on, on));
                }
                "else" => {
                    let (live, taken) = stack.last_mut().ok_or("#else with no #if")?;
                    *live = !*taken;
                    *taken = true;
                }
                "endif" => {
                    stack.pop().ok_or("#endif with no #if")?;
                }
                "define" if live => {
                    let rest = trimmed["#define".len()..].trim_start();
                    let bytes = rest.as_bytes();
                    let end = word_at(bytes, 0);
                    let (parameters, body) = if bytes.get(end) == Some(&b'(') {
                        let close = closing(bytes, end).ok_or("Unclosed #define")?;
                        (Some(split_arguments(&rest[end + 1..close])), &rest[close + 1..])
                    } else {
                        (None, &rest[end..])
                    };
                    let body = body.trim().to_owned();
                    macros.insert(rest[..end].to_owned(), Macro { parameters, body });
                }
                "define" => {}
                _ if live => return Err(format!("Unsupported directive: {trimmed}")),
                _ => {}
            }
        } else if live {
            out.push_str(&expand_macros(line, &macros)?);
            out.push('\n');
        }
    }
    if stack.is_empty() { Ok(out) } else { Err("Unfinished #if.".to_owned()) }
}

/// A body as one statement or brace each. Brackets hold their semicolons.
fn split_statements(body: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for character in body.chars() {
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 && matches!(character, '{' | '}') {
            if !current.trim().is_empty() {
                parts.push(current.trim().to_owned());
            }
            current.clear();
            parts.push(character.to_string());
            continue;
        }
        current.push(if character == '\n' { ' ' } else { character });
        if depth == 0 && character == ';' {
            parts.push(current.trim().to_owned());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }
    parts.retain(|part| part != ";");
    parts
}

/// `if(1 == 2)`: what a macro's selection comes to once its numbers are in.
fn literal_condition(condition: &str) -> Option<bool> {
    let (left, right) = condition.split_once("==")?;
    let number = |text: &str| text.trim().parse::<i32>().ok();
    Some(number(left)? == number(right)?)
}

/// A leading `if (...)`, `else if (...)` or `else`, and what follows it.
fn statement_head(statement: &str) -> Option<(&str, &str)> {
    let rest = statement.strip_prefix("else").map_or(statement, str::trim_start);
    if let Some(after) = rest.strip_prefix("if") {
        let after = after.trim_start();
        if after.starts_with('(') {
            let open = statement.len() - after.len();
            let close = closing(statement.as_bytes(), open)?;
            return Some((&statement[..=close], &statement[close + 1..]));
        }
    }
    statement.strip_prefix("else").filter(|after| after.is_empty() || after.starts_with(' '))
        .map(|after| ("else", after))
}

/// A constant bound to a register in its declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisterConstant {
    name: String,
    base: u32,
    array: bool,
    boolean: bool,
}

fn rewrite_registers(code: &str, constants: &[RegisterConstant]) -> String {
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut at = 0;
    while at < bytes.len() {
        if !(bytes[at].is_ascii_alphabetic() || bytes[at] == b'_') {
            let end = if bytes[at].is_ascii_digit() { word_at(bytes, at) } else { at + 1 };
            out.push_str(&code[at..end]);
            at = end;
            continue;
        }
        let end = word_at(bytes, at);
        let word = &code[at..end];
        let constant = constants.iter().find(|constant| constant.name == word);
        let Some(constant) = constant.filter(|_| at == 0 || bytes[at - 1] != b'.') else {
            out.push_str(word);
            at = end;
            continue;
        };
        let base = constant.base;
        if !constant.array {
            out.push_str(&format!("{REGISTER_ARRAY}[{base}]"));
            at = end;
            continue;
        }
        let Some(close) = closing(bytes, end) else {
            out.push_str(word);
            at = end;
            continue;
        };
        let inner = rewrite_registers(&code[end + 1..close], constants);
        let register = format!("{REGISTER_ARRAY}[int({base} + ({inner}))]");
        at = close + 1;
        if constant.boolean {
            let component = word_at(bytes, at + 1);
            out.push_str(&format!("({register}{} != 0.0)", &code[at..component]));
            at = component;
        } else {
            out.push_str(&register);
        }
    }
    out
}

/// `finalAlpha.xxxx`: a scalar's swizzle is newer than these shaders' GLSL.
fn splat_scalars(code: &str, widths: &HashMap<String, usize>) -> String {
    let mut code = code.to_owned();
    for (name, _) in widths.iter().filter(|(_, width)| **width == 1) {
        code = code.replace(&format!("{name}.xxxx"), &format!("vec4({name})"));
    }
    code
}

/// In `x.rgb = ...`, HLSL cuts a `float4` down to fit the `float3`s beside it.
/// Inside a call the parameter decides, so those are left alone.
fn truncate_to_rgb(code: &str, widths: &HashMap<String, usize>) -> String {
    let Some((target, value)) = code.split_once('=') else {
        return code.to_owned();
    };
    if !target.trim_end().ends_with(".rgb") || value.starts_with('=') {
        return code.to_owned();
    }
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut at = 0;
    let mut depth = 0usize;
    while at < bytes.len() {
        match bytes[at] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
            let end = word_at(bytes, at);
            let word = &value[at..end];
            out.push_str(word);
            let bare = depth == 0 && !matches!(bytes.get(end), Some(b'.' | b'(' | b'['));
            if bare && widths.get(word) == Some(&4) && (at == 0 || bytes[at - 1] != b'.') {
                out.push_str(".rgb");
            }
            at = end;
        } else if bytes[at].is_ascii_digit() {
            let end = word_at(bytes, at);
            out.push_str(&value[at..end]);
            at = end;
        } else {
            out.push(bytes[at] as char);
            at += 1;
        }
    }
    format!("{target}={out}")
}

fn renamed(code: &str) -> String {
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
            let end = word_at(bytes, at);
            let word = &code[at..end];
            let name = RENAMED.iter().find(|(from, _)| *from == word);
            out.push_str(name.map_or(word, |(_, to)| to));
            at = end;
        } else if bytes[at].is_ascii_digit() {
            let end = word_at(bytes, at);
            out.push_str(&code[at..end]);
            at = end;
        } else {
            out.push(bytes[at] as char);
            at += 1;
        }
    }
    out
}

/// What a semantic reads in the fragment shader.
fn semantic_value(semantic: &str) -> Option<String> {
    if semantic == "SV_POSITION" {
        return Some("gl_FragCoord".to_owned());
    }
    if let Some(index) = semantic.strip_prefix("COLOR") {
        return Some(format!("v_color{index}"));
    }
    semantic.strip_prefix("TEXCOORD").map(|index| format!("v_tc{index}"))
}

fn register_of(declaration: &str, prefix: char) -> Option<u32> {
    declaration.split("register(").nth(1)?.strip_prefix(prefix)?.split(')').next()?.parse().ok()
}

/// One statement of a function body, without any `if` before it.
fn translate_body_line(
    code: &str,
    constants: &[RegisterConstant],
    widths: &mut HashMap<String, usize>,
) -> String {
    let mut code = rewrite_registers(&renamed(code), constants);
    // A texture can be read through another's sampler state.
    for slot in 0..MAX_TEXTURES {
        let call = format!("Texture{slot}.Sample(");
        while let Some(start) = code.find(&call) {
            let arguments = start + call.len();
            let comma = code[arguments..].find(',').map_or(arguments, |at| arguments + at);
            code.replace_range(start..comma, &format!("texture(s{slot}"));
        }
    }
    let code = truncate_to_rgb(&splat_scalars(&code, widths), widths);
    convert_assignments(&translate_statement(&code, None), widths)
}

/// A body's statements as GLSL, dropping what a literal condition rules out.
fn translate_body(
    body: &str,
    constants: &[RegisterConstant],
    widths: &mut HashMap<String, usize>,
    out: &mut String,
) {
    for statement in split_statements(body) {
        let Some((head, rest)) = statement_head(&statement) else {
            out.push_str(&translate_body_line(&statement, constants, widths));
            out.push('\n');
            continue;
        };
        let rest = rest.trim();
        if let Some(condition) = head.strip_prefix("if").map(str::trim).and_then(|condition| {
            condition.strip_prefix('(')?.strip_suffix(')')
        }) && let Some(value) = literal_condition(condition)
        {
            if value && !rest.is_empty() {
                out.push_str(&translate_body_line(rest, constants, widths));
                out.push('\n');
            }
            continue;
        }
        let condition = rewrite_registers(&renamed(head), constants);
        out.push_str(&translate_statement(&condition, None));
        out.push(' ');
        if !rest.is_empty() {
            out.push_str(&translate_body_line(rest, constants, widths));
        }
        out.push('\n');
    }
}

/// Translate `file` and what it includes under `defines`, as the game's
/// compile does. `read` fetches a file by name.
pub(crate) fn translate_fx(
    file: &str,
    version_declaration: &str,
    defines: &[(&str, &str)],
    read: impl Fn(&str) -> Result<String, String>,
) -> Result<TranslatedFxShader, String> {
    let mut source = String::new();
    expand(file, &read, &mut HashSet::new(), &mut source)?;
    let source = preprocess(&source, defines)?;

    let mut samplers = Vec::new();
    let mut constants = Vec::new();
    let mut uniforms = String::new();
    let mut code = String::new();
    let mut input = None;
    let mut registers: Vec<RegisterConstant> = Vec::new();
    let mut lines = source.lines().map(str::trim).filter(|line| !line.is_empty());
    while let Some(line) = lines.next() {
        if line.starts_with("SamplerState ") || line == ";" || line == "};" {
            continue;
        }
        if let Some(base) = register_of(line, 'c') {
            let declaration = line.split(':').next().unwrap_or("");
            let mut words = declaration.split_whitespace();
            let (kind, name) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
            let (name, array) = name.split_once('[').map_or((name, false), |(name, _)| (name, true));
            let boolean = kind.starts_with("bool");
            registers.push(RegisterConstant { name: name.to_owned(), base, array, boolean });
            continue;
        }
        let texture = [
            ("Texture2D ", SamplerKind::Texture2D, "sampler2D"),
            ("TextureCube ", SamplerKind::Cube, "samplerCube"),
            ("Texture3D ", SamplerKind::Volume, "sampler3D"),
        ]
        .into_iter()
        .find_map(|(prefix, kind, name)| Some((line.strip_prefix(prefix)?, kind, name)));
        if let Some((rest, kind, name)) = texture {
            let slot = register_of(rest, 't').ok_or_else(|| format!("Unreadable texture: {line}"))?;
            samplers.push(SamplerBinding { slot, kind });
            uniforms.push_str(&format!("uniform {name} s{slot};\n"));
            continue;
        }
        if let Some(rest) = line.strip_prefix("static const int ") {
            code.push_str(&format!("const int {rest}\n"));
            continue;
        }
        if line.starts_with("cbuffer ") {
            for member in lines.by_ref().take_while(|member| !member.starts_with('}')) {
                let mut words = member.trim_end_matches(';').split_whitespace();
                let (Some(kind), Some(name)) = (words.next(), words.next()) else {
                    continue;
                };
                let register = constants.len();
                let swizzle = if matches!(kind, "float" | "half") { ".x" } else { "" };
                uniforms.push_str(&format!(
                    "uniform vec4 c{register};\n#define {name} c{register}{swizzle}\n"
                ));
                constants.push(name.to_owned());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("struct ") {
            let name = rest.split_whitespace().next().unwrap_or("");
            let mut fields = String::new();
            let mut values = Vec::new();
            for field in lines.by_ref().take_while(|field| !field.starts_with('}')) {
                let Some((declaration, semantic)) = field.trim_end_matches(';').split_once(':') else {
                    continue;
                };
                let member = declaration.split_whitespace().last().unwrap_or("");
                fields.push_str(&format!("    vec4 {member};\n"));
                values.push(
                    semantic_value(semantic.trim())
                        .ok_or_else(|| format!("Unknown semantic: {semantic}"))?,
                );
            }
            code.push_str(&format!("struct {name} {{\n{fields}}};\n"));
            input = Some(format!("{name}({})", values.join(", ")));
            continue;
        }
        // A function: its signature runs to its opening brace.
        let mut signature = line.to_owned();
        while !signature.contains(')') {
            let next = lines.next().ok_or_else(|| format!("Unfinished function: {line}"))?;
            signature.push(' ');
            signature.push_str(next);
        }
        let close = signature.rfind(')').unwrap_or(signature.len() - 1);
        signature.truncate(close + 1);
        if lines.next() != Some("{") {
            return Err(format!("Unsupported declaration: {line}"));
        }
        // Calls pass their numbers as floats, and anything compares to one.
        let signature = signature.replace("uniform const int", "float").replace("const int", "float");
        code.push_str(&translate_statement(&renamed(&signature.replace("uniform const", "const")), None));
        code.push_str(" {\n");
        let mut body = String::new();
        let mut depth = 1usize;
        for statement in lines.by_ref() {
            depth += statement.matches('{').count();
            depth -= statement.matches('}').count().min(depth);
            if depth == 0 {
                break;
            }
            body.push_str(statement);
            body.push('\n');
        }
        translate_body(&body, &registers, &mut HashMap::new(), &mut code);
        code.push_str("}\n");
    }

    let mut glsl = version_declaration.to_owned();
    for varying in ["color0", "color1", "tc0", "tc1", "tc2", "tc3", "tc4"] {
        glsl.push_str(&format!("in vec4 v_{varying};\n"));
    }
    glsl.push_str("out vec4 out_color;\n");
    if !registers.is_empty() {
        glsl.push_str(&format!("uniform vec4 {REGISTER_ARRAY}[{REGISTER_COUNT}];\n"));
    }
    glsl.push_str(&uniforms);
    glsl.push_str(HELPERS);
    glsl.push_str(&promoting_helpers());
    glsl.push_str(DOT_TRUNCATING);
    glsl.push_str(&code);
    let input = input.ok_or("The shader declares no input.")?;
    Ok(TranslatedFxShader { glsl, input, constants, samplers })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "#include \"inner.h\"\n\
        SamplerState TexSampler0 : register(s0);\n\
        Texture2D Texture0 : register(t0);\n\
        TextureCube Texture3 : register(t3);\n\
        struct PS_INPUT {\n   float4 Pos : SV_POSITION;\n   float4 D0 : COLOR0;\n   \
        float4 T0 : TEXCOORD0;\n};\n\
        cbuffer _cb0 {\n   half4 c_tint;\n   float c_alpha_ref;\n}\n\
        static const int mode_add = 1; // a comment\n\
        half4 Shade(\n\tPS_INPUT i,\n\tuniform const int nMode)\n{\n\
        half3 scale;\n\
        if(nMode==mode_add)\n{\nscale = 0.5;\n}\n\
        half4 base = Texture0.Sample(TexSampler0, i.T0.xy);\n\
        return half4(lerp(1, base.rgb, scale.x), inner(base.a));\n}\n";

    fn read(name: &str) -> Result<String, String> {
        match name {
            "main.h" => Ok(FIXTURE.to_owned()),
            "inner.h" => Ok("half inner(half value)\n{\nreturn value * 2;\n}\n".to_owned()),
            other => Err(format!("no {other}")),
        }
    }

    #[test]
    fn translation_keeps_the_function_and_numbers_the_constants() {
        let shader = translate_fx("main.h", "#version 330\n", &[], read).unwrap();
        assert_eq!(shader.constants, ["c_tint", "c_alpha_ref"]);
        assert_eq!(shader.register("c_alpha_ref"), Some(1));
        assert_eq!(shader.samplers.len(), 2);
        let glsl = shader.entry("Shade(i, mode_add)");
        assert!(glsl.contains("#define c_alpha_ref c1.x"));
        assert!(glsl.contains("const int mode_add = 1;"));
        assert!(glsl.contains("scale= to3(0.5);"), "{glsl}");
        assert!(glsl.contains("texture(s0, i.T0.xy)"), "{glsl}");
        assert!(glsl.contains("PS_INPUT i = PS_INPUT(gl_FragCoord, v_color0, v_tc0);"));
        assert!(glsl.contains("float inner(float value) {"));
        assert!(glsl.contains("float nMode"));
    }

    #[test]
    fn a_missing_include_is_an_error() {
        let read = |name: &str| match name {
            "main.h" => Ok("#include \"gone.h\"\n".to_owned()),
            other => Err(format!("no {other}")),
        };
        assert_eq!(translate_fx("main.h", "", &[], read), Err("no gone.h".to_owned()));
    }

    #[test]
    fn a_directive_it_does_not_follow_is_an_error_not_a_guess() {
        let read = |_: &str| Ok("#pragma once\n".to_owned());
        assert!(translate_fx("main.h", "", &[], read).is_err());
    }

    #[test]
    fn macros_expand_and_literal_conditions_fold() {
        let source = "#define ONE 1\n\
            #define PICK(v, n) if(n == 1) v = 2; if(n == 2) v = 3;\n\
            #define CALL(a, b) PICK(a, b)\n\
            #ifdef SETUP\nfloat4 present;\n#else\nfloat4 absent;\n#endif\n\
            #if CUBE != 1\nplain\n#else\ncube\n#endif\n\
            CALL(SETUP)\n";
        let out = preprocess(source, &[("SETUP", "x, ONE"), ("CUBE", "1")]).unwrap();
        assert!(out.contains("float4 present;") && !out.contains("absent"));
        assert!(out.contains("cube") && !out.contains("plain"));
        assert!(out.contains("if(1 == 1) x = 2; if(1 == 2) x = 3;"), "{out}");

        let mut glsl = String::new();
        translate_body("float3 x; if(1 == 1) x = 2; if(1 == 2) x = 3;", &[], &mut HashMap::new(), &mut glsl);
        assert!(glsl.contains("x= to3(2.0);"), "{glsl}");
        assert!(!glsl.contains("3.0"), "{glsl}");
    }

    #[test]
    fn a_register_constant_lands_in_the_array_and_a_bool_compares() {
        let constants = [
            RegisterConstant { name: "table".to_owned(), base: 1, array: true, boolean: false },
            RegisterConstant { name: "flags".to_owned(), base: 19, array: true, boolean: true },
            RegisterConstant { name: "one".to_owned(), base: 16, array: false, boolean: false },
        ];
        assert_eq!(rewrite_registers("table[6*(p)-3 + 0].x", &constants), "ce_registers[int(1 + (6*(p)-3 + 0))].x");
        assert_eq!(rewrite_registers("flags[1].y ? a : one.x", &constants), "(ce_registers[int(19 + (1))].y != 0.0) ? a : ce_registers[16].x");
    }

    #[test]
    fn a_float4_beside_a_float3_is_cut_down() {
        let widths = HashMap::from([("t2".to_owned(), 4), ("mask".to_owned(), 1)]);
        assert_eq!(truncate_to_rgb("r0.rgb = r1 * t2 * mask;", &widths), "r0.rgb = r1 * t2.rgb * mask;");
        assert_eq!(truncate_to_rgb("r0.rgb = t2.aaa;", &widths), "r0.rgb = t2.aaa;");
        assert_eq!(truncate_to_rgb("r0 = t2;", &widths), "r0 = t2;");
        assert_eq!(truncate_to_rgb("r0.rgb = mix(t2, 1);", &widths), "r0.rgb = mix(t2, 1);");
    }

    #[test]
    fn the_head_of_a_statement_comes_off() {
        assert_eq!(statement_head("if(a == (b)) v = 1;"), Some(("if(a == (b))", " v = 1;")));
        assert_eq!(statement_head("else if (x)"), Some(("else if (x)", "")));
        assert_eq!(statement_head("else"), Some(("else", "")));
        assert_eq!(statement_head("elsewhere = 1;"), None);
    }
}
