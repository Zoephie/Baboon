//! Typed GLSL for one entry point, and what it reaches of the program.
//!
//! HLSL's rules that GLSL lacks all surface here: a value converts to any
//! narrower or wider shape on assignment, mixed shapes truncate in arithmetic,
//! an `in` scalar splats. Matrices keep each HLSL row as a GLSL column, so
//! `m[i]` and every constructor read the same, and `mul(m, v)` becomes `v * m`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::parse::{
    Direction, Expression, Function, Program, Scalar, Statement, TextureKind, Type, Variable,
};
use super::pre::number_value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Vertex,
    Pixel,
}

/// A shader input or output the caller must wire up, by semantic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interface {
    pub(crate) semantic: String,
    pub(crate) glsl: String,
    pub(crate) ty: Type,
}

#[derive(Debug, Clone)]
pub(crate) struct Translated {
    pub(crate) source: String,
    /// Uniforms the entry point reads: name, then type.
    pub(crate) uniforms: Vec<(String, Type)>,
    pub(crate) textures: Vec<(String, TextureKind)>,
    /// What a caller must feed; the preview binds these by name.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) attributes: Vec<Interface>,
}

/// The helpers GLSL lacks. Unused ones cost nothing.
const HELPERS: &str = "float hlsl_fmod(float a, float b) { return a - b * trunc(a / b); }
vec2 hlsl_fmod(vec2 a, vec2 b) { return a - b * trunc(a / b); }
vec3 hlsl_fmod(vec3 a, vec3 b) { return a - b * trunc(a / b); }
vec4 hlsl_fmod(vec4 a, vec4 b) { return a - b * trunc(a / b); }
void hlsl_clip(float x) { if (x < 0.0) discard; }
void hlsl_clip(vec2 x) { if (any(lessThan(x, vec2(0.0)))) discard; }
void hlsl_clip(vec3 x) { if (any(lessThan(x, vec3(0.0)))) discard; }
void hlsl_clip(vec4 x) { if (any(lessThan(x, vec4(0.0)))) discard; }
";

const RESERVED_FIELDS: [&str; 22] = [
    "input", "output", "sample", "texture", "filter", "common", "partition", "active", "smooth", "flat",
    "noise", "half", "fixed", "precision", "attribute", "varying", "buffer", "shared", "patch",
    "subroutine", "image", "resource",
];

#[derive(Debug, Clone)]
struct Code {
    text: String,
    ty: Type,
    /// An array of arrays, stored flat: the base and this value's offset.
    flat: Option<(String, String)>,
}

impl Code {
    fn new(text: impl Into<String>, ty: Type) -> Self {
        Self { text: text.into(), ty, flat: None }
    }
}

fn is_numeric(ty: &Type) -> bool {
    matches!(ty, Type::Scalar(_) | Type::Vector(..) | Type::Matrix(..))
}

fn scalar_of(ty: &Type) -> Option<Scalar> {
    match ty {
        Type::Scalar(scalar) | Type::Vector(scalar, _) | Type::Matrix(scalar, ..) => Some(*scalar),
        _ => None,
    }
}

fn with_scalar(ty: &Type, scalar: Scalar) -> Type {
    match ty {
        Type::Scalar(_) => Type::Scalar(scalar),
        Type::Vector(_, size) => Type::Vector(scalar, *size),
        Type::Matrix(_, rows, columns) => Type::Matrix(scalar, *rows, *columns),
        other => other.clone(),
    }
}

fn wider(a: Scalar, b: Scalar) -> Scalar {
    let rank = |scalar| match scalar {
        Scalar::Bool => 0,
        Scalar::Int => 1,
        Scalar::Uint => 2,
        Scalar::Float => 3,
    };
    if rank(a) >= rank(b) { a } else { b }
}

/// The texture a sampler pair struct wraps. The kit's structs are named for it.
fn pair_kind(name: &str) -> Option<TextureKind> {
    if !name.contains("texture_sampler") {
        return None;
    }
    Some(if name.ends_with("_cube") {
        TextureKind::Cube
    } else if name.ends_with("_3d") {
        TextureKind::Volume
    } else if name.ends_with("_array") {
        TextureKind::FlatArray
    } else {
        TextureKind::Flat
    })
}

fn sampler_type(kind: TextureKind) -> &'static str {
    match kind {
        TextureKind::Flat => "sampler2D",
        TextureKind::FlatArray => "sampler2DArray",
        TextureKind::Volume => "sampler3D",
        TextureKind::Cube => "samplerCube",
    }
}

fn scalar_name(scalar: Scalar) -> &'static str {
    match scalar {
        Scalar::Bool => "bool",
        Scalar::Int => "int",
        Scalar::Uint => "uint",
        Scalar::Float => "float",
    }
}

fn vector_name(scalar: Scalar, size: usize) -> String {
    let prefix = match scalar {
        Scalar::Bool => "b",
        Scalar::Int => "i",
        Scalar::Uint => "u",
        Scalar::Float => "",
    };
    format!("{prefix}vec{size}")
}

/// A GLSL type, without any array suffix.
fn glsl_base(ty: &Type) -> String {
    match ty {
        Type::Void => "void".to_owned(),
        Type::Scalar(scalar) => scalar_name(*scalar).to_owned(),
        Type::Vector(scalar, size) => vector_name(*scalar, *size),
        // Each HLSL row is a column here.
        Type::Matrix(_, rows, columns) => format!("mat{rows}x{columns}"),
        Type::Array(element, _) => glsl_base(element),
        Type::Named(name) => match pair_kind(name) {
            Some(kind) => sampler_type(kind).to_owned(),
            None => format!("T_{name}"),
        },
        Type::Texture(kind) => sampler_type(*kind).to_owned(),
        Type::Sampler => "int".to_owned(),
    }
}

/// `[n]`, with nested arrays stored flat for want of arrays of arrays.
fn array_suffix(ty: &Type) -> String {
    match flat_length(ty) {
        Some(length) => format!("[{length}]"),
        None => String::new(),
    }
}

fn flat_length(ty: &Type) -> Option<usize> {
    let Type::Array(element, length) = ty else {
        return None;
    };
    Some(length.unwrap_or(0) * flat_length(element).unwrap_or(1))
}

fn declaration(ty: &Type, name: &str) -> String {
    format!("{} {name}{}", glsl_base(ty), array_suffix(ty))
}

fn component_count(ty: &Type) -> usize {
    match ty {
        Type::Vector(_, size) => *size,
        Type::Matrix(_, rows, columns) => rows * columns,
        _ => 1,
    }
}

fn float_text(text: &str) -> String {
    if text.bytes().all(|byte| byte.is_ascii_digit()) { format!("{text}.0") } else { text.to_owned() }
}

fn number_code(text: &str) -> Code {
    let hexadecimal = text.starts_with("0x") || text.starts_with("0X");
    let is_float = !hexadecimal && text.contains(['.', 'e', 'E', 'f', 'F', 'h', 'H']);
    if is_float {
        return Code::new(text.trim_end_matches(['f', 'F', 'h', 'H', 'l', 'L']), Type::Scalar(Scalar::Float));
    }
    if text.ends_with(['u', 'U']) {
        let digits = text.trim_end_matches(['u', 'U']);
        return Code::new(format!("{digits}u"), Type::Scalar(Scalar::Uint));
    }
    Code::new(text.trim_end_matches(['l', 'L']), Type::Scalar(Scalar::Int))
}

struct Emitter<'a> {
    program: &'a Program,
    stage: Stage,
    scopes: Vec<HashMap<Arc<str>, Code>>,
    functions: HashMap<usize, String>,
    function_text: Vec<String>,
    /// Functions being written, which HLSL forbids reaching again.
    open: HashSet<usize>,
    structs: Vec<Arc<str>>,
    struct_text: Vec<String>,
    globals: HashMap<usize, Code>,
    /// Declarations and static initializers, by source position.
    global_lines: Vec<(usize, String)>,
    global_inits: Vec<(usize, String)>,
    uniforms: Vec<(String, Type)>,
    textures: Vec<(String, TextureKind)>,
    /// Written before and after the statement in hand.
    before: Vec<String>,
    after: Vec<String>,
    temporaries: usize,
    returns: Type,
}

pub(crate) fn translate(program: &Program, entry: &str, stage: Stage) -> Result<Translated, String> {
    let index = program
        .functions
        .iter()
        .position(|function| &*function.name == entry)
        .ok_or_else(|| format!("no entry point {entry}"))?;
    let mut emitter = Emitter {
        program,
        stage,
        scopes: Vec::new(),
        functions: HashMap::new(),
        function_text: Vec::new(),
        open: HashSet::new(),
        structs: Vec::new(),
        struct_text: Vec::new(),
        globals: HashMap::new(),
        global_lines: Vec::new(),
        global_inits: Vec::new(),
        uniforms: Vec::new(),
        textures: Vec::new(),
        before: Vec::new(),
        after: Vec::new(),
        temporaries: 0,
        returns: Type::Void,
    };
    let name = emitter.function(index).map_err(|error| format!("{entry}: {error}"))?;
    let (attributes, main) = emitter.main_function(&program.functions[index], &name)?;
    emitter.global_lines.sort_by_key(|(at, _)| *at);
    emitter.global_inits.sort_by_key(|(at, _)| *at);
    let mut source = String::from("#version 410 core\n");
    source.push_str(if stage == Stage::Pixel { HELPERS } else { "" });
    for text in &emitter.struct_text {
        source.push_str(text);
    }
    for (_, line) in &emitter.global_lines {
        source.push_str(line);
    }
    for text in &emitter.function_text {
        source.push_str(text);
    }
    source.push_str("void hlsl_init_globals() {\n");
    for (_, line) in &emitter.global_inits {
        source.push_str(line);
    }
    source.push_str("}\n");
    source.push_str(&main);
    Ok(Translated { source, uniforms: emitter.uniforms, textures: emitter.textures, attributes })
}

