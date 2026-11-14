//! Just enough of HaloScript's syntax to find tag paths and what each one is
//! passed to: lists, atoms, strings and comments. No evaluation.

/// Where a path-like piece of script text sits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Place {
    /// Argument `index` (from 0) of a call to `function`.
    Argument { function: String, index: usize },
    /// The value of `(global <value_type> name value)`.
    GlobalValue { value_type: String },
    /// Code whose expected type cannot be read from its surroundings: the
    /// head of a list, a script body, a list inside a list.
    Unknown,
    /// Inside a comment, or a run within a longer string. Never compiled as
    /// a tag path.
    Text,
}

/// A run of script text that could be a tag path: it holds a `\` or `/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PathToken {
    /// Byte range in the script.
    pub start: usize,
    pub end: usize,
    /// 1-based line.
    pub line: usize,
    pub place: Place,
}

impl PathToken {
    pub fn text<'a>(&self, script: &'a [u8]) -> &'a [u8] {
        &script[self.start..self.end]
    }
}

#[derive(Default)]
struct Frame {
    /// The list's first atom, lowercased; `Some("")` when the list starts
    /// with something other than an atom.
    head: Option<String>,
    /// How many arguments (elements after the head) have been seen.
    arguments: usize,
    /// For `(global <type> …)`: the declared type.
    global_type: Option<String>,
}

impl Frame {
    /// Register one element and say where it sits: `None` for the head.
    fn element(&mut self, atom: Option<&str>) -> Option<Place> {
        let Some(head) = &self.head else {
            self.head = Some(atom.map(str::to_ascii_lowercase).unwrap_or_default());
            return None;
        };
        let index = self.arguments;
        self.arguments += 1;
        let place = match head.as_str() {
            "" => Place::Unknown,
            "global" => match index {
                0 => {
                    self.global_type = atom.map(str::to_ascii_lowercase);
                    Place::Unknown
                }
                2 => match &self.global_type {
                    Some(value_type) => Place::GlobalValue { value_type: value_type.clone() },
                    None => Place::Unknown,
                },
                _ => Place::Unknown,
            },
            // Declarations: their bodies are any expression.
            "script" => Place::Unknown,
            function => Place::Argument { function: function.to_owned(), index },
        };
        Some(place)
    }
}

fn is_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'\\' | b'/' | b'\'' | b'!' | b'&' | b'+' | b'@' | b'#' | b'$' | b'%' | b'~')
}

fn has_separator(bytes: &[u8]) -> bool {
    bytes.iter().any(|byte| matches!(byte, b'\\' | b'/'))
}

/// Path-like runs within free text (a comment, or a string that is more than
/// one path), as [`Place::Text`].
fn text_runs(script: &[u8], start: usize, end: usize, line_at: &dyn Fn(usize) -> usize, out: &mut Vec<PathToken>) {
    let mut at = start;
    while at < end {
        if !is_path_byte(script[at]) {
            at += 1;
            continue;
        }
        let run_start = at;
        while at < end && is_path_byte(script[at]) {
            at += 1;
        }
        // A sentence's full stop is not part of the path.
        let mut run_end = at;
        while run_end > run_start && script[run_end - 1] == b'.' {
            run_end -= 1;
        }
        if has_separator(&script[run_start..run_end]) {
            out.push(PathToken { start: run_start, end: run_end, line: line_at(run_start), place: Place::Text });
        }
    }
}

