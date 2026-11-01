//! The C preprocessor HLSL inherits, working in tokens.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// How deep `#include`s may go before a cycle is assumed.
const MAX_INCLUDE_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Identifier,
    Number,
    Text,
    Punctuation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) text: Arc<str>,
}

impl Token {
    fn new(kind: TokenKind, text: &str) -> Self {
        Self { kind, text: Arc::from(text) }
    }

    pub(crate) fn is(&self, text: &str) -> bool {
        self.kind != TokenKind::Text && &*self.text == text
    }
}

/// Punctuation, longest first so a greedy match is the right one.
const PUNCTUATION: [&str; 39] = [
    "<<=", ">>=", "##", "++", "--", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "+=", "-=", "*=",
    "/=", "%=", "&=", "|=", "^=", "::", "(", ")", "[", "]", "{", "}", ";", ",", ":", "?", "=", "<", ">",
    "+", "-", "*", "/",
];

/// Comments gone and continued lines joined, as logical lines.
fn logical_lines(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut text = Vec::with_capacity(bytes.len());
    let mut at = 0;
    let mut quoted = false;
    while at < bytes.len() {
        let byte = bytes[at];
        if quoted {
            quoted = byte != b'"' && byte != b'\n';
        } else if byte == b'"' {
            quoted = true;
        } else if byte == b'/' && bytes.get(at + 1) == Some(&b'/') {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        } else if byte == b'/' && bytes.get(at + 1) == Some(&b'*') {
            at += 2;
            while at + 1 < bytes.len() && !(bytes[at] == b'*' && bytes[at + 1] == b'/') {
                at += 1;
            }
            at += 2;
            text.push(b' ');
            continue;
        }
        if byte == b'\\' {
            let mut next = at + 1;
            while bytes.get(next).is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r')) {
                next += 1;
            }
            if bytes.get(next) == Some(&b'\n') {
                at = next + 1;
                text.push(b' ');
                continue;
            }
        }
        if byte != b'\r' {
            text.push(byte);
        }
        at += 1;
    }
    String::from_utf8_lossy(&text).split('\n').map(str::to_owned).collect()
}

pub(crate) fn tokenize(line: &str) -> Vec<Token> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte.is_ascii_whitespace() {
            at += 1;
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = at;
            while bytes.get(at).is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_') {
                at += 1;
            }
            tokens.push(Token::new(TokenKind::Identifier, &line[start..at]));
        } else if byte.is_ascii_digit()
            || (byte == b'.' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit))
        {
            let start = at;
            while bytes.get(at).is_some_and(|byte| {
                byte.is_ascii_alphanumeric()
                    || *byte == b'.'
                    || (matches!(*byte, b'+' | b'-')
                        && matches!(bytes[at - 1], b'e' | b'E')
                        && !line[start..at].starts_with("0x"))
            }) {
                at += 1;
            }
            tokens.push(Token::new(TokenKind::Number, &line[start..at]));
        } else if byte == b'"' || byte == b'<' && tokens.last().is_some_and(|last: &Token| last.is("include")) {
            let close = if byte == b'"' { '"' } else { '>' };
            let end = line[at + 1..].find(close).map_or(bytes.len(), |end| at + 1 + end);
            tokens.push(Token::new(TokenKind::Text, &line[at + 1..end]));
            at = end + 1;
        } else if let Some(mark) = PUNCTUATION.iter().find(|mark| line[at..].starts_with(**mark)) {
            tokens.push(Token::new(TokenKind::Punctuation, mark));
            at += mark.len();
        } else {
            let end = (at + 1..=line.len()).find(|end| line.is_char_boundary(*end)).unwrap_or(line.len());
            tokens.push(Token::new(TokenKind::Punctuation, &line[at..end]));
            at = end;
        }
    }
    tokens
}

#[derive(Debug, Clone)]
struct Macro {
    parameters: Option<Vec<Arc<str>>>,
    body: Vec<Token>,
}

/// A token and the macros that may no longer expand in it.
#[derive(Clone)]
struct Painted {
    token: Token,
    hidden: Arc<Vec<Arc<str>>>,
}

fn painted(tokens: &[Token]) -> VecDeque<Painted> {
    let hidden = Arc::new(Vec::new());
    tokens.iter().map(|token| Painted { token: token.clone(), hidden: Arc::clone(&hidden) }).collect()
}

/// One conditional's state: whether this branch reads, whether one has yet,
/// and whether the conditional sits in live code at all.
struct Conditional {
    active: bool,
    taken: bool,
    outer: bool,
}

pub(crate) struct Preprocessor<'a> {
    macros: HashMap<Arc<str>, Macro>,
    include: &'a mut dyn FnMut(&str) -> Option<Arc<str>>,
    output: Vec<Token>,
}