/// Semantics upper-cased: the kit spells them both ways.
fn varying_name(semantic: &str) -> String {
    format!("v_{}", semantic.to_ascii_uppercase())
}

fn target_index(semantic: &str) -> Option<usize> {
    let upper = semantic.to_ascii_uppercase();
    let digits = upper.strip_prefix("SV_TARGET").or_else(|| upper.strip_prefix("COLOR"))?;
    if digits.is_empty() { Some(0) } else { digits.parse().ok() }
}

impl Emitter<'_> {
    fn resolved(&mut self, variable: &Variable) -> Result<Type, String> {
        let mut ty = variable.ty.clone();
        for dimension in variable.dimensions.iter().rev() {
            let length = dimension.as_ref().map(|dimension| self.constant(dimension)).transpose()?;
            ty = Type::Array(Box::new(ty), length.map(|length| length as usize));
        }
        Ok(ty)
    }

    /// An array length or other value known at compile time.
    fn constant(&mut self, expression: &Expression) -> Result<i64, String> {
        match expression {
            Expression::Number(text) => {
                number_value(text).ok_or_else(|| format!("{text} is not a whole number"))
            }
            Expression::Name(name) => {
                let global = self.program.globals.iter().rev().find(|global| global.name == *name);
                let init = global.and_then(|global| global.init.clone());
                let init = init.ok_or_else(|| format!("{name} is not constant"))?;
                self.constant(&init)
            }
            Expression::Cast(_, value) => self.constant(value),
            Expression::Unary(mark, value) if &**mark == "-" => Ok(-self.constant(value)?),
            Expression::Binary(mark, left, right) => {
                let (left, right) = (self.constant(left)?, self.constant(right)?);
                match &**mark {
                    "+" => Ok(left + right),
                    "-" => Ok(left - right),
                    "*" => Ok(left * right),
                    "/" => left.checked_div(right).ok_or_else(|| "divides by zero".to_owned()),
                    "<<" => Ok(left << right),
                    _ => Err(format!("{mark} in a constant")),
                }
            }
            _ => Err("not a constant".to_owned()),
        }
    }

    fn temporary(&mut self) -> String {
        self.temporaries += 1;
        format!("hlsl_t{}", self.temporaries)
    }

    fn use_struct(&mut self, name: &Arc<str>) -> Result<(), String> {
        if pair_kind(name).is_some() || self.structs.contains(name) {
            return Ok(());
        }
        let (_, fields) = self
            .program
            .structs
            .iter()
            .find(|(known, _)| known == name)
            .ok_or_else(|| format!("no struct {name}"))?;
        let mut text = format!("struct T_{name} {{\n");
        for field in fields {
            let ty = self.resolved(field)?;
            self.use_type(&ty)?;
            text.push_str(&format!("    {};\n", declaration(&ty, &field_name(&field.name))));
        }
        text.push_str("};\n");
        self.structs.push(name.clone());
        self.struct_text.push(text);
        Ok(())
    }

    fn use_type(&mut self, ty: &Type) -> Result<(), String> {
        match ty {
            Type::Named(name) => self.use_struct(name),
            Type::Array(element, _) => self.use_type(element),
            _ => Ok(()),
        }
    }

    fn struct_fields(&self, name: &str) -> Result<&[Variable], String> {
        let found = self.program.structs.iter().find(|(known, _)| &**known == name);
        found.map(|(_, fields)| fields.as_slice()).ok_or_else(|| format!("no struct {name}"))
    }

    fn lookup(&mut self, name: &Arc<str>) -> Result<Code, String> {
        for scope in self.scopes.iter().rev() {
            if let Some(code) = scope.get(name) {
                return Ok(code.clone());
            }
        }
        let index = self.program.globals.iter().rposition(|global| global.name == *name);
        let index = index.ok_or_else(|| format!("unknown name {name}"))?;
        self.global(index)
    }

    fn global(&mut self, index: usize) -> Result<Code, String> {
        if let Some(code) = self.globals.get(&index) {
            return Ok(code.clone());
        }
        let program = self.program;
        let variable = &program.globals[index];
        let ty = self.resolved(variable)?;
        self.use_type(&ty)?;
        let name = variable.name.to_string();
        if let Type::Named(pair) = &ty
            && pair_kind(pair).is_some()
        {
            // `{ sampler, texture, .. }`: the texture stands for the pair.
            let Some(Expression::List(items)) = &variable.init else {
                return Err(format!("{name} pairs nothing"));
            };
            let texture = items.get(1).ok_or_else(|| format!("{name} has no texture"))?;
            let code = self.expression(texture)?;
            let code = Code::new(code.text, ty);
            self.globals.insert(index, code.clone());
            return Ok(code);
        }
        match &ty {
            Type::Sampler => {
                let code = Code::new(String::new(), Type::Sampler);
                self.globals.insert(index, code.clone());
                return Ok(code);
            }
            Type::Texture(kind) => {
                self.global_lines.push((index, format!("uniform {} {name};\n", sampler_type(*kind))));
                self.textures.push((name.clone(), *kind));
                let code = Code::new(name, ty);
                self.globals.insert(index, code.clone());
                return Ok(code);
            }
            _ => {}
        }
        let Some(init) = &variable.init else {
            self.global_lines.push((index, format!("uniform {};\n", declaration(&ty, &name))));
            self.uniforms.push((name.clone(), ty.clone()));
            let code = Code::new(name, ty);
            self.globals.insert(index, code.clone());
            return Ok(code);
        };
        let glsl = format!("g_{name}");
        let code = Code::new(glsl.clone(), ty.clone());
        self.globals.insert(index, code.clone());
        self.global_lines.push((index, format!("{};\n", declaration(&ty, &glsl))));
        // Initializers run in order at `main`, since a uniform can feed one.
        let saved = (std::mem::take(&mut self.before), std::mem::take(&mut self.after));
        let scopes = std::mem::take(&mut self.scopes);
        let value = self.convert_init(init, &ty);
        self.scopes = scopes;
        let value = value?;
        let mut line = String::new();
        for hoisted in std::mem::take(&mut self.before) {
            line.push_str(&hoisted);
        }
        line.push_str(&format!("    {glsl} = {};\n", value.text));
        (self.before, self.after) = saved;
        self.global_inits.push((index, line));
        Ok(code)
    }

    fn zero(&mut self, ty: &Type) -> Result<String, String> {
        Ok(match ty {
            Type::Scalar(Scalar::Bool) => "false".to_owned(),
            Type::Scalar(Scalar::Float) => "0.0".to_owned(),
            Type::Scalar(Scalar::Uint) => "0u".to_owned(),
            Type::Scalar(Scalar::Int) => "0".to_owned(),
            Type::Vector(..) => format!("{}(0)", glsl_base(ty)),
            Type::Matrix(..) => format!("{}(0.0)", glsl_base(ty)),
            Type::Array(element, _) => {
                let length = flat_length(ty).unwrap_or(0);
                let mut leaf: &Type = element;
                while let Type::Array(inner, _) = leaf {
                    leaf = inner;
                }
                let zero = self.zero(leaf)?;
                format!("{}[{length}]({})", glsl_base(ty), vec![zero; length].join(", "))
            }
            Type::Named(name) => {
                self.use_struct(name)?;
                let program = self.program;
                let (_, fields) = program.structs.iter().find(|(known, _)| known == name).ok_or("no struct")?;
                let mut parts = Vec::new();
                for field in fields {
                    let ty = self.resolved(field)?;
                    parts.push(self.zero(&ty)?);
                }
                format!("T_{name}({})", parts.join(", "))
            }
            other => return Err(format!("no zero for {other:?}")),
        })
    }

    /// Scalars of a nested initializer, flattened.
    fn flatten(&mut self, items: &[Expression], output: &mut Vec<Code>) -> Result<(), String> {
        for item in items {
            match item {
                Expression::List(inner) => self.flatten(inner, output)?,
                other => output.push(self.expression(other)?),
            }
        }
        Ok(())
    }

    fn convert_init(&mut self, init: &Expression, ty: &Type) -> Result<Code, String> {
        let Expression::List(items) = init else {
            let value = self.expression(init)?;
            return self.convert(value, ty);
        };
        match ty {
            Type::Array(element, length) => {
                let length = length.unwrap_or(items.len());
                let mut parts = Vec::with_capacity(length);
                if items.len() == length {
                    for item in items {
                        parts.push(self.convert_init(item, element)?.text);
                    }
                } else {
                    let mut flat = Vec::new();
                    self.flatten(items, &mut flat)?;
                    let each = component_count(element).max(1);
                    for chunk in flat.chunks(each) {
                        let inner: Vec<_> = chunk.iter().map(|code| code.text.clone()).collect();
                        parts.push(format!("{}({})", glsl_base(element), inner.join(", ")));
                    }
                }
                Ok(Code::new(
                    format!("{}[{}]({})", glsl_base(element), parts.len(), parts.join(", ")),
                    Type::Array(element.clone(), Some(parts.len())),
                ))
            }
            Type::Named(name) => {
                self.use_struct(name)?;
                let program = self.program;
                let (_, fields) = program.structs.iter().find(|(known, _)| known == name).ok_or("no struct")?;
                let mut parts = Vec::new();
                for (field, item) in fields.iter().zip(items) {
                    let ty = self.resolved(field)?;
                    parts.push(self.convert_init(item, &ty)?.text);
                }
                Ok(Code::new(format!("T_{name}({})", parts.join(", ")), ty.clone()))
            }
            Type::Vector(..) | Type::Matrix(..) => {
                let mut flat = Vec::new();
                self.flatten(items, &mut flat)?;
                let parts: Vec<_> = flat.iter().map(|code| code.text.clone()).collect();
                Ok(Code::new(format!("{}({})", glsl_base(ty), parts.join(", ")), ty.clone()))
            }
            Type::Scalar(_) => {
                let first = items.first().ok_or("an empty initializer")?;
                let value = self.expression(first)?;
                self.convert(value, ty)
            }
            other => Err(format!("cannot initialize {other:?} from a list")),
        }
    }

    fn convert(&mut self, value: Code, to: &Type) -> Result<Code, String> {
        if value.ty == *to {
            return Ok(value);
        }
        let done = |text: String| Ok(Code::new(text, to.clone()));
        match (&value.ty, to) {
            (Type::Scalar(from), Type::Scalar(scalar)) => match (from, scalar) {
                (Scalar::Int, Scalar::Float) => done(if value.text.parse::<i64>().is_ok() {
                    float_text(&value.text)
                } else {
                    format!("float({})", value.text)
                }),
                _ => done(format!("{}({})", scalar_name(*scalar), value.text)),
            },
            (Type::Scalar(_), Type::Vector(scalar, _)) => {
                let single = self.convert(value, &Type::Scalar(*scalar))?;
                done(format!("{}({})", glsl_base(to), single.text))
            }
            (Type::Scalar(_), Type::Matrix(scalar, rows, columns)) => {
                let single = self.convert(value, &Type::Scalar(*scalar))?;
                let parts = vec![single.text; rows * columns];
                done(format!("{}({})", glsl_base(to), parts.join(", ")))
            }
            (Type::Vector(..), Type::Scalar(scalar)) => {
                let first = format!("{}.x", value.text);
                self.convert(Code::new(first, with_scalar(&Type::Scalar(scalar_of(&value.ty).unwrap()), scalar_of(&value.ty).unwrap())), &Type::Scalar(*scalar))
            }
            (Type::Vector(from, size), Type::Vector(scalar, wanted)) => {
                let shape = if wanted < size {
                    format!("{}.{}", value.text, &"xyzw"[..*wanted])
                } else if wanted == size {
                    value.text.clone()
                } else {
                    let extra = vec!["0"; wanted - size].join(", ");
                    return done(format!("{}({}, {extra})", glsl_base(to), value.text));
                };
                if from == scalar {
                    return done(shape);
                }
                done(format!("{}({shape})", glsl_base(to)))
            }
            (Type::Matrix(..), Type::Matrix(..)) => done(format!("{}({})", glsl_base(to), value.text)),
            (Type::Matrix(_, 1, _), Type::Vector(..)) => {
                let row = Code::new(format!("{}[0]", value.text), Type::Vector(Scalar::Float, 4));
                self.convert(row, to)
            }
            (Type::Scalar(_), Type::Named(_) | Type::Array(..)) => {
                let zero = self.zero(to)?;
                done(zero)
            }
            (Type::Array(from, Some(_)), Type::Array(element, None)) if from == element => {
                Ok(Code::new(value.text, value.ty))
            }
            (Type::Named(from), Type::Texture(kind)) if pair_kind(from) == Some(*kind) => {
                done(value.text)
            }
            (Type::Texture(kind), Type::Named(name)) if pair_kind(name) == Some(*kind) => {
                done(value.text)
            }
            (from, _) => Err(format!("cannot convert {from:?} to {to:?}: {}", value.text)),
        }
    }

    /// How far a value is from a parameter's type; lower binds first.
    fn conversion_cost(&self, from: &Type, to: &Type) -> Option<u32> {
        if from == to {
            return Some(0);
        }
        match (from, to) {
            (Type::Array(a, _), Type::Array(b, None)) if a == b => Some(0),
            (Type::Array(a, x), Type::Array(b, y)) if a == b && x == y => Some(0),
            (Type::Named(name), Type::Texture(kind)) | (Type::Texture(kind), Type::Named(name)) => {
                (pair_kind(name) == Some(*kind)).then_some(1)
            }
            (Type::Named(a), Type::Named(b)) => {
                (pair_kind(a).is_some() && pair_kind(a) == pair_kind(b)).then_some(1)
            }
            (Type::Scalar(a), Type::Scalar(b)) => Some(if a == b { 0 } else { 1 }),
            (Type::Vector(a, x), Type::Vector(b, y)) if x == y => Some(if a == b { 0 } else { 1 }),
            (Type::Vector(_, x), Type::Vector(_, y)) if y < x => Some(3),
            (Type::Vector(..), Type::Scalar(_)) => Some(4),
            (Type::Scalar(_), Type::Vector(..)) => Some(2),
            (Type::Scalar(_), Type::Matrix(..)) => Some(5),
            (Type::Matrix(_, a, b), Type::Matrix(_, x, y)) if x <= a && y <= b => Some(2),
            _ => None,
        }
    }

    /// The overload arguments of these types call, by the lowest total cost.
    fn overload(&mut self, name: &str, arguments: &[Type]) -> Result<Option<usize>, String> {
        let mut best: Option<(u32, usize)> = None;
        let mut named = false;
        for (index, function) in self.program.functions.iter().enumerate() {
            if &*function.name != name {
                continue;
            }
            named = true;
            let required =
                function.parameters.iter().filter(|parameter| parameter.variable.init.is_none()).count();
            if arguments.len() < required || arguments.len() > function.parameters.len() {
                continue;
            }
            let mut total = 0;
            let mut fits = true;
            for (argument, parameter) in arguments.iter().zip(&function.parameters) {
                let ty = self.resolved(&parameter.variable)?;
                match self.conversion_cost(argument, &ty) {
                    Some(cost) => total += cost,
                    None => fits = false,
                }
            }
            if fits && best.is_none_or(|(cost, _)| total < cost) {
                best = Some((total, index));
            }
        }
        if named && best.is_none() {
            return Err(format!("no overload of {name} takes {arguments:?}"));
        }
        Ok(best.map(|(_, index)| index))
    }

    fn function(&mut self, index: usize) -> Result<String, String> {
        if let Some(name) = self.functions.get(&index) {
            return Ok(name.clone());
        }
        if !self.open.insert(index) {
            return Err("recursion".to_owned());
        }
        let program = self.program;
        let function: &Function = &program.functions[index];
        let name = format!("f{index}_{}", function.name);
        // A function is written whole between another's statements.
        let saved_scopes = std::mem::take(&mut self.scopes);
        let saved_before = std::mem::take(&mut self.before);
        let saved_after = std::mem::take(&mut self.after);
        let saved_returns = std::mem::replace(&mut self.returns, function.returns.clone());
        let written = self.function_text_of(function, &name);
        self.scopes = saved_scopes;
        self.before = saved_before;
        self.after = saved_after;
        self.returns = saved_returns;
        self.open.remove(&index);
        let text = written.map_err(|error| format!("{}: {error}", function.name))?;
        self.function_text.push(text);
        self.functions.insert(index, name.clone());
        Ok(name)
    }

    fn function_text_of(&mut self, function: &Function, name: &str) -> Result<String, String> {
        self.use_type(&function.returns)?;
        let mut scope = HashMap::new();
        let mut parameters = Vec::new();
        for parameter in &function.parameters {
            let ty = self.resolved(&parameter.variable)?;
            self.use_type(&ty)?;
            if ty == Type::Sampler {
                scope.insert(parameter.variable.name.clone(), Code::new("", Type::Sampler));
                continue;
            }
            let glsl = format!("l_{}", parameter.variable.name);
            let direction = match parameter.direction {
                Direction::In => "in",
                Direction::Out => "out",
                Direction::InOut => "inout",
            };
            parameters.push(format!("{direction} {}", declaration(&ty, &glsl)));
            scope.insert(parameter.variable.name.clone(), Code::new(glsl, ty));
        }
        self.scopes.push(scope);
        let mut text =
            format!("{} {name}({}) {{\n", glsl_base(&function.returns), parameters.join(", "));
        self.statements(&function.body, &mut text)?;
        text.push_str("}\n");
        self.scopes.pop();
        Ok(text)
    }

    fn statements(&mut self, statements: &[Statement], text: &mut String) -> Result<(), String> {
        for statement in statements {
            self.statement(statement, text)?;
        }
        Ok(())
    }

    fn flushed(&mut self, line: String, text: &mut String) {
        for hoisted in std::mem::take(&mut self.before) {
            text.push_str(&hoisted);
        }
        text.push_str(&line);
        for hoisted in std::mem::take(&mut self.after) {
            text.push_str(&hoisted);
        }
    }

    fn scoped_statement(&mut self, statement: &Statement, text: &mut String) -> Result<(), String> {
        self.scopes.push(HashMap::new());
        text.push_str("{\n");
        let result = match statement {
            Statement::Block(statements) => self.statements(statements, text),
            other => self.statement(other, text),
        };
        text.push_str("}\n");
        self.scopes.pop();
        result
    }

    fn statement(&mut self, statement: &Statement, text: &mut String) -> Result<(), String> {
        match statement {
            Statement::Declare(variables) => {
                for variable in variables {
                    let mut ty = self.resolved(variable)?;
                    self.use_type(&ty)?;
                    let glsl = format!("l_{}", variable.name);
                    if let Type::Named(pair) = &ty
                        && pair_kind(pair).is_some()
                    {
                        let init = variable.init.as_ref().ok_or("a pair needs a value")?;
                        let init = match init {
                            Expression::List(items) => items.get(1).ok_or("a pair has no texture")?,
                            other => other,
                        };
                        let value = self.expression(init)?;
                        self.declare(&variable.name, Code::new(value.text, ty));
                        continue;
                    }
                    let value = match &variable.init {
                        Some(init) => Some(self.convert_init(init, &ty)?),
                        None => None,
                    };
                    if let (Type::Array(_, None), Some(value)) = (&ty, &value) {
                        ty = value.ty.clone();
                    }
                    let line = match value {
                        Some(value) => format!("{} = {};\n", declaration(&ty, &glsl), value.text),
                        None => format!("{};\n", declaration(&ty, &glsl)),
                    };
                    self.flushed(line, text);
                    self.declare(&variable.name, Code::new(glsl, ty));
                }
            }
            Statement::Expression(expression) => {
                let value = self.expression(expression)?;
                if !value.text.is_empty() {
                    self.flushed(format!("{};\n", value.text), text);
                }
            }
            Statement::Block(statements) => self.scoped_statement(&Statement::Block(statements.clone()), text)?,
            Statement::If(condition, then, otherwise) => {
                let condition = self.truth(condition)?;
                self.flushed(format!("if ({condition})\n"), text);
                self.scoped_statement(then, text)?;
                if let Some(otherwise) = otherwise {
                    text.push_str("else\n");
                    self.scoped_statement(otherwise, text)?;
                }
            }
            Statement::For(init, condition, step, body) => {
                self.scopes.push(HashMap::new());
                text.push_str("{\n");
                if let Some(init) = init {
                    self.statement(init, text)?;
                }
                let condition = match condition {
                    Some(condition) => self.truth(condition)?,
                    None => String::new(),
                };
                let step = match step {
                    Some(step) => self.expression(step)?.text,
                    None => String::new(),
                };
                self.flushed(format!("for (; {condition}; {step})\n"), text);
                self.scoped_statement(body, text)?;
                text.push_str("}\n");
                self.scopes.pop();
            }
            Statement::While(condition, body) => {
                let condition = self.truth(condition)?;
                self.flushed(format!("while ({condition})\n"), text);
                self.scoped_statement(body, text)?;
            }
            Statement::DoWhile(body, condition) => {
                text.push_str("do\n");
                self.scoped_statement(body, text)?;
                let condition = self.truth(condition)?;
                self.flushed(format!("while ({condition});\n"), text);
            }
            Statement::Switch(value, cases) => {
                let value = self.expression(value)?;
                let value = self.convert(value, &Type::Scalar(Scalar::Int))?;
                self.flushed(format!("switch ({}) {{\n", value.text), text);
                self.scopes.push(HashMap::new());
                for (label, statements) in cases {
                    match label {
                        Some(label) => {
                            let label = self.expression(label)?;
                            text.push_str(&format!("case {}:\n", label.text));
                        }
                        None => text.push_str("default:\n"),
                    }
                    self.statements(statements, text)?;
                }
                self.scopes.pop();
                text.push_str("}\n");
            }
            Statement::Return(value) => {
                let line = match value {
                    Some(value) => {
                        let returns = self.returns.clone();
                        let value = self.convert_init(value, &returns)?;
                        // A hoisted write has to land before leaving.
                        if self.after.is_empty() {
                            format!("return {};\n", value.text)
                        } else {
                            let name = self.temporary();
                            let hold = format!("{} = {};\n", declaration(&returns, &name), value.text);
                            self.flushed(hold, text);
                            format!("return {name};\n")
                        }
                    }
                    None => "return;\n".to_owned(),
                };
                self.flushed(line, text);
            }
            Statement::Discard => text.push_str("discard;\n"),
            Statement::Break => text.push_str("break;\n"),
            Statement::Continue => text.push_str("continue;\n"),
        }
        Ok(())
    }

    fn declare(&mut self, name: &Arc<str>, code: Code) {
        self.scopes.last_mut().expect("statements run in a scope").insert(name.clone(), code);
    }

    fn truth(&mut self, expression: &Expression) -> Result<String, String> {
        let value = self.expression(expression)?;
        Ok(self.truth_of(value)?.text)
    }

    fn truth_of(&mut self, value: Code) -> Result<Code, String> {
        match &value.ty {
            Type::Vector(scalar, size) if *scalar != Scalar::Bool => {
                let zero = self.zero(&value.ty)?;
                let text = format!("notEqual({}, {zero})", value.text);
                Ok(Code::new(text, Type::Vector(Scalar::Bool, *size)))
            }
            Type::Vector(..) => Ok(value),
            _ => self.convert(value, &Type::Scalar(Scalar::Bool)),
        }
    }

    /// Shapes HLSL's rules give two operands, and the numeric type they share.
    fn common(&mut self, left: Code, right: Code) -> Result<(Code, Code, Type), String> {
        let scalar = wider(
            scalar_of(&left.ty).ok_or_else(|| format!("{:?} is not numeric", left.ty))?,
            scalar_of(&right.ty).ok_or_else(|| format!("{:?} is not numeric", right.ty))?,
        );
        let shape = match (&left.ty, &right.ty) {
            (Type::Vector(_, a), Type::Vector(_, b)) => Type::Vector(scalar, *a.min(b)),
            (Type::Matrix(_, a, b), Type::Matrix(_, c, d)) => Type::Matrix(scalar, *a.min(c), *b.min(d)),
            (Type::Scalar(_), other) | (other, Type::Scalar(_)) => with_scalar(other, scalar),
            (Type::Matrix(..), other) | (other, Type::Matrix(..)) => with_scalar(other, scalar),
            _ => Type::Scalar(scalar),
        };
        let keep = |code: &Code, this: &mut Self, shape: &Type| -> Result<Code, String> {
            // GLSL takes a lone scalar beside a vector, so only its kind moves.
            if matches!(code.ty, Type::Scalar(_)) {
                this.convert(code.clone(), &Type::Scalar(scalar))
            } else {
                this.convert(code.clone(), shape)
            }
        };
        let left = keep(&left, self, &shape)?;
        let right = keep(&right, self, &shape)?;
        Ok((left, right, shape))
    }

    fn binary(&mut self, mark: &str, left: Code, right: Code) -> Result<Code, String> {
        if matches!(mark, "&&" | "||") {
            let (left, right) = (self.truth_of(left)?, self.truth_of(right)?);
            if let Type::Vector(_, size) = left.ty {
                let right = self.convert(right, &Type::Vector(Scalar::Bool, size))?;
                let glsl = if mark == "&&" { "min" } else { "max" };
                let text = format!(
                    "bvec{size}({glsl}(ivec{size}({}), ivec{size}({})))",
                    left.text, right.text
                );
                return Ok(Code::new(text, Type::Vector(Scalar::Bool, size)));
            }
            return Ok(Code::new(
                format!("({} {mark} {})", left.text, right.text),
                Type::Scalar(Scalar::Bool),
            ));
        }
        let (mut left, mut right, shape) = self.common(left, right)?;
        if matches!(mark, "==" | "!=" | "<" | ">" | "<=" | ">=") {
            if let Type::Vector(_, size) = shape {
                left = self.convert(left, &shape)?;
                right = self.convert(right, &shape)?;
                let glsl = match mark {
                    "==" => "equal",
                    "!=" => "notEqual",
                    "<" => "lessThan",
                    ">" => "greaterThan",
                    "<=" => "lessThanEqual",
                    _ => "greaterThanEqual",
                };
                return Ok(Code::new(
                    format!("{glsl}({}, {})", left.text, right.text),
                    Type::Vector(Scalar::Bool, size),
                ));
            }
            return Ok(Code::new(
                format!("({} {mark} {})", left.text, right.text),
                Type::Scalar(Scalar::Bool),
            ));
        }
        if scalar_of(&shape) == Some(Scalar::Bool) {
            let ints = with_scalar(&shape, Scalar::Int);
            left = self.convert(left, &ints)?;
            right = self.convert(right, &ints)?;
            return self.binary(mark, left, right);
        }
        if mark == "%" && scalar_of(&shape) == Some(Scalar::Float) {
            left = self.convert(left, &shape)?;
            right = self.convert(right, &shape)?;
            return Ok(Code::new(format!("hlsl_fmod({}, {})", left.text, right.text), shape));
        }
        if matches!(shape, Type::Matrix(..)) && mark == "*" {
            left = self.convert(left, &shape)?;
            right = self.convert(right, &shape)?;
            return Ok(Code::new(format!("matrixCompMult({}, {})", left.text, right.text), shape));
        }
        Ok(Code::new(format!("({} {mark} {})", left.text, right.text), shape))
    }

    fn expression(&mut self, expression: &Expression) -> Result<Code, String> {
        match expression {
            Expression::Number(text) => Ok(number_code(text)),
            Expression::Name(name) => match &**name {
                "true" | "false" => Ok(Code::new(name.to_string(), Type::Scalar(Scalar::Bool))),
                _ => self.lookup(name),
            },
            Expression::Unary(mark, value) => {
                let value = self.expression(value)?;
                match &**mark {
                    "!" => {
                        let value = self.truth_of(value)?;
                        let text = match value.ty {
                            Type::Vector(..) => format!("not({})", value.text),
                            _ => format!("(!{})", value.text),
                        };
                        Ok(Code::new(text, value.ty))
                    }
                    _ => Ok(Code::new(format!("({mark}{})", value.text), value.ty)),
                }
            }
            Expression::Postfix(mark, value) => {
                let value = self.expression(value)?;
                Ok(Code::new(format!("({}{mark})", value.text), value.ty))
            }
            Expression::Binary(mark, left, right) => {
                let (left, right) = (self.expression(left)?, self.expression(right)?);
                self.binary(mark, left, right)
            }
            Expression::Assign(mark, target, value) => {
                let target = self.expression(target)?;
                let mut value = self.convert_init(value, &target.ty)?;
                if &**mark != "=" {
                    let operation = mark.trim_end_matches('=');
                    value = self.binary(operation, target.clone(), value)?;
                    value = self.convert(value, &target.ty)?;
                }
                Ok(Code::new(format!("({} = {})", target.text, value.text), target.ty))
            }
            Expression::Ternary(condition, first, second) => {
                let condition = self.expression(condition)?;
                let condition = self.truth_of(condition)?;
                let (first, second) = (self.expression(first)?, self.expression(second)?);
                let (first, second, shape) = if is_numeric(&first.ty) && is_numeric(&second.ty) {
                    let (first, second, shape) = self.common(first, second)?;
                    (self.convert(first, &shape)?, self.convert(second, &shape)?, shape)
                } else {
                    let shape = first.ty.clone();
                    (first, second, shape)
                };
                if let Type::Vector(_, size) = condition.ty {
                    let wide = match &shape {
                        Type::Scalar(scalar) => Type::Vector(*scalar, size),
                        other => other.clone(),
                    };
                    let first = self.convert(first, &wide)?;
                    let second = self.convert(second, &wide)?;
                    let text = format!("mix({}, {}, {})", second.text, first.text, condition.text);
                    return Ok(Code::new(text, wide));
                }
                Ok(Code::new(format!("({} ? {} : {})", condition.text, first.text, second.text), shape))
            }
            Expression::Comma(first, second) => {
                let (first, second) = (self.expression(first)?, self.expression(second)?);
                Ok(Code::new(format!("({}, {})", first.text, second.text), second.ty))
            }
            Expression::Cast(ty, value) => {
                self.use_type(ty)?;
                let value = self.expression(value)?;
                self.convert(value, ty)
            }
            Expression::Construct(ty, arguments) => self.construct(ty, arguments),
            Expression::Call(name, arguments) => self.call(name, arguments),
            Expression::Method(object, name, arguments) => {
                let object = self.expression(object)?;
                self.method(object, name, arguments)
            }
            Expression::Member(object, name) => {
                let object = self.expression(object)?;
                self.member(object, name)
            }
            Expression::Index(object, index) => {
                let object = self.expression(object)?;
                let index = self.expression(index)?;
                let index = self.convert(index, &Type::Scalar(Scalar::Int))?;
                self.index(object, index)
            }
            Expression::List(_) => Err("a list outside an initializer".to_owned()),
        }
    }

    fn construct(&mut self, ty: &Type, arguments: &[Expression]) -> Result<Code, String> {
        let mut parts = Vec::with_capacity(arguments.len());
        for argument in arguments {
            parts.push(self.expression(argument)?);
        }
        if let [only] = parts.as_slice()
            && matches!(only.ty, Type::Scalar(_))
            && !matches!(ty, Type::Matrix(..))
        {
            return self.convert(only.clone(), ty);
        }
        // GLSL rejects a mix only in matrices, and a bool among floats.
        let kind = scalar_of(ty).ok_or("a constructor of a non-numeric")?;
        let mut texts = Vec::with_capacity(parts.len());
        for part in parts {
            let wanted = with_scalar(&part.ty, kind);
            texts.push(self.convert(part, &wanted)?.text);
        }
        Ok(Code::new(format!("{}({})", glsl_base(ty), texts.join(", ")), ty.clone()))
    }

    fn member(&mut self, object: Code, name: &str) -> Result<Code, String> {
        if let Type::Named(pair) = &object.ty
            && let Some(kind) = pair_kind(pair)
        {
            return match name {
                "t" => Ok(Code::new(object.text, Type::Texture(kind))),
                "s" => Ok(Code::new("", Type::Sampler)),
                _ => Ok(Code::new("false", Type::Scalar(Scalar::Bool))),
            };
        }
        if let Type::Named(structure) = &object.ty {
            let structure = structure.clone();
            let field = self
                .struct_fields(&structure)?
                .iter()
                .find(|field| &*field.name == name)
                .cloned()
                .ok_or_else(|| format!("{structure} has no {name}"))?;
            let ty = self.resolved(&field)?;
            return Ok(Code::new(format!("{}.{}", object.text, field_name(name)), ty));
        }
        if let Type::Matrix(scalar, ..) = object.ty
            && let Some(rest) = name.strip_prefix('_')
        {
            let digits: Vec<usize> = rest
                .trim_start_matches('m')
                .bytes()
                .filter(u8::is_ascii_digit)
                .map(|digit| (digit - b'0') as usize)
                .collect();
            let base = usize::from(!rest.starts_with('m'));
            let parts: Vec<_> = digits
                .chunks(2)
                .map(|pair| format!("{}[{}][{}]", object.text, pair[0] - base, pair[1] - base))
                .collect();
            return Ok(match parts.len() {
                1 => Code::new(parts[0].clone(), Type::Scalar(scalar)),
                size => Code::new(format!("{}({})", vector_name(scalar, size), parts.join(", ")), Type::Vector(scalar, size)),
            });
        }
        let scalar = scalar_of(&object.ty).ok_or_else(|| format!("{:?} has no .{name}", object.ty))?;
        let size = name.len();
        if !(1..=4).contains(&size) || !name.bytes().all(|byte| b"xyzwrgba".contains(&byte)) {
            return Err(format!("bad swizzle .{name}"));
        }
        if matches!(object.ty, Type::Scalar(_)) {
            let ty = if size == 1 { Type::Scalar(scalar) } else { Type::Vector(scalar, size) };
            return self.convert(object, &ty);
        }
        let ty = if size == 1 { Type::Scalar(scalar) } else { Type::Vector(scalar, size) };
        Ok(Code::new(format!("{}.{name}", object.text), ty))
    }

    fn index(&mut self, object: Code, index: Code) -> Result<Code, String> {
        if let Some((base, offset)) = &object.flat {
            let at = format!("{offset} + {}", index.text);
            let Type::Array(element, _) = &object.ty else {
                return Err("flat value is not an array".to_owned());
            };
            return Ok(Code::new(format!("{base}[{at}]"), (**element).clone()));
        }
        match &object.ty {
            Type::Array(element, _) => {
                if let Type::Array(inner, Some(length)) = &**element {
                    let offset = format!("({}) * {length}", index.text);
                    let items: Vec<_> = (0..*length)
                        .map(|item| format!("{}[{offset} + {item}]", object.text))
                        .collect();
                    let text = format!("{}[{length}]({})", glsl_base(inner), items.join(", "));
                    let mut code = Code::new(text, (**element).clone());
                    code.flat = Some((object.text.clone(), offset));
                    return Ok(code);
                }
                Ok(Code::new(format!("{}[{}]", object.text, index.text), (**element).clone()))
            }
            Type::Vector(scalar, _) => {
                Ok(Code::new(format!("{}[{}]", object.text, index.text), Type::Scalar(*scalar)))
            }
            Type::Matrix(scalar, _, columns) => {
                Ok(Code::new(format!("{}[{}]", object.text, index.text), Type::Vector(*scalar, *columns)))
            }
            other => Err(format!("cannot index {other:?}")),
        }
    }

    fn call(&mut self, name: &str, arguments: &[Expression]) -> Result<Code, String> {
        let mut values = Vec::with_capacity(arguments.len());
        for argument in arguments {
            values.push(self.expression(argument)?);
        }
        let types: Vec<_> = values.iter().map(|value| value.ty.clone()).collect();
        let Some(index) = self.overload(name, &types)? else {
            return self.intrinsic(name, values);
        };
        let symbol = self.function(index)?;
        let program = self.program;
        let function = &program.functions[index];
        let mut passed = Vec::with_capacity(function.parameters.len());
        for (at, parameter) in function.parameters.iter().enumerate() {
            let ty = self.resolved(&parameter.variable)?;
            let Some(value) = values.get(at).cloned() else {
                let init = parameter.variable.init.as_ref().ok_or("missing argument")?;
                let value = self.convert_init(init, &ty)?;
                passed.push(value.text);
                continue;
            };
            if ty == Type::Sampler {
                continue;
            }
            if parameter.direction == Direction::In {
                passed.push(self.convert(value, &ty)?.text);
            } else if value.ty == ty {
                passed.push(value.text);
            } else {
                // `out` through a different shape needs somewhere to land.
                let temporary = self.temporary();
                let mut line = format!("{};\n", declaration(&ty, &temporary));
                if parameter.direction == Direction::InOut {
                    let held = self.convert(value.clone(), &ty)?;
                    line.push_str(&format!("{temporary} = {};\n", held.text));
                }
                self.before.push(line);
                let back = self.convert(Code::new(temporary.clone(), ty.clone()), &value.ty)?;
                self.after.push(format!("{} = {};\n", value.text, back.text));
                passed.push(temporary);
            }
        }
        Ok(Code::new(format!("{symbol}({})", passed.join(", ")), function.returns.clone()))
    }

    /// Every argument in the widest shape all share, as floats unless given.
    fn unified(&mut self, values: Vec<Code>, scalar: Option<Scalar>) -> Result<(Vec<String>, Type), String> {
        let mut size = None;
        let mut kind = scalar;
        for value in &values {
            if let Type::Vector(_, length) = value.ty {
                size = Some(size.map_or(length, |size: usize| size.min(length)));
            }
            let element = scalar_of(&value.ty).ok_or_else(|| format!("{:?} is not numeric", value.ty))?;
            if scalar.is_none() {
                kind = Some(kind.map_or(element, |kind| wider(kind, element)));
            }
        }
        let kind = kind.expect("an intrinsic takes arguments");
        let kind = if kind == Scalar::Bool { Scalar::Int } else { kind };
        let ty = match (size, values.iter().find(|value| matches!(value.ty, Type::Matrix(..)))) {
            (_, Some(matrix)) => with_scalar(&matrix.ty, kind),
            (Some(size), None) => Type::Vector(kind, size),
            (None, None) => Type::Scalar(kind),
        };
        let mut texts = Vec::with_capacity(values.len());
        for value in values {
            texts.push(self.convert(value, &ty)?.text);
        }
        Ok((texts, ty))
    }

    fn intrinsic(&mut self, name: &str, mut values: Vec<Code>) -> Result<Code, String> {
        let float = Some(Scalar::Float);
        let single = Type::Scalar(Scalar::Float);
        let bool_type = Type::Scalar(Scalar::Bool);
        let renamed = match name {
            "frac" => Some("fract"),
            "ddx" | "ddx_fine" | "ddx_coarse" => Some("dFdx"),
            "ddy" | "ddy_fine" | "ddy_coarse" => Some("dFdy"),
            "abs" | "sign" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh"
            | "exp" | "exp2" | "log" | "log2" | "sqrt" | "floor" | "ceil" | "round" | "trunc" | "fwidth"
            | "radians" | "degrees" | "normalize" => Some(name),
            _ => None,
        };
        if let Some(glsl) = renamed {
            let keeps_kind = matches!(name, "abs" | "sign");
            let (texts, ty) = self.unified(values, if keeps_kind { None } else { float })?;
            return Ok(Code::new(format!("{glsl}({})", texts[0]), ty));
        }
        match name {
            "saturate" => {
                let (texts, ty) = self.unified(values, float)?;
                Ok(Code::new(format!("clamp({}, 0.0, 1.0)", texts[0]), ty))
            }
            "lerp" => {
                let t = values.pop().ok_or("lerp takes three")?;
                let (texts, ty) = self.unified(values, float)?;
                let t = match t.ty {
                    Type::Scalar(_) => self.convert(t, &single)?,
                    _ => self.convert(t, &ty)?,
                };
                Ok(Code::new(format!("mix({}, {}, {})", texts[0], texts[1], t.text), ty))
            }
            "min" | "max" | "clamp" | "step" | "smoothstep" | "pow" | "atan2" | "fmod" | "reflect"
            | "ldexp" | "distance" => {
                let integers = matches!(name, "min" | "max" | "clamp");
                let (texts, ty) = self.unified(values, if integers { None } else { float })?;
                let glsl = match name {
                    "atan2" => "atan",
                    "fmod" => "hlsl_fmod",
                    other => other,
                };
                let ty = if name == "distance" { single } else { ty };
                Ok(Code::new(format!("{glsl}({})", texts.join(", ")), ty))
            }
            "dot" => {
                let (texts, ty) = self.unified(values, None)?;
                let result = Type::Scalar(scalar_of(&ty).unwrap_or(Scalar::Float));
                if matches!(ty, Type::Scalar(_)) {
                    return Ok(Code::new(format!("({} * {})", texts[0], texts[1]), result));
                }
                if result != single {
                    let wide = with_scalar(&ty, Scalar::Float);
                    let (a, b) = (
                        format!("{}({})", glsl_base(&wide), texts[0]),
                        format!("{}({})", glsl_base(&wide), texts[1]),
                    );
                    return Ok(Code::new(format!("dot({a}, {b})"), single));
                }
                Ok(Code::new(format!("dot({}, {})", texts[0], texts[1]), result))
            }
            "cross" => {
                let vector = Type::Vector(Scalar::Float, 3);
                let right = values.pop().ok_or("cross takes two")?;
                let left = values.pop().ok_or("cross takes two")?;
                let (left, right) = (self.convert(left, &vector)?, self.convert(right, &vector)?);
                Ok(Code::new(format!("cross({}, {})", left.text, right.text), vector))
            }
            "length" => {
                let (texts, ty) = self.unified(values, float)?;
                let text = match ty {
                    Type::Scalar(_) => format!("abs({})", texts[0]),
                    _ => format!("length({})", texts[0]),
                };
                Ok(Code::new(text, single))
            }
            "rcp" => {
                let (texts, ty) = self.unified(values, float)?;
                Ok(Code::new(format!("(1.0 / {})", texts[0]), ty))
            }
            "rsqrt" => {
                let (texts, ty) = self.unified(values, float)?;
                Ok(Code::new(format!("(1.0 / sqrt({}))", texts[0]), ty))
            }
            "log10" => {
                let (texts, ty) = self.unified(values, float)?;
                Ok(Code::new(format!("(log({}) / 2.302585093)", texts[0]), ty))
            }
            "mad" => {
                let (texts, ty) = self.unified(values, float)?;
                Ok(Code::new(format!("({} * {} + {})", texts[0], texts[1], texts[2]), ty))
            }
            "mul" => self.multiply(values),
            "transpose" => {
                let value = values.pop().ok_or("transpose takes one")?;
                let Type::Matrix(scalar, rows, columns) = value.ty else {
                    return Err("transpose of a non-matrix".to_owned());
                };
                Ok(Code::new(format!("transpose({})", value.text), Type::Matrix(scalar, columns, rows)))
            }
            "determinant" => {
                let value = values.pop().ok_or("determinant takes one")?;
                Ok(Code::new(format!("determinant({})", value.text), single))
            }
            "all" | "any" => {
                let value = values.pop().ok_or("all takes one")?;
                let value = self.truth_of(value)?;
                Ok(match value.ty {
                    Type::Vector(..) => Code::new(format!("{name}({})", value.text), bool_type),
                    _ => value,
                })
            }
            "isnan" | "isinf" => {
                let (texts, ty) = self.unified(values, float)?;
                let ty = with_scalar(&ty, Scalar::Bool);
                Ok(Code::new(format!("{name}({})", texts[0]), ty))
            }
            "clip" => {
                let (texts, _) = self.unified(values, float)?;
                Ok(Code::new(format!("hlsl_clip({})", texts[0]), Type::Void))
            }
            "sincos" => {
                let [angle, sine, cosine]: [Code; 3] =
                    values.try_into().map_err(|_| "sincos takes three")?;
                let ty = sine.ty.clone();
                let angle = self.convert(angle, &ty)?;
                Ok(Code::new(
                    format!(
                        "(({} = sin({})), ({} = cos({})))",
                        sine.text, angle.text, cosine.text, angle.text
                    ),
                    Type::Void,
                ))
            }
            "modf" | "frexp" => {
                let [value, out]: [Code; 2] = values.try_into().map_err(|_| "takes two")?;
                let ty = value.ty.clone();
                let text = format!("{name}({}, {})", value.text, out.text);
                Ok(Code::new(text, ty))
            }
            "asfloat" => {
                let value = values.pop().ok_or("asfloat takes one")?;
                let ty = with_scalar(&value.ty, Scalar::Float);
                Ok(Code::new(
                    match scalar_of(&value.ty) {
                        Some(Scalar::Uint) => format!("uintBitsToFloat({})", value.text),
                        Some(Scalar::Int) => format!("intBitsToFloat({})", value.text),
                        _ => value.text,
                    },
                    ty,
                ))
            }
            "asuint" | "asint" => {
                let value = values.pop().ok_or("takes one")?;
                let unsigned = name == "asuint";
                let scalar = if unsigned { Scalar::Uint } else { Scalar::Int };
                let ty = with_scalar(&value.ty, scalar);
                let glsl = if unsigned { "floatBitsToUint" } else { "floatBitsToInt" };
                Ok(Code::new(format!("{glsl}({})", value.text), ty))
            }
            "tex2D" | "tex3D" | "texCUBE" => {
                let (texture, coordinate) = two(values)?;
                let kind = texture_kind(&texture.ty)?;
                let coordinate = self.coordinate(coordinate, kind)?;
                Ok(Code::new(format!("texture({}, {})", texture.text, coordinate.text), vec4()))
            }
            other => Err(format!("unknown function {other}")),
        }
    }

    fn multiply(&mut self, mut values: Vec<Code>) -> Result<Code, String> {
        let right = values.pop().ok_or("mul takes two")?;
        let left = values.pop().ok_or("mul takes two")?;
        match (&left.ty, &right.ty) {
            (Type::Scalar(_), _) | (_, Type::Scalar(_)) => self.binary("*", left, right),
            (Type::Matrix(scalar, rows, columns), Type::Vector(..)) => {
                let (scalar, rows, columns) = (*scalar, *rows, *columns);
                let right = self.convert(right, &Type::Vector(scalar, columns))?;
                let text = format!("({} * {})", right.text, left.text);
                Ok(Code::new(text, Type::Vector(scalar, rows)))
            }
            (Type::Vector(..), Type::Matrix(scalar, rows, columns)) => {
                let (scalar, rows, columns) = (*scalar, *rows, *columns);
                let left = self.convert(left, &Type::Vector(scalar, rows))?;
                let text = format!("({} * {})", right.text, left.text);
                Ok(Code::new(text, Type::Vector(scalar, columns)))
            }
            (Type::Matrix(scalar, rows, _), Type::Matrix(_, _, columns)) => {
                let ty = Type::Matrix(*scalar, *rows, *columns);
                Ok(Code::new(format!("({} * {})", right.text, left.text), ty))
            }
            (Type::Vector(..), Type::Vector(..)) => self.intrinsic("dot", vec![left, right]),
            (a, b) => Err(format!("mul of {a:?} and {b:?}")),
        }
    }

    fn coordinate(&mut self, value: Code, kind: TextureKind) -> Result<Code, String> {
        let size = if kind == TextureKind::Flat { 2 } else { 3 };
        self.convert(value, &Type::Vector(Scalar::Float, size))
    }

    fn method(&mut self, object: Code, name: &str, arguments: &[Expression]) -> Result<Code, String> {
        let kind = texture_kind(&object.ty)?;
        let mut values = Vec::with_capacity(arguments.len());
        for argument in arguments {
            values.push(self.expression(argument)?);
        }
        // The sampler object says nothing to GLSL: state lives on the texture.
        if matches!(values.first(), Some(Code { ty: Type::Sampler, .. })) {
            values.remove(0);
        }
        let texture = object.text;
        let mut parts = values.into_iter();
        let mut next = |what: &str| parts.next().ok_or_else(|| format!("{name} needs {what}"));
        match name {
            "Sample" | "SampleBias" | "SampleLevel" | "SampleGrad" => {
                let coordinate = next("a coordinate")?;
                let coordinate = self.coordinate(coordinate, kind)?;
                let mut rest: Vec<Code> = parts.collect();
                let count = match name {
                    "Sample" => 0,
                    "SampleGrad" => 2,
                    _ => 1,
                };
                let offset = (rest.len() > count).then(|| rest.remove(count));
                let offset = offset.map(|offset| self.convert(offset, &Type::Vector(Scalar::Int, 2))).transpose()?;
                let mut inputs = vec![texture, coordinate.text];
                for extra in rest {
                    let wanted = if name == "SampleGrad" { coordinate.ty.clone() } else { Type::Scalar(Scalar::Float) };
                    inputs.push(self.convert(extra, &wanted)?.text);
                }
                let base = match name {
                    "SampleLevel" => "textureLod",
                    "SampleGrad" => "textureGrad",
                    _ => "texture",
                };
                let text = match offset {
                    Some(offset) => {
                        // GLSL puts a bias after the offset.
                        let bias = (name == "SampleBias").then(|| inputs.pop()).flatten();
                        inputs.push(offset.text);
                        inputs.extend(bias);
                        format!("{base}Offset({})", inputs.join(", "))
                    }
                    None => format!("{base}({})", inputs.join(", ")),
                };
                Ok(Code::new(text, vec4()))
            }
            "Load" => {
                let location = next("a location")?;
                let offset = parts.next();
                let (position, level) = match kind {
                    TextureKind::Flat => (2, 2),
                    _ => (3, 3),
                };
                let whole = Type::Vector(Scalar::Int, position + 1);
                let location = self.convert(location, &whole)?;
                let swizzle = &"xyzw"[..position];
                let (held, lod) = (location.text.clone(), &"xyzw"[level..=level]);
                let mut text = format!("{held}.{swizzle}");
                if let Some(offset) = offset {
                    let offset = self.convert(offset, &Type::Vector(Scalar::Int, position))?;
                    text = format!("({text} + {})", offset.text);
                }
                Ok(Code::new(format!("texelFetch({texture}, {text}, {held}.{lod})"), vec4()))
            }
            "SampleCmp" | "SampleCmpLevelZero" => {
                let coordinate = next("a coordinate")?;
                let compare = next("a reference")?;
                let coordinate = self.coordinate(coordinate, kind)?;
                let compare = self.convert(compare, &Type::Scalar(Scalar::Float))?;
                let text = format!(
                    "(({} <= texture({texture}, {}).r) ? 1.0 : 0.0)",
                    compare.text, coordinate.text
                );
                Ok(Code::new(text, Type::Scalar(Scalar::Float)))
            }
            "Gather" | "GatherRed" | "GatherGreen" | "GatherBlue" | "GatherAlpha" => {
                let coordinate = next("a coordinate")?;
                let coordinate = self.coordinate(coordinate, kind)?;
                let component = match name {
                    "GatherGreen" => 1,
                    "GatherBlue" => 2,
                    "GatherAlpha" => 3,
                    _ => 0,
                };
                let text = match parts.next() {
                    Some(offset) => {
                        let offset = self.convert(offset, &Type::Vector(Scalar::Int, 2))?;
                        format!(
                            "textureGatherOffset({texture}, {}, {}, {component})",
                            coordinate.text, offset.text
                        )
                    }
                    None => format!("textureGather({texture}, {}, {component})", coordinate.text),
                };
                Ok(Code::new(text, vec4()))
            }
            "GetDimensions" => {
                let rest: Vec<Code> = parts.collect();
                // (mip, width, height, levels) or (width, height).
                let (level, sizes) = match rest.len() {
                    4 | 5 => (rest[0].text.clone(), &rest[1..3]),
                    _ => ("0".to_owned(), &rest[..2.min(rest.len())]),
                };
                let mut writes = Vec::new();
                for (axis, size) in sizes.iter().enumerate() {
                    let read = Code::new(
                        format!("textureSize({texture}, {level}).{}", &"xy"[axis..=axis]),
                        Type::Scalar(Scalar::Int),
                    );
                    let read = self.convert(read, &size.ty)?;
                    writes.push(format!("({} = {})", size.text, read.text));
                }
                Ok(Code::new(format!("({})", writes.join(", ")), Type::Void))
            }
            other => Err(format!("unknown texture method {other}")),
        }
    }

    fn main_function(
        &mut self,
        function: &Function,
        symbol: &str,
    ) -> Result<(Vec<Interface>, String), String> {
        let mut header = String::new();
        let mut body = String::from("    hlsl_init_globals();\n");
        let mut attributes = Vec::new();
        let mut call = Vec::new();
        let mut after = String::new();
        for (at, parameter) in function.parameters.iter().enumerate() {
            let ty = self.resolved(&parameter.variable)?;
            let local = format!("hlsl_p{at}");
            body.push_str(&format!("    {};\n", declaration(&ty, &local)));
            call.push(local.clone());
            if parameter.direction != Direction::In {
                self.wire_output(&ty, parameter.variable.semantic.as_deref(), &local, &mut header, &mut after)?;
            }
            if parameter.direction != Direction::Out {
                self.wire_input(
                    &ty,
                    parameter.variable.semantic.as_deref(),
                    &local,
                    &mut header,
                    &mut body,
                    &mut attributes,
                )?;
            }
        }
        let call = format!("{symbol}({})", call.join(", "));
        if function.returns == Type::Void {
            body.push_str(&format!("    {call};\n"));
        } else {
            body.push_str(&format!("    {} = {call};\n", declaration(&function.returns, "hlsl_result")));
            self.wire_output(&function.returns, function.semantic.as_deref(), "hlsl_result", &mut header, &mut after)?;
        }
        body.push_str(&after);
        Ok((attributes, format!("{header}void main() {{\n{body}}}\n")))
    }

    fn wire_input(
        &mut self,
        ty: &Type,
        semantic: Option<&str>,
        target: &str,
        header: &mut String,
        body: &mut String,
        attributes: &mut Vec<Interface>,
    ) -> Result<(), String> {
        if let Type::Named(name) = ty {
            let fields = self.struct_fields(name)?.to_vec();
            for field in fields {
                let ty = self.resolved(&field)?;
                let place = format!("{target}.{}", field_name(&field.name));
                self.wire_input(&ty, field.semantic.as_deref(), &place, header, body, attributes)?;
            }
            return Ok(());
        }
        let Some(semantic) = semantic else {
            return Ok(());
        };
        let upper = semantic.to_ascii_uppercase();
        let source = match (self.stage, upper.as_str()) {
            (Stage::Pixel, "SV_POSITION" | "VPOS") => "gl_FragCoord".to_owned(),
            (Stage::Pixel, "SV_ISFRONTFACE" | "VFACE") => "gl_FrontFacing".to_owned(),
            (Stage::Pixel, "SV_CLIPDISTANCE") => return Ok(()),
            (Stage::Vertex, "SV_VERTEXID") => "gl_VertexID".to_owned(),
            (Stage::Vertex, "SV_INSTANCEID") => "gl_InstanceID".to_owned(),
            (Stage::Vertex, _) => {
                // The preview's vertices are all floats, whatever HLSL calls them.
                let glsl = format!("a_{upper}");
                let attribute = with_scalar(ty, Scalar::Float);
                header.push_str(&format!("in {} {glsl};\n", glsl_base(&attribute)));
                attributes.push(Interface { semantic: upper.clone(), glsl: glsl.clone(), ty: attribute });
                if attribute_is_float(ty) { glsl } else { format!("{}({glsl})", glsl_base(ty)) }
            }
            (Stage::Pixel, _) => {
                let glsl = varying_name(&upper);
                let flat = if matches!(scalar_of(ty), Some(Scalar::Float)) { "" } else { "flat " };
                let line = format!("{flat}in {} {glsl};\n", glsl_base(ty));
                if !header.contains(&line) {
                    header.push_str(&line);
                }
                glsl
            }
        };
        let source_type = match upper.as_str() {
            "SV_POSITION" | "VPOS" => Type::Vector(Scalar::Float, 4),
            "SV_ISFRONTFACE" | "VFACE" => Type::Scalar(Scalar::Bool),
            "SV_VERTEXID" | "SV_INSTANCEID" => Type::Scalar(Scalar::Int),
            _ => ty.clone(),
        };
        let value = self.convert(Code::new(source, source_type), ty)?;
        body.push_str(&format!("    {target} = {};\n", value.text));
        Ok(())
    }

    fn wire_output(
        &mut self,
        ty: &Type,
        semantic: Option<&str>,
        source: &str,
        header: &mut String,
        after: &mut String,
    ) -> Result<(), String> {
        if let Type::Named(name) = ty {
            let fields = self.struct_fields(name)?.to_vec();
            for field in fields {
                let ty = self.resolved(&field)?;
                let place = format!("{source}.{}", field_name(&field.name));
                self.wire_output(&ty, field.semantic.as_deref(), &place, header, after)?;
            }
            return Ok(());
        }
        let Some(semantic) = semantic else {
            return Ok(());
        };
        let upper = semantic.to_ascii_uppercase();
        match (self.stage, upper.as_str()) {
            (Stage::Vertex, "SV_POSITION" | "POSITION") => {
                after.push_str(&format!("    gl_Position = {source};\n"));
            }
            (_, "SV_CLIPDISTANCE") => {}
            (Stage::Pixel, "SV_DEPTH" | "DEPTH") => {
                after.push_str(&format!("    gl_FragDepth = {source};\n"));
            }
            (Stage::Vertex, _) => {
                let glsl = varying_name(&upper);
                let flat = if matches!(scalar_of(ty), Some(Scalar::Float)) { "" } else { "flat " };
                header.push_str(&format!("{flat}out {} {glsl};\n", glsl_base(ty)));
                after.push_str(&format!("    {glsl} = {source};\n"));
            }
            (Stage::Pixel, _) => {
                let index = target_index(&upper).ok_or_else(|| format!("output {semantic}"))?;
                header.push_str(&format!("layout(location = {index}) out vec4 o_target{index};\n"));
                let wide = Type::Vector(Scalar::Float, 4);
                let value = match ty {
                    Type::Vector(_, 4) => Code::new(source, ty.clone()),
                    Type::Scalar(_) => Code::new(format!("vec4({source}, 0.0, 0.0, 0.0)"), wide.clone()),
                    Type::Vector(_, size) => {
                        let padding = vec!["0.0"; 4 - size].join(", ");
                        Code::new(format!("vec4({source}, {padding})"), wide.clone())
                    }
                    other => return Err(format!("output of {other:?}")),
                };
                let value = self.convert(value, &wide)?;
                after.push_str(&format!("    o_target{index} = {};\n", value.text));
            }
        }
        Ok(())
    }
}