/// Every path-like run in `script`, in order.
pub(super) fn path_tokens(script: &[u8]) -> Vec<PathToken> {
    // Line starts, for line numbers.
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(script.iter().enumerate().filter(|(_, byte)| **byte == b'\n').map(|(at, _)| at + 1))
        .collect();
    let line_at = |offset: usize| line_starts.partition_point(|start| *start <= offset);

    let mut out = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut at = 0;
    let len = script.len();
    while at < len {
        match script[at] {
            b';' => {
                let block = script.get(at + 1) == Some(&b'*');
                let start = at + if block { 2 } else { 1 };
                let end = if block {
                    script[start..]
                        .windows(2)
                        .position(|pair| pair == b"*;")
                        .map_or(len, |found| start + found)
                } else {
                    script[start..].iter().position(|byte| *byte == b'\n').map_or(len, |found| start + found)
                };
                text_runs(script, start, end, &line_at, &mut out);
                at = if block { (end + 2).min(len) } else { end };
            }
            b'(' => {
                if let Some(parent) = stack.last_mut() {
                    parent.element(None);
                }
                stack.push(Frame::default());
                at += 1;
            }
            b')' => {
                stack.pop();
                at += 1;
            }
            b'"' => {
                let start = at + 1;
                let end = script[start..].iter().position(|byte| *byte == b'"').map_or(len, |found| start + found);
                let place = stack.last_mut().map_or(Some(Place::Unknown), |frame| frame.element(None));
                let inner = &script[start..end];
                if has_separator(inner) {
                    if inner.iter().all(|byte| is_path_byte(*byte)) {
                        out.push(PathToken { start, end, line: line_at(start), place: place.unwrap_or(Place::Unknown) });
                    } else {
                        text_runs(script, start, end, &line_at, &mut out);
                    }
                }
                at = (end + 1).min(len);
            }
            byte if byte.is_ascii_whitespace() => at += 1,
            _ => {
                let start = at;
                while at < len && !script[at].is_ascii_whitespace() && !matches!(script[at], b'(' | b')' | b'"' | b';') {
                    at += 1;
                }
                let atom = &script[start..at];
                let text = std::str::from_utf8(atom).ok();
                let place = stack.last_mut().map_or(Some(Place::Unknown), |frame| frame.element(text));
                if has_separator(atom) {
                    out.push(PathToken { start, end: at, line: line_at(start), place: place.unwrap_or(Place::Unknown) });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(script: &str) -> Vec<(String, usize, Place)> {
        path_tokens(script.as_bytes())
            .into_iter()
            .map(|token| (String::from_utf8_lossy(token.text(script.as_bytes())).into_owned(), token.line, token.place))
            .collect()
    }

    fn argument(function: &str, index: usize) -> Place {
        Place::Argument { function: function.to_owned(), index }
    }

    #[test]
    fn quoted_and_bare_paths_know_the_call_and_position_they_sit_in() {
        let script = "(script dormant hunters\n\t(custom_animation (ai_get_unit x) \"objects\\characters\\hunter\\hunter\" \"shake\" true)\n\t(render_model_predict objects\\characters\\elite\\elite 25))";
        assert_eq!(
            places(script),
            [
                ("objects\\characters\\hunter\\hunter".to_owned(), 2, argument("custom_animation", 1)),
                ("objects\\characters\\elite\\elite".to_owned(), 3, argument("render_model_predict", 0)),
            ]
        );
    }

    #[test]
    fn a_global_value_takes_the_declared_type() {
        assert_eq!(
            places("(global object_definition grunt \"objects\\characters\\grunt\\grunt\")"),
            [("objects\\characters\\grunt\\grunt".to_owned(), 1, Place::GlobalValue { value_type: "object_definition".into() })]
        );
    }

    #[test]
    fn comments_and_sentences_are_text() {
        let script = "; uses objects\\characters\\grunt\\grunt.\n;* block\n mentions objects/a/b *;\n(print \"move objects\\x\\y now\")";
        assert_eq!(
            places(script),
            [
                ("objects\\characters\\grunt\\grunt".to_owned(), 1, Place::Text),
                ("objects/a/b".to_owned(), 3, Place::Text),
                ("objects\\x\\y".to_owned(), 4, Place::Text),
            ]
        );
    }

    #[test]
    fn a_nested_call_takes_up_its_argument_position() {
        assert_eq!(
            places("(object_create_anew_containing (foo) \"a\\b\")"),
            [("a\\b".to_owned(), 1, argument("object_create_anew_containing", 1))]
        );
    }
}