impl<'a> Preprocessor<'a> {
    pub(crate) fn new(
        defines: &[(&str, &str)],
        include: &'a mut dyn FnMut(&str) -> Option<Arc<str>>,
    ) -> Self {
        let mut macros = HashMap::new();
        for (name, value) in defines {
            macros.insert(Arc::from(*name), Macro { parameters: None, body: tokenize(value) });
        }
        Self { macros, include, output: Vec::new() }
    }

    pub(crate) fn run(mut self, source: &str) -> Result<Vec<Token>, String> {
        self.file(source, 0)?;
        Ok(self.output)
    }

    fn file(&mut self, source: &str, depth: usize) -> Result<(), String> {
        if depth > MAX_INCLUDE_DEPTH {
            return Err("includes nest too deeply".to_owned());
        }
        let mut conditionals: Vec<Conditional> = Vec::new();
        let mut pending: Vec<Token> = Vec::new();
        for line in logical_lines(source) {
            let active = conditionals.last().is_none_or(|conditional| conditional.active);
            let trimmed = line.trim_start();
            let Some(directive) = trimmed.strip_prefix('#') else {
                if active {
                    pending.extend(tokenize(&line));
                }
                continue;
            };
            let directive = directive.trim_start();
            let word_end = directive
                .find(|character: char| !character.is_ascii_alphanumeric())
                .unwrap_or(directive.len());
            let (word, rest) = directive.split_at(word_end);
            match word {
                "ifdef" | "ifndef" | "if" => {
                    let taken = active && {
                        match word {
                            "if" => self.condition(rest)?,
                            _ => {
                                let name = rest.trim();
                                self.macros.contains_key(name) == (word == "ifdef")
                            }
                        }
                    };
                    conditionals.push(Conditional { active: taken, taken, outer: active });
                }
                "elif" => {
                    let conditional = conditionals.last().ok_or("#elif outside a conditional")?;
                    let (outer, taken) = (conditional.outer, conditional.taken);
                    let now = outer && !taken && self.condition(rest)?;
                    let conditional = conditionals.last_mut().ok_or("#elif outside a conditional")?;
                    conditional.active = now;
                    conditional.taken |= now;
                }
                "else" => {
                    let conditional = conditionals.last_mut().ok_or("#else outside a conditional")?;
                    conditional.active = conditional.outer && !conditional.taken;
                    conditional.taken = true;
                }
                "endif" => {
                    conditionals.pop().ok_or("#endif outside a conditional")?;
                }
                _ if !active => {}
                "define" => {
                    self.flush(&mut pending)?;
                    self.define(rest)?;
                }
                "undef" => {
                    self.flush(&mut pending)?;
                    self.macros.remove(rest.trim());
                }
                "include" => {
                    self.flush(&mut pending)?;
                    let name = tokenize(&format!("#include {}", rest.trim()));
                    let name = name.last().ok_or("#include names nothing")?.text.clone();
                    let text = (self.include)(&name).ok_or_else(|| format!("no include {name}"))?;
                    self.file(&text, depth + 1)?;
                }
                "error" => return Err(format!("#error{rest}")),
                _ => {}
            }
        }
        self.flush(&mut pending)
    }

    fn flush(&mut self, pending: &mut Vec<Token>) -> Result<(), String> {
        let expanded = self.expand(painted(pending))?;
        self.output.extend(expanded);
        pending.clear();
        Ok(())
    }

    fn define(&mut self, rest: &str) -> Result<(), String> {
        let rest = rest.trim_start();
        let end = rest
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .unwrap_or(rest.len());
        let (name, after) = rest.split_at(end);
        if name.is_empty() {
            return Err("#define names nothing".to_owned());
        }
        // A parameter list opens right on the name; a space means a body.
        let (parameters, body) = match after.strip_prefix('(') {
            Some(list) => {
                let close = list.find(')').ok_or_else(|| format!("#define {name} never closes"))?;
                let names = list[..close]
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(Arc::from)
                    .collect();
                (Some(names), &list[close + 1..])
            }
            None => (None, after),
        };
        self.macros.insert(Arc::from(name), Macro { parameters, body: tokenize(body) });
        Ok(())
    }

    /// Prosser's algorithm: each token remembers which macros made it.
    fn expand(&self, mut input: VecDeque<Painted>) -> Result<Vec<Token>, String> {
        let mut output = Vec::with_capacity(input.len());
        while let Some(next) = input.pop_front() {
            let name = &next.token.text;
            let definition = (next.token.kind == TokenKind::Identifier
                && !next.hidden.iter().any(|hidden| hidden == name))
            .then(|| self.macros.get(name))
            .flatten();
            let Some(definition) = definition else {
                output.push(next.token);
                continue;
            };
            let mut hidden = (*next.hidden).clone();
            hidden.push(name.clone());
            let hidden = Arc::new(hidden);
            let body = match &definition.parameters {
                None => paste(definition.body.clone()),
                Some(parameters) => {
                    if !input.front().is_some_and(|front| front.token.is("(")) {
                        output.push(next.token);
                        continue;
                    }
                    input.pop_front();
                    let arguments = collect_arguments(&mut input, name)?;
                    self.substitute(definition, parameters, &arguments, name)?
                }
            };
            for token in body.into_iter().rev() {
                input.push_front(Painted { token, hidden: Arc::clone(&hidden) });
            }
        }
        Ok(output)
    }