fn attribute_is_float(ty: &Type) -> bool {
    scalar_of(ty) == Some(Scalar::Float)
}

fn vec4() -> Type {
    Type::Vector(Scalar::Float, 4)
}

fn two(values: Vec<Code>) -> Result<(Code, Code), String> {
    let mut values = values.into_iter();
    match (values.next(), values.next()) {
        (Some(first), Some(second)) => Ok((first, second)),
        _ => Err("expected two arguments".to_owned()),
    }
}

fn texture_kind(ty: &Type) -> Result<TextureKind, String> {
    match ty {
        Type::Texture(kind) => Ok(*kind),
        Type::Named(name) => pair_kind(name).ok_or_else(|| format!("{name} is not a texture")),
        other => Err(format!("{other:?} is not a texture")),
    }
}

fn field_name(name: &str) -> String {
    if RESERVED_FIELDS.contains(&name) { format!("{name}_") } else { name.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::super::parse::parse;
    use super::super::pre::Preprocessor;
    use super::*;

    fn glsl(source: &str, entry: &str, stage: Stage) -> Translated {
        let mut include = |_: &str| None;
        let tokens = Preprocessor::new(&[], &mut include).run(source).unwrap();
        translate(&parse(&tokens).unwrap(), entry, stage).unwrap()
    }

    fn pixel(body: &str) -> String {
        let source = format!("float4 m[2]; float4x3 t;\n{body}\n");
        glsl(&source, "main_ps", Stage::Pixel).source
    }

    #[test]
    fn a_wider_value_truncates_on_assignment() {
        let source = pixel("float4 main_ps(float4 a : TEXCOORD0) : SV_Target { float3 b = a; return b.xyzz; }");
        assert!(source.contains("vec3 l_b = l_a.xyz;"), "{source}");
    }

    #[test]
    fn a_scalar_splats_on_assignment() {
        let source = pixel("float4 main_ps() : SV_Target { float3 b = 1; return b.xyzz; }");
        assert!(source.contains("vec3 l_b = vec3(1.0);"), "{source}");
    }

    #[test]
    fn mixed_shapes_truncate_in_arithmetic() {
        let source = pixel("float4 main_ps(float4 a : TEXCOORD0, float2 b : TEXCOORD1) : SV_Target { float2 c = a + b; return c.xyxy; }");
        assert!(source.contains("(l_a.xy + l_b)"), "{source}");
    }

    #[test]
    fn a_row_major_mul_swaps_for_glsl() {
        let source = pixel("float4 main_ps(float3 a : TEXCOORD0) : SV_Target { float3 b = mul(t, a); return b.xyzz; }");
        assert!(source.contains("(l_a * t)"), "{source}");
        assert!(source.contains("uniform mat4x3 t;"), "{source}");
    }

    #[test]
    fn overloads_bind_to_the_closest_parameter_type() {
        let source = pixel("float f(float2 a) { return 1; } float f(float3 a) { return 2; } float4 main_ps(float3 a : TEXCOORD0) : SV_Target { return f(a); }");
        assert!(source.contains("float f1_f(in vec3 l_a)"), "{source}");
        assert!(!source.contains("f0_f"), "the unused overload is not written: {source}");
    }

    #[test]
    fn a_sampler_pair_collapses_to_its_texture() {
        let source = "struct texture_sampler_2d { sampler s; texture2D t; };\nsampler a; texture2D b;\nstatic const texture_sampler_2d pair = { a, b };\nfloat4 sample2D(in texture_sampler_2d s, in float2 uv) { return s.t.Sample(s.s, uv); }\nfloat4 main_ps(float2 uv : TEXCOORD0) : SV_Target { return sample2D(pair, uv); }\n";
        let translated = glsl(source, "main_ps", Stage::Pixel);
        assert!(translated.source.contains("texture(l_s, l_uv)"), "{}", translated.source);
        assert_eq!(translated.textures, [("b".to_owned(), TextureKind::Flat)]);
    }

    #[test]
    fn statics_fed_by_uniforms_initialize_at_main() {
        let source = pixel("float u; static float s = u * 2; float4 main_ps() : SV_Target { return s; }");
        assert!(source.contains("g_s = (u * 2.0);"), "{source}");
        assert!(source.contains("hlsl_init_globals();"), "{source}");
    }

    #[test]
    fn unreached_code_never_has_to_translate() {
        let translated = glsl(
            "float nope() { return unknown_thing(1); }\nfloat4 main_ps() : SV_Target { return 1; }\n",
            "main_ps",
            Stage::Pixel,
        );
        assert!(!translated.source.contains("nope"));
    }

    #[test]
    fn nested_arrays_are_stored_flat() {
        let source = "float4 n[4][3];\nfloat4 main_vs(float4 p : POSITION) : SV_Position { float4 r[3] = n[1]; return n[2][1]; }\n";
        let text = glsl(source, "main_vs", Stage::Vertex).source;
        assert!(text.contains("uniform vec4 n[12];"), "{text}");
        assert!(text.contains("n[(1) * 3 + 1 ]") || text.contains("n[(2) * 3 + 1]"), "{text}");
    }

    #[test]
    fn an_entry_wires_its_semantics() {
        let source = "struct v { float4 p : SV_Position; float2 uv : TEXCOORD0; };\nstruct a { float3 p : POSITION; };\nv main_vs(in a i) { v o; o.p = float4(i.p, 1); o.uv = 0; return o; }\n";
        let translated = glsl(source, "main_vs", Stage::Vertex);
        assert_eq!(translated.attributes[0].semantic, "POSITION");
        assert!(translated.source.contains("gl_Position = hlsl_result.p;"), "{}", translated.source);
        assert!(translated.source.contains("out vec2 v_TEXCOORD0;"), "{}", translated.source);
    }

    #[test]
    fn writing_out_through_another_shape_goes_by_way_of_a_copy() {
        let source = pixel("void f(out float3 a) { a = 1; } float4 main_ps() : SV_Target { float4 b = 0; f(b); return b; }");
        assert!(source.contains("hlsl_t1"), "{source}");
    }
}
