//! HLSL's grammar, as far as the kit's shaders use it.

use std::sync::Arc;

use super::pre::{Token, TokenKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Scalar {
    Bool,
    Int,
    Uint,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TextureKind {
    /// `Texture1D`: a row of texels, read by one coordinate.
    Line,
    Flat,
    FlatArray,
    Volume,
    Cube,
    /// `ByteAddressBuffer`: words read by byte address.
    Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Type {
    Void,
    Scalar(Scalar),
    Vector(Scalar, usize),
    /// Rows, then columns.
    Matrix(Scalar, usize, usize),
    /// Element and length; none for a size left to its initializer.
    Array(Box<Type>, Option<usize>),
    Named(Arc<str>),
    Texture(TextureKind),
    Sampler,
}

#[derive(Debug, Clone)]
pub(crate) struct Variable {
    pub(crate) ty: Type,
    pub(crate) name: Arc<str>,
    pub(crate) semantic: Option<Arc<str>>,
    pub(crate) init: Option<Expression>,
    /// Array lengths not yet folded, outermost first.
    pub(crate) dimensions: Vec<Option<Expression>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    In,
    Out,
    InOut,
}

#[derive(Debug, Clone)]
pub(crate) struct Parameter {
    pub(crate) direction: Direction,
    /// `uniform`: an entry point's parameter the effect pass compiles in, from
    /// its literal arguments in order.
    pub(crate) uniform: bool,
    pub(crate) variable: Variable,
}

#[derive(Debug)]
pub(crate) struct Function {
    pub(crate) returns: Type,
    pub(crate) name: Arc<str>,
    pub(crate) parameters: Vec<Parameter>,
    pub(crate) semantic: Option<Arc<str>>,
    pub(crate) body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub(crate) enum Expression {
    Number(Arc<str>),
    Name(Arc<str>),
    Unary(Arc<str>, Box<Expression>),
    Postfix(Arc<str>, Box<Expression>),
    Binary(Arc<str>, Box<Expression>, Box<Expression>),
    Assign(Arc<str>, Box<Expression>, Box<Expression>),
    Ternary(Box<Expression>, Box<Expression>, Box<Expression>),
    Comma(Box<Expression>, Box<Expression>),
    Cast(Type, Box<Expression>),
    Call(Arc<str>, Vec<Expression>),
    /// `float3(..)`, or an array's `float[2](..)`.
    Construct(Type, Vec<Expression>),
    Method(Box<Expression>, Arc<str>, Vec<Expression>),
    Member(Box<Expression>, Arc<str>),
    Index(Box<Expression>, Box<Expression>),
    List(Vec<Expression>),
}

#[derive(Debug, Clone)]
pub(crate) enum Statement {
    Declare(Vec<Variable>),
    Expression(Expression),
    Block(Vec<Statement>),
    If(Expression, Box<Statement>, Option<Box<Statement>>),
    For(Option<Box<Statement>>, Option<Expression>, Option<Expression>, Box<Statement>),
    While(Expression, Box<Statement>),
    DoWhile(Box<Statement>, Expression),
    /// Cases in order: the label, none for `default`, and its statements.
    Switch(Expression, Vec<(Option<Expression>, Vec<Statement>)>),
    Return(Option<Expression>),
    Discard,
    Break,
    Continue,
}

/// One `SetVertexShader(CompileShader(vs_5_0, f(..)))` of an effect pass.
#[derive(Debug, Clone)]
pub(crate) struct PassShader {
    /// `vs`, `ps`, `cs` and so on: the profile's stage.
    pub(crate) stage: Arc<str>,
    pub(crate) function: Arc<str>,
    /// Literal arguments the pass compiles the function with.
    pub(crate) arguments: Vec<Expression>,
}

/// A D3DX effect's `technique11 name { pass name { .. } }`.
#[derive(Debug, Clone)]
pub(crate) struct Technique {
    pub(crate) name: Arc<str>,
    pub(crate) passes: Vec<(Arc<str>, Vec<PassShader>)>,
}

#[derive(Debug, Default)]
pub(crate) struct Program {
    pub(crate) structs: Vec<(Arc<str>, Vec<Variable>)>,
    pub(crate) typedefs: Vec<(Arc<str>, Type)>,
    /// In source order: a static's initializer may read an earlier one.
    pub(crate) globals: Vec<Variable>,
    pub(crate) functions: Vec<Function>,
    /// An effect file's techniques, in source order.
    pub(crate) techniques: Vec<Technique>,
}

impl Program {
    /// The function a technique's pass compiles for a stage (`vs`, `ps`).
    pub(crate) fn pass_function(&self, technique: &str, pass: &str, stage: &str) -> Option<&PassShader> {
        let technique = self.techniques.iter().find(|candidate| &*candidate.name == technique)?;
        let (_, shaders) = technique.passes.iter().find(|(name, _)| &**name == pass)?;
        shaders.iter().find(|shader| &*shader.stage == stage)
    }
}

const MODIFIERS: [&str; 13] = [
    "static", "const", "uniform", "extern", "shared", "volatile", "row_major", "column_major",
    "linear", "centroid", "nointerpolation", "noperspective", "precise",
];

const TEMPLATE_TYPES: [&str; 2] = ["vector", "matrix"];

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    program: Program,
}

pub(crate) fn parse(tokens: &[Token]) -> Result<Program, String> {
    let mut parser = Parser { tokens, at: 0, program: Program::default() };
    while parser.peek().is_some() {
        parser.top_level().map_err(|error| parser.located(&error))?;
    }
    Ok(parser.program)
}

fn scalar_type(name: &str) -> Option<Scalar> {
    match name {
        "bool" => Some(Scalar::Bool),
        "int" | "min16int" => Some(Scalar::Int),
        "uint" | "dword" | "min16uint" => Some(Scalar::Uint),
        "float" | "half" | "double" | "min16float" | "min10float" | "real" => Some(Scalar::Float),
        _ => None,
    }
}

/// A built-in numeric type by name: `float`, `float3`, `float3x4`.
pub(crate) fn numeric_type(name: &str) -> Option<Type> {
    // `min16float` has digits of its own.
    let skip = if name.starts_with("min1") { 5 } else { 0 };
    let Some(split) = name.get(skip..)?.find(|character: char| character.is_ascii_digit()) else {
        return scalar_type(name).map(Type::Scalar);
    };
    let (base, shape) = name.split_at(skip + split);
    let scalar = scalar_type(base)?;
    let bytes = shape.as_bytes();
    let dimension = |byte: u8| (b'1'..=b'4').contains(&byte).then(|| (byte - b'0') as usize);
    match bytes {
        [] => Some(Type::Scalar(scalar)),
        [size] => match dimension(*size)? {
            1 => Some(Type::Scalar(scalar)),
            size => Some(Type::Vector(scalar, size)),
        },
        [rows, b'x', columns] => Some(Type::Matrix(scalar, dimension(*rows)?, dimension(*columns)?)),
        _ => None,
    }
}

fn texture_type(name: &str) -> Option<Type> {
    let kind = match name.to_ascii_lowercase().as_str() {
        "texture1d" => TextureKind::Line,
        "texture2d" | "rwtexture2d" => TextureKind::Flat,
        "texture2darray" => TextureKind::FlatArray,
        "texture3d" => TextureKind::Volume,
        "texturecube" => TextureKind::Cube,
        "byteaddressbuffer" => TextureKind::Bytes,
        _ => return None,
    };
    Some(Type::Texture(kind))
}

impl Parser<'_> {
    fn located(&self, error: &str) -> String {
        let from = self.at.saturating_sub(8);
        let to = (self.at + 8).min(self.tokens.len());
        let near: Vec<_> = self.tokens[from..to].iter().map(|token| &*token.text).collect();
        format!("{error} near `{}`", near.join(" "))
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn peek_at(&self, ahead: usize) -> Option<&Token> {
        self.tokens.get(self.at + ahead)
    }

    fn at_mark(&self, text: &str) -> bool {
        self.peek().is_some_and(|token| token.is(text))
    }

    fn eat(&mut self, text: &str) -> bool {
        let found = self.at_mark(text);
        self.at += usize::from(found);
        found
    }

    fn expect(&mut self, text: &str) -> Result<(), String> {
        if self.eat(text) { Ok(()) } else { Err(format!("expected {text}")) }
    }

    fn identifier(&mut self) -> Result<Arc<str>, String> {
        let token = self.peek().ok_or("ran out of tokens")?;
        if token.kind != TokenKind::Identifier {
            return Err("expected a name".to_owned());
        }
        let name = token.text.clone();
        self.at += 1;
        Ok(name)
    }

    fn is_type_name(&self, name: &str) -> bool {
        numeric_type(name).is_some()
            || texture_type(name).is_some()
            || TEMPLATE_TYPES.contains(&name)
            || matches!(name, "void" | "sampler" | "SamplerState" | "SamplerComparisonState")
            || name.starts_with("sampler2D")
            || self.program.structs.iter().any(|(known, _)| &**known == name)
            || self.program.typedefs.iter().any(|(known, _)| &**known == name)
    }

    /// `[attribute]`s, which only steer D3D's compiler.
    fn skip_attributes(&mut self) {
        while self.at_mark("[") {
            let mut depth = 0;
            while let Some(token) = self.peek() {
                depth += i32::from(token.is("[")) - i32::from(token.is("]"));
                self.at += 1;
                if depth == 0 {
                    break;
                }
            }
        }
    }

    fn top_level(&mut self) -> Result<(), String> {
        if self.eat(";") {
            return Ok(());
        }
        self.skip_attributes();
        if self.at_mark("technique") || self.at_mark("technique10") || self.at_mark("technique11") {
            return self.technique();
        }
        // An effect's `string` globals only annotate it for tools.
        if self.at_mark("string") || self.modifiers_then("string") {
            self.modifiers();
            self.at += 1;
            self.identifier()?;
            self.skip_annotations()?;
            return self.skip_statement();
        }
        if self.at_mark("struct") {
            self.structure()?;
            return Ok(());
        }
        if self.eat("typedef") {
            // `typedef struct { .. } a, b;` names one anonymous struct twice.
            let ty = if self.at_mark("struct") && self.peek_at(1).is_some_and(|token| token.is("{")) {
                self.at += 1;
                let anonymous: Arc<str> = Arc::from(format!("__anonymous_{}", self.program.structs.len()));
                self.struct_body(anonymous.clone())?;
                Type::Named(anonymous)
            } else {
                self.type_name()?
            };
            loop {
                let name = self.identifier()?;
                self.program.typedefs.push((name, ty.clone()));
                if !self.eat(",") {
                    break;
                }
            }
            return self.expect(";");
        }
        if self.at_mark("cbuffer") || self.at_mark("tbuffer") {
            self.at += 1;
            self.identifier()?;
            self.skip_register()?;
            self.expect("{")?;
            while !self.eat("}") {
                if self.eat(";") {
                    continue;
                }
                let mut declared = self.declaration()?;
                self.program.globals.append(&mut declared);
            }
            return Ok(());
        }
        self.modifiers();
        let ty = self.type_name()?;
        let after_type = self.at;
        let name = self.identifier()?;
        if self.eat("(") {
            return self.function(ty, name);
        }
        self.at = after_type;
        let mut declared = self.declarators(ty)?;
        self.program.globals.append(&mut declared);
        Ok(())
    }

    fn structure(&mut self) -> Result<Arc<str>, String> {
        self.expect("struct")?;
        let name = self.identifier()?;
        self.struct_body(name)
    }

    fn struct_body(&mut self, name: Arc<str>) -> Result<Arc<str>, String> {
        self.program.structs.push((name.clone(), Vec::new()));
        let slot = self.program.structs.len() - 1;
        self.expect("{")?;
        let mut fields = Vec::new();
        while !self.eat("}") {
            let mut declared = self.declaration()?;
            fields.append(&mut declared);
        }
        self.eat(";");
        self.program.structs[slot].1 = fields;
        Ok(name)
    }

    /// Skip storage words, saying whether there were any.
    fn modifiers(&mut self) -> bool {
        let start = self.at;
        while self.peek().is_some_and(|token| MODIFIERS.iter().any(|modifier| token.is(modifier))) {
            self.at += 1;
        }
        self.at != start
    }

    fn type_name(&mut self) -> Result<Type, String> {
        self.eat("struct");
        let name = self.identifier()?;
        let ty = self.named_type(&name)?;
        // A texture's element type, which the shaders never lean on.
        if matches!(ty, Type::Texture(_)) && self.at_mark("<") {
            self.skip_template()?;
        }
        Ok(ty)
    }

    fn named_type(&mut self, name: &str) -> Result<Type, String> {
        if let Some(ty) = numeric_type(name).or_else(|| texture_type(name)) {
            return Ok(ty);
        }
        match name {
            "void" => return Ok(Type::Void),
            "sampler" | "SamplerState" | "SamplerComparisonState" => return Ok(Type::Sampler),
            "sampler2D" => return Ok(Type::Texture(TextureKind::Flat)),
            "vector" | "matrix" => return self.template_type(name),
            _ => {}
        }
        if self.program.structs.iter().any(|(known, _)| &**known == name) {
            return Ok(Type::Named(Arc::from(name)));
        }
        let typedef = self.program.typedefs.iter().find(|(known, _)| &**known == name);
        typedef.map(|(_, ty)| ty.clone()).ok_or_else(|| format!("unknown type {name}"))
    }

    fn template_type(&mut self, name: &str) -> Result<Type, String> {
        self.expect("<")?;
        let element = self.identifier()?;
        let scalar = scalar_type(&element).ok_or("template of a non-scalar")?;
        let mut size = || -> Result<usize, String> {
            self.expect(",")?;
            let value = self.peek().and_then(|token| token.text.parse().ok()).ok_or("template size")?;
            self.at += 1;
            Ok(value)
        };
        let ty = if name == "vector" {
            Type::Vector(scalar, size()?)
        } else {
            let rows = size()?;
            Type::Matrix(scalar, rows, size()?)
        };
        self.expect(">")?;
        Ok(ty)
    }

    fn skip_template(&mut self) -> Result<(), String> {
        self.expect("<")?;
        while !self.eat(">") {
            self.at += 1;
            if self.peek().is_none() {
                return Err("template never closes".to_owned());
            }
        }
        Ok(())
    }

    /// `: register(..)` and `: packoffset(..)`, which GLSL binds by name.
    fn skip_register(&mut self) -> Result<(), String> {
        while self.at_mark(":")
            && self.peek_at(1).is_some_and(|token| token.is("register") || token.is("packoffset"))
        {
            self.at += 2;
            self.expect("(")?;
            let mut depth = 1;
            while depth > 0 {
                let token = self.peek().ok_or("register never closes")?;
                depth += i32::from(token.is("(")) - i32::from(token.is(")"));
                self.at += 1;
            }
        }
        Ok(())
    }

    fn semantic(&mut self) -> Result<Option<Arc<str>>, String> {
        self.skip_register()?;
        let mut semantic = None;
        if self.eat(":") {
            semantic = Some(self.identifier()?);
        }
        self.skip_register()?;
        Ok(semantic)
    }

    fn declaration(&mut self) -> Result<Vec<Variable>, String> {
        self.modifiers();
        let ty = self.type_name()?;
        self.declarators(ty)
    }

    fn declarators(&mut self, ty: Type) -> Result<Vec<Variable>, String> {
        let mut variables = Vec::new();
        loop {
            variables.push(self.declarator(ty.clone())?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(";")?;
        Ok(variables)
    }

    fn declarator(&mut self, ty: Type) -> Result<Variable, String> {
        let name = self.identifier()?;
        let dimensions = self.dimensions()?;
        let semantic = self.semantic()?;
        self.skip_annotations()?;
        // `= sampler_state { .. }` sets D3DX sampler state, which the
        // engine overrides at draw time.
        if self.at_mark("=") && self.peek_at(1).is_some_and(|token| token.is("sampler_state")) {
            self.at += 2;
            self.skip_braces()?;
            return Ok(Variable { ty, name, semantic, init: None, dimensions });
        }
        let init = if self.eat("=") { Some(self.initializer()?) } else { None };
        Ok(Variable { ty, name, semantic, init, dimensions })
    }

    /// Whether storage words lead up to `word`, without moving.
    fn modifiers_then(&self, word: &str) -> bool {
        let mut at = self.at;
        while self.tokens.get(at).is_some_and(|token| MODIFIERS.iter().any(|modifier| token.is(modifier))) {
            at += 1;
        }
        at != self.at && self.tokens.get(at).is_some_and(|token| token.is(word))
    }

    /// Up to and past the next `;` at this nesting.
    fn skip_statement(&mut self) -> Result<(), String> {
        let mut depth = 0;
        loop {
            let token = self.peek().ok_or("statement never ends")?;
            depth += i32::from(token.is("{") || token.is("(")) - i32::from(token.is("}") || token.is(")"));
            let ends = depth == 0 && token.is(";");
            self.at += 1;
            if ends {
                return Ok(());
            }
        }
    }

    fn skip_braces(&mut self) -> Result<(), String> {
        self.expect("{")?;
        let mut depth = 1;
        while depth > 0 {
            let token = self.peek().ok_or("braces never close")?;
            depth += i32::from(token.is("{")) - i32::from(token.is("}"));
            self.at += 1;
        }
        Ok(())
    }

    /// `< .. >` effect annotations, which describe a variable to tools.
    fn skip_annotations(&mut self) -> Result<(), String> {
        while self.at_mark("<") {
            let mut depth = 0;
            loop {
                let token = self.peek().ok_or("annotation never closes")?;
                depth += i32::from(token.is("<")) - i32::from(token.is(">"));
                self.at += 1;
                if depth == 0 {
                    break;
                }
            }
        }
        Ok(())
    }

    /// `technique11 name <..> { pass name <..> { state; .. } .. }`. Each
    /// pass keeps its `SetXShader(CompileShader(profile, f(args)))`; other
    /// state is the engine's to set.
    fn technique(&mut self) -> Result<(), String> {
        self.at += 1;
        // A technique may go unnamed (water's).
        let name = if self.at_mark("{") || self.at_mark("<") { Arc::from("") } else { self.identifier()? };
        self.skip_annotations()?;
        self.expect("{")?;
        let mut passes = Vec::new();
        while !self.eat("}") {
            self.expect("pass")?;
            let pass = self.identifier()?;
            self.skip_annotations()?;
            self.expect("{")?;
            let mut shaders = Vec::new();
            while !self.eat("}") {
                let setter = self.peek().map(|token| token.text.clone()).ok_or("pass never closes")?;
                let stage = setter.strip_prefix("Set").and_then(|rest| rest.strip_suffix("Shader"));
                if let Some(stage) = stage.filter(|_| self.peek_at(1).is_some_and(|token| token.is("("))) {
                    let stage: Arc<str> = Arc::from(match stage {
                        "Vertex" => "vs",
                        "Pixel" => "ps",
                        "Compute" => "cs",
                        "Geometry" => "gs",
                        "Hull" => "hs",
                        "Domain" => "ds",
                        other => other,
                    });
                    self.at += 2;
                    if self.eat("CompileShader") {
                        self.expect("(")?;
                        self.identifier()?;
                        self.expect(",")?;
                        let function = self.identifier()?;
                        self.expect("(")?;
                        let mut arguments = Vec::new();
                        if !self.eat(")") {
                            loop {
                                arguments.push(self.assignment()?);
                                if !self.eat(",") {
                                    break;
                                }
                            }
                            self.expect(")")?;
                        }
                        // Closing `CompileShader(` and `SetXShader(`.
                        self.expect(")")?;
                        self.expect(")")?;
                        shaders.push(PassShader { stage, function, arguments });
                    } else {
                        // `SetGeometryShader(NULL)` and the like.
                        let mut depth = 1;
                        while depth > 0 {
                            let token = self.peek().ok_or("pass state never closes")?;
                            depth += i32::from(token.is("(")) - i32::from(token.is(")"));
                            self.at += 1;
                        }
                    }
                    self.expect(";")?;
                } else {
                    self.skip_statement()?;
                }
            }
            passes.push((pass, shaders));
        }
        self.eat(";");
        self.program.techniques.push(Technique { name, passes });
        Ok(())
    }

    fn dimensions(&mut self) -> Result<Vec<Option<Expression>>, String> {
        let mut dimensions = Vec::new();
        while self.eat("[") {
            if self.eat("]") {
                dimensions.push(None);
            } else {
                dimensions.push(Some(self.expression()?));
                self.expect("]")?;
            }
        }
        Ok(dimensions)
    }

    fn initializer(&mut self) -> Result<Expression, String> {
        if !self.eat("{") {
            return self.assignment();
        }
        let mut items = Vec::new();
        while !self.eat("}") {
            items.push(self.initializer()?);
            if !self.eat(",") {
                self.expect("}")?;
                break;
            }
        }
        Ok(Expression::List(items))
    }

    fn function(&mut self, returns: Type, name: Arc<str>) -> Result<(), String> {
        let mut parameters = Vec::new();
        if !self.eat(")") {
            if self.at_mark("void") && self.peek_at(1).is_some_and(|token| token.is(")")) {
                self.at += 1;
            } else {
                loop {
                    parameters.push(self.parameter()?);
                    if !self.eat(",") {
                        break;
                    }
                }
            }
            self.expect(")")?;
        }
        let semantic = self.semantic()?;
        if self.eat(";") {
            return Ok(());
        }
        self.expect("{")?;
        let body = self.block()?;
        self.program.functions.push(Function { returns, name, parameters, semantic, body });
        Ok(())
    }

    fn parameter(&mut self) -> Result<Parameter, String> {
        let mut direction = Direction::In;
        let mut uniform = false;
        loop {
            if self.eat("uniform") {
                uniform = true;
            } else if self.eat("in") {
            } else if self.eat("out") {
                direction = Direction::Out;
            } else if self.eat("inout") {
                direction = Direction::InOut;
            } else if !self.modifiers() {
                break;
            }
        }
        let ty = self.type_name()?;
        Ok(Parameter { direction, uniform, variable: self.declarator(ty)? })
    }

    fn block(&mut self) -> Result<Vec<Statement>, String> {
        let mut statements = Vec::new();
        while !self.eat("}") {
            if self.peek().is_none() {
                return Err("block never closes".to_owned());
            }
            statements.push(self.statement()?);
        }
        Ok(statements)
    }

    fn at_declaration(&self) -> bool {
        let Some(token) = self.peek() else {
            return false;
        };
        if MODIFIERS.iter().any(|modifier| token.is(modifier)) {
            return true;
        }
        token.kind == TokenKind::Identifier
            && self.is_type_name(&token.text)
            && self.peek_at(1).is_some_and(|next| {
                next.kind == TokenKind::Identifier || next.is("<")
            })
    }

    fn statement(&mut self) -> Result<Statement, String> {
        self.skip_attributes();
        if self.eat("{") {
            return Ok(Statement::Block(self.block()?));
        }
        if self.eat(";") {
            return Ok(Statement::Block(Vec::new()));
        }
        if self.eat("if") {
            self.expect("(")?;
            let condition = self.expression()?;
            self.expect(")")?;
            let then = Box::new(self.statement()?);
            let otherwise = if self.eat("else") { Some(Box::new(self.statement()?)) } else { None };
            return Ok(Statement::If(condition, then, otherwise));
        }
        if self.eat("for") {
            self.expect("(")?;
            let init = if self.eat(";") {
                None
            } else if self.at_declaration() {
                Some(Box::new(Statement::Declare(self.declaration()?)))
            } else {
                let init = self.expression()?;
                self.expect(";")?;
                Some(Box::new(Statement::Expression(init)))
            };
            let condition = if self.at_mark(";") { None } else { Some(self.expression()?) };
            self.expect(";")?;
            let step = if self.at_mark(")") { None } else { Some(self.expression()?) };
            self.expect(")")?;
            return Ok(Statement::For(init, condition, step, Box::new(self.statement()?)));
        }
        if self.eat("while") {
            self.expect("(")?;
            let condition = self.expression()?;
            self.expect(")")?;
            return Ok(Statement::While(condition, Box::new(self.statement()?)));
        }
        if self.eat("do") {
            let body = Box::new(self.statement()?);
            self.expect("while")?;
            self.expect("(")?;
            let condition = self.expression()?;
            self.expect(")")?;
            self.expect(";")?;
            return Ok(Statement::DoWhile(body, condition));
        }
        if self.eat("switch") {
            self.expect("(")?;
            let value = self.expression()?;
            self.expect(")")?;
            self.expect("{")?;
            let mut cases: Vec<(Option<Expression>, Vec<Statement>)> = Vec::new();
            while !self.eat("}") {
                if self.eat("case") {
                    let label = self.ternary()?;
                    self.expect(":")?;
                    cases.push((Some(label), Vec::new()));
                } else if self.eat("default") {
                    self.expect(":")?;
                    cases.push((None, Vec::new()));
                } else {
                    let statement = self.statement()?;
                    cases.last_mut().ok_or("a statement before any case")?.1.push(statement);
                }
            }
            return Ok(Statement::Switch(value, cases));
        }
        if self.eat("return") {
            let value = if self.at_mark(";") { None } else { Some(self.expression()?) };
            self.expect(";")?;
            return Ok(Statement::Return(value));
        }
        for (word, statement) in
            [("discard", Statement::Discard), ("break", Statement::Break), ("continue", Statement::Continue)]
        {
            if self.eat(word) {
                self.expect(";")?;
                return Ok(statement);
            }
        }
        if self.at_declaration() {
            return Ok(Statement::Declare(self.declaration()?));
        }
        let expression = self.expression()?;
        self.expect(";")?;
        Ok(Statement::Expression(expression))
    }

    fn expression(&mut self) -> Result<Expression, String> {
        let mut left = self.assignment()?;
        while self.eat(",") {
            let right = self.assignment()?;
            left = Expression::Comma(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn assignment(&mut self) -> Result<Expression, String> {
        let left = self.ternary()?;
        const MARKS: [&str; 11] = ["=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>="];
        let mark = self.peek().and_then(|token| MARKS.iter().find(|mark| token.is(mark)));
        let Some(mark) = mark else {
            return Ok(left);
        };
        self.at += 1;
        let right = self.assignment()?;
        Ok(Expression::Assign(Arc::from(*mark), Box::new(left), Box::new(right)))
    }

    fn ternary(&mut self) -> Result<Expression, String> {
        let condition = self.binary(0)?;
        if !self.eat("?") {
            return Ok(condition);
        }
        let first = self.assignment()?;
        self.expect(":")?;
        let second = self.assignment()?;
        Ok(Expression::Ternary(Box::new(condition), Box::new(first), Box::new(second)))
    }

    fn binary(&mut self, level: usize) -> Result<Expression, String> {
        const LEVELS: [&[&str]; 10] = [
            &["||"],
            &["&&"],
            &["|"],
            &["^"],
            &["&"],
            &["==", "!="],
            &["<", ">", "<=", ">="],
            &["<<", ">>"],
            &["+", "-"],
            &["*", "/", "%"],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(mark) =
            self.peek().and_then(|token| LEVELS[level].iter().find(|mark| token.is(mark)))
        {
            self.at += 1;
            let right = self.binary(level + 1)?;
            left = Expression::Binary(Arc::from(*mark), Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Whether `(` opens a cast: a type, then `)`, then something to cast.
    fn at_cast(&self) -> bool {
        if !self.at_mark("(") {
            return false;
        }
        let Some(name) = self.peek_at(1) else {
            return false;
        };
        name.kind == TokenKind::Identifier
            && self.is_type_name(&name.text)
            && self.peek_at(2).is_some_and(|close| close.is(")") || close.is("["))
    }

    fn unary(&mut self) -> Result<Expression, String> {
        for mark in ["-", "+", "!", "~", "++", "--"] {
            if self.eat(mark) {
                return Ok(Expression::Unary(Arc::from(mark), Box::new(self.unary()?)));
            }
        }
        if self.at_cast() {
            self.at += 1;
            let ty = self.type_name()?;
            let dimensions = self.dimensions()?;
            self.expect(")")?;
            let ty = dimensions.iter().rev().fold(ty, |ty, dimension| {
                let length = match dimension {
                    Some(Expression::Number(text)) => text.parse().ok(),
                    _ => None,
                };
                Type::Array(Box::new(ty), length)
            });
            return Ok(Expression::Cast(ty, Box::new(self.unary()?)));
        }
        self.postfix()
    }

    fn arguments(&mut self) -> Result<Vec<Expression>, String> {
        let mut arguments = Vec::new();
        if self.eat(")") {
            return Ok(arguments);
        }
        loop {
            arguments.push(self.assignment()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(arguments)
    }

    fn postfix(&mut self) -> Result<Expression, String> {
        let mut value = self.primary()?;
        loop {
            if self.eat(".") {
                let name = self.identifier()?;
                value = if self.eat("(") {
                    Expression::Method(Box::new(value), name, self.arguments()?)
                } else {
                    Expression::Member(Box::new(value), name)
                };
            } else if self.eat("[") {
                let index = self.expression()?;
                self.expect("]")?;
                value = Expression::Index(Box::new(value), Box::new(index));
            } else if self.at_mark("++") || self.at_mark("--") {
                let mark = self.peek().expect("just peeked").text.clone();
                self.at += 1;
                value = Expression::Postfix(mark, Box::new(value));
            } else {
                return Ok(value);
            }
        }
    }

    fn primary(&mut self) -> Result<Expression, String> {
        if self.eat("(") {
            let value = self.expression()?;
            self.expect(")")?;
            return Ok(value);
        }
        let token = self.peek().ok_or("ran out of tokens")?.clone();
        match token.kind {
            TokenKind::Number => {
                self.at += 1;
                Ok(Expression::Number(token.text))
            }
            TokenKind::Identifier => {
                self.at += 1;
                if self.at_mark("(") {
                    self.at += 1;
                    let arguments = self.arguments()?;
                    return match self.is_type_name(&token.text) {
                        true => Ok(Expression::Construct(self.named_type(&token.text)?, arguments)),
                        false => Ok(Expression::Call(token.text, arguments)),
                    };
                }
                Ok(Expression::Name(token.text))
            }
            _ => Err(format!("unexpected {}", token.text)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::pre::Preprocessor;
    use super::*;

    fn parsed(source: &str) -> Program {
        let mut include = |_: &str, _: &str| None;
        let tokens = Preprocessor::new(&[], &mut include).run(source).unwrap();
        parse(&tokens).unwrap()
    }

    #[test]
    fn an_effects_techniques_name_each_passs_functions() {
        let program = parsed(concat!(
            "float4 a_ps() : SV_Target { return 0; }\nfloat4 a_vs() : SV_Position { return 0; }\n",
            "technique11 albedo <int blendMaterialCount = 2;> {\n",
            "  pass _default { SetPixelShader(CompileShader(ps_5_0, a_ps())); }\n",
            "  pass rigid { SetVertexShader(CompileShader(vs_5_0, a_vs())); SetGeometryShader(NULL); }\n",
            "}\ntechnique11 { pass water { SetVertexShader(CompileShader(vs_5_0, a_vs(1, 2))); } }\n",
        ));
        assert_eq!(program.functions.len(), 2);
        assert_eq!(&*program.pass_function("albedo", "_default", "ps").unwrap().function, "a_ps");
        assert_eq!(&*program.pass_function("albedo", "rigid", "vs").unwrap().function, "a_vs");
        // A pass names one stage only: asking another finds nothing.
        assert!(program.pass_function("albedo", "rigid", "ps").is_none());
        assert!(program.pass_function("albedo", "skinned", "vs").is_none());
        assert_eq!(program.pass_function("", "water", "vs").unwrap().arguments.len(), 2);
    }

    #[test]
    fn annotations_strings_and_sampler_state_fall_away() {
        let program = parsed(concat!(
            "static const float k = 1; string k_notes <int RegisterOffset = 3; string Name = \"k\"; float3 Default = float3(1, 1, 1);>;\n",
            "float4 tint <string Name = \"tint\"; float Minimum = 0;> = float4(1, 1, 1, 1);\n",
            "sampler s = sampler_state { Filter = MIN_MAG_MIP_LINEAR; AddressU = Wrap; };\n",
            "float f() { return k; }\n",
        ));
        let names: Vec<&str> = program.globals.iter().map(|global| &*global.name).collect();
        assert_eq!(names, ["k", "tint", "s"]);
        assert!(program.globals[1].init.is_some(), "an annotated initializer survives");
        assert!(program.globals[2].init.is_none(), "sampler state is the engine's");
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn a_typedef_can_name_one_anonymous_struct_twice() {
        let program = parsed("typedef struct { float a; int b; } pair, other_pair;\npair f(other_pair p) { return p; }\n");
        assert_eq!(program.structs.len(), 1);
        let named: Vec<_> = program.typedefs.iter().map(|(name, ty)| (&**name, ty.clone())).collect();
        assert_eq!(named[0].1, named[1].1);
        assert_eq!(named[0].0, "pair");
        assert_eq!(named[1].0, "other_pair");
        assert_eq!(program.functions[0].returns, Type::Named(program.structs[0].0.clone()));
    }

    #[test]
    fn numeric_types_read_their_shape() {
        assert_eq!(numeric_type("float"), Some(Type::Scalar(Scalar::Float)));
        assert_eq!(numeric_type("float1"), Some(Type::Scalar(Scalar::Float)));
        assert_eq!(numeric_type("int3"), Some(Type::Vector(Scalar::Int, 3)));
        assert_eq!(numeric_type("float3x4"), Some(Type::Matrix(Scalar::Float, 3, 4)));
        assert_eq!(numeric_type("min16float4"), Some(Type::Vector(Scalar::Float, 4)));
        assert_eq!(numeric_type("float5"), None);
        assert_eq!(numeric_type("floaty"), None);
    }

    #[test]
    fn a_function_reads_its_directions_and_semantics() {
        let program = parsed("struct v { float4 p : SV_Position; };\nfloat4 f(in v a, out float b, inout int c[2]) : SV_Target { return a.p; }\n");
        let function = &program.functions[0];
        let directions: Vec<_> = function.parameters.iter().map(|at| at.direction).collect();
        assert_eq!(directions, [Direction::In, Direction::Out, Direction::InOut]);
        assert_eq!(function.semantic.as_deref(), Some("SV_Target"));
        assert_eq!(function.parameters[2].variable.dimensions.len(), 1);
        assert_eq!(program.structs[0].1[0].semantic.as_deref(), Some("SV_Position"));
    }

    #[test]
    fn a_bracketed_name_is_a_cast_only_when_it_names_a_type() {
        let program = parsed("float f() { float a = (float)1; float b = (a) + 1; return a; }\n");
        let Statement::Declare(first) = &program.functions[0].body[0] else { panic!() };
        assert!(matches!(first[0].init, Some(Expression::Cast(..))));
        let Statement::Declare(second) = &program.functions[0].body[1] else { panic!() };
        assert!(matches!(second[0].init, Some(Expression::Binary(..))));
    }

    #[test]
    fn registers_buffers_and_attributes_fall_away() {
        let program = parsed("cbuffer c : register(b1) { float4 a : register(c3); float b[2]; };\n[maxvertexcount(3)] float f() { [loop] for (int i = 0; i < 2; i++) { } return 0; }\n");
        assert_eq!(program.globals.len(), 2);
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn precedence_holds() {
        let program = parsed("int f() { return 1 + 2 * 3; }\n");
        let Statement::Return(Some(Expression::Binary(mark, ..))) = &program.functions[0].body[0] else {
            panic!()
        };
        assert_eq!(&**mark, "+");
    }

    #[test]
    fn typedefs_and_templated_textures_are_types() {
        let program = parsed("typedef float3 colour;\nTexture2D<float4> t;\ncolour f() { colour c = 0; return c; }\n");
        assert_eq!(program.functions[0].returns, Type::Vector(Scalar::Float, 3));
        assert_eq!(program.globals[0].ty, Type::Texture(TextureKind::Flat));
    }
}