    fn substitute(
        &self,
        definition: &Macro,
        parameters: &[Arc<str>],
        arguments: &[Vec<Painted>],
        name: &str,
    ) -> Result<Vec<Token>, String> {
        if arguments.len() < parameters.len() && !(parameters.len() == 1 && arguments.is_empty()) {
            return Err(format!("{name} takes {} arguments", parameters.len()));
        }
        let mut body = Vec::with_capacity(definition.body.len());
        for token in &definition.body {
            let position = (token.kind == TokenKind::Identifier)
                .then(|| parameters.iter().position(|parameter| *parameter == token.text))
                .flatten();
            let Some(position) = position else {
                body.push(token.clone());
                continue;
            };
            let argument = arguments.get(position).map_or(&[][..], Vec::as_slice);
            // fxc expands an argument even beside `##`, unlike C, and the
            // kit relies on it: `CALC_MATERIAL(material_type)`.
            body.extend(self.expand(argument.iter().cloned().collect())?);
            if argument.is_empty() {
                body.push(Token::new(TokenKind::Text, ""));
            }
        }
        Ok(paste(body))
    }

    fn condition(&self, text: &str) -> Result<bool, String> {
        let tokens = tokenize(text);
        let mut resolved = Vec::with_capacity(tokens.len());
        let mut at = 0;
        while at < tokens.len() {
            if tokens[at].is("defined") {
                let bracketed = tokens.get(at + 1).is_some_and(|token| token.is("("));
                let name = tokens.get(at + if bracketed { 2 } else { 1 }).ok_or("defined of nothing")?;
                let found = self.macros.contains_key(&name.text);
                resolved.push(Token::new(TokenKind::Number, if found { "1" } else { "0" }));
                at += if bracketed { 4 } else { 2 };
            } else {
                resolved.push(tokens[at].clone());
                at += 1;
            }
        }
        let expanded = self.expand(painted(&resolved))?;
        let mut reader = ConditionReader { tokens: &expanded, at: 0 };
        Ok(reader.ternary()? != 0)
    }
}

fn collect_arguments(input: &mut VecDeque<Painted>, name: &str) -> Result<Vec<Vec<Painted>>, String> {
    let mut arguments = vec![Vec::new()];
    let mut depth = 0;
    loop {
        let Some(next) = input.pop_front() else {
            return Err(format!("{name}( never closes"));
        };
        if next.token.is("(") {
            depth += 1;
        } else if next.token.is(")") {
            if depth == 0 {
                break;
            }
            depth -= 1;
        } else if next.token.is(",") && depth == 0 {
            arguments.push(Vec::new());
            continue;
        }
        arguments.last_mut().expect("starts with one").push(next);
    }
    if arguments.len() == 1 && arguments[0].is_empty() {
        arguments.clear();
    }
    Ok(arguments)
}

/// Apply `##`. An empty argument leaves an empty text token to join.
fn paste(tokens: Vec<Token>) -> Vec<Token> {
    let mut output: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut joining = false;
    for token in tokens {
        if token.is("##") {
            joining = true;
        } else if joining {
            joining = false;
            match output.pop() {
                Some(last) => {
                    let text = format!("{}{}", last.text, token.text);
                    output.extend(tokenize(&text));
                }
                None => output.push(token),
            }
        } else {
            output.push(token);
        }
    }
    output.retain(|token| !(token.kind == TokenKind::Text && token.text.is_empty()));
    output
}

/// `#if` arithmetic. Any name left after expansion reads 0.
struct ConditionReader<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl ConditionReader<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn eat(&mut self, text: &str) -> bool {
        let found = self.peek().is_some_and(|token| token.is(text));
        self.at += usize::from(found);
        found
    }

    fn ternary(&mut self) -> Result<i64, String> {
        let condition = self.binary(0)?;
        if !self.eat("?") {
            return Ok(condition);
        }
        let first = self.ternary()?;
        if !self.eat(":") {
            return Err("#if ternary has no :".to_owned());
        }
        let second = self.ternary()?;
        Ok(if condition != 0 { first } else { second })
    }

    fn binary(&mut self, level: usize) -> Result<i64, String> {
        const LEVELS: [&[&str]; 8] = [
            &["||"],
            &["&&"],
            &["|"],
            &["&"],
            &["==", "!="],
            &["<", ">", "<=", ">="],
            &["+", "-"],
            &["*", "/", "%"],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(mark) =
            self.peek().and_then(|token| LEVELS[level].iter().copied().find(|mark| token.is(mark)))
        {
            self.at += 1;
            let right = self.binary(level + 1)?;
            left = match mark {
                "||" => i64::from(left != 0 || right != 0),
                "&&" => i64::from(left != 0 && right != 0),
                "|" => left | right,
                "&" => left & right,
                "==" => i64::from(left == right),
                "!=" => i64::from(left != right),
                "<" => i64::from(left < right),
                ">" => i64::from(left > right),
                "<=" => i64::from(left <= right),
                ">=" => i64::from(left >= right),
                "+" => left + right,
                "-" => left - right,
                "*" => left * right,
                "/" => left.checked_div(right).ok_or("#if divides by zero")?,
                _ => left.checked_rem(right).ok_or("#if divides by zero")?,
            };
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<i64, String> {
        if self.eat("!") {
            return Ok(i64::from(self.unary()? == 0));
        }
        if self.eat("-") {
            return Ok(-self.unary()?);
        }
        if self.eat("(") {
            let value = self.ternary()?;
            if !self.eat(")") {
                return Err("#if bracket never closes".to_owned());
            }
            return Ok(value);
        }
        let token = self.peek().ok_or("#if ends early")?;
        let value = match token.kind {
            TokenKind::Number => number_value(&token.text).unwrap_or(0),
            _ => 0,
        };
        self.at += 1;
        Ok(value)
    }
}

/// An integer literal's value, with its suffixes and in its base.
pub(crate) fn number_value(text: &str) -> Option<i64> {
    let digits = text.trim_end_matches(['u', 'U', 'l', 'L']);
    match digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok(),
        None => digits.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(source: &str, defines: &[(&str, &str)]) -> String {
        let mut include = |name: &str| (name == "inner.fx").then(|| Arc::from("int from_inner;\n"));
        let tokens = Preprocessor::new(defines, &mut include).run(source).unwrap();
        tokens.iter().map(|token| &*token.text).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn pasting_names_a_macro_that_then_expands() {
        let source = "#define VERTEX_TYPE_s_world 0\n#define TYPE(t) VERTEX_TYPE_##t\nTYPE(s_world)\n";
        assert_eq!(text(source, &[]), "0");
    }

    #[test]
    fn arguments_expand_even_beside_a_paste_as_in_fxc() {
        let source = "#define kind fancy\n#define A(x) calc_##x##_ps\nA(kind)\n";
        assert_eq!(text(source, &[]), "calc_fancy_ps");
    }

    #[test]
    fn a_macro_does_not_expand_inside_itself() {
        assert_eq!(text("#define f(x) f(x + 1)\nf(2)\n", &[]), "f ( 2 + 1 )");
    }

    #[test]
    fn a_function_macro_can_find_its_bracket_after_the_expansion() {
        let source = "#define CALL(t) calc_##t\n#define calc_a(x) real(x)\nCALL(a)(1)\n";
        assert_eq!(text(source, &[]), "real ( 1 )");
    }

    #[test]
    fn a_name_without_a_bracket_is_left_alone() {
        assert_eq!(text("#define f(x) x\nf ;\n", &[]), "f ;");
    }

    #[test]
    fn conditionals_nest_and_skip_dead_branches() {
        let source = "#if DX == 11 && defined(pc)\na\n#ifndef pc\nb\n#else\nc\n#endif\n#elif 1\nd\n#else\ne\n#endif\n";
        assert_eq!(text(source, &[("DX", "11"), ("pc", "1")]), "a c");
        assert_eq!(text(source, &[("DX", "9")]), "d");
    }

    #[test]
    fn a_dead_branch_can_hold_an_error_and_an_unknown_include() {
        assert_eq!(text("#if 0\n#error no\n#include \"gone.fx\"\n#endif\nx\n", &[]), "x");
    }

    #[test]
    fn includes_are_read_in_place() {
        assert_eq!(text("a\n#include \"inner.fx\"\nb\n", &[]), "a int from_inner ; b");
    }

    #[test]
    fn comments_and_continuations_vanish() {
        let source = "#define A 1 \\\n 2 // gone\nA /* also\n gone */ 3\n";
        assert_eq!(text(source, &[]), "1 2 3");
    }

    #[test]
    fn numbers_keep_their_exponents_and_suffixes() {
        let tokens = tokenize("1e-2f+.5 0x1F");
        let texts: Vec<_> = tokens.iter().map(|token| &*token.text).collect();
        assert_eq!(texts, ["1e-2f", "+", ".5", "0x1F"]);
    }
}
