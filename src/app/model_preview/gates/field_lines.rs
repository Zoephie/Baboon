//! The streaming `Debug` splitter: a line per top-level field, each field's
//! text hashed as it streams so a large preview never sits in memory whole.

use super::*;

/// A rename's names, new to old, from `BABOON_GATE_RENAMED`
/// (`New=Old,New2=Old2`): the snapshots write each new type or variant name
/// back as its old one, so a rename is checked against the baselines it
/// already had rather than blessed. `Debug` writes a name in one piece, and
/// only a whole piece is mapped.
type Renamed = std::collections::HashMap<String, String>;

fn renamed_from_env() -> &'static Renamed {
    static RENAMED: std::sync::OnceLock<Renamed> = std::sync::OnceLock::new();
    RENAMED.get_or_init(|| {
        let map = std::env::var("BABOON_GATE_RENAMED").unwrap_or_default();
        map.split(',')
            .filter_map(|pair| pair.split_once('='))
            .map(|(new, old)| (new.trim().to_owned(), old.trim().to_owned()))
            .collect()
    })
}

/// `piece` as the baselines knew it: its old name if it is a renamed one.
fn as_before<'a>(renamed: &'a Renamed, piece: &'a str) -> &'a str {
    renamed.get(piece).map_or(piece, String::as_str)
}

/// `value`'s `Debug` text, with any renamed name written as before.
pub(super) fn debug_text(value: &dyn std::fmt::Debug) -> String {
    struct Text<'a>(String, &'a Renamed);
    impl std::fmt::Write for Text<'_> {
        fn write_str(&mut self, piece: &str) -> std::fmt::Result {
            self.0.push_str(as_before(self.1, piece));
            Ok(())
        }
    }
    let mut text = Text(String::new(), renamed_from_env());
    let _ = write!(text, "{value:?}");
    text.0
}

/// FNV-1a, so a baseline means the same under any toolchain.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// One line for a value: its name, the length of its `Debug` text and that
/// text's hash.
pub(super) fn line(out: &mut String, name: &str, text: &str) {
    let _ = writeln!(out, "{name} {} {:016x}", text.len(), fnv1a(text));
}

/// Hashes a struct's compact `Debug` as it is written, a line per top-level
/// field, so a difference names the field it is in without the whole text
/// ever being held (a Halo 4 object's particles run to gigabytes). Fields are
/// found by bracket depth, outside quoted text, and only in a braced struct:
/// anything else (a tuple struct, a map) is one line under the prefix.
struct FieldLines<'a> {
    out: &'a mut String,
    /// The names to write as they were.
    renamed: &'a Renamed,
    prefix: &'a str,
    /// The field being hashed: its name, length and hash so far.
    field: Option<(String, usize, u64)>,
    /// Brackets open, counting the struct's own.
    depth: usize,
    quoted: bool,
    escaped: bool,
    /// At the struct's own level, a field's name as it is read, up to `: `.
    naming: Option<String>,
    /// The value opened with `{` after a type's name, so is a struct with
    /// named fields (a bare map opens with `{` too).
    braced: bool,
    /// What came before the value's first bracket.
    named: String,
    /// Every byte, for a value that is not a braced struct.
    whole: (usize, u64),
    /// Whether any field's line was written.
    split: bool,
}

impl FieldLines<'_> {
    fn finish_field(&mut self) {
        if let Some((name, length, hash)) = self.field.take() {
            let _ = writeln!(self.out, "{}.{name} {length} {hash:016x}", self.prefix);
            self.split = true;
        }
    }

    /// The last field's line, or the whole value's if it had none.
    fn close(mut self) {
        self.finish_field();
        if !self.split {
            let (length, hash) = self.whole;
            let _ = writeln!(self.out, "{} {length} {hash:016x}", self.prefix);
        }
    }

    fn step(&mut self, byte: u8) {
        let (length, hash) = &mut self.whole;
        *length += 1;
        *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        if let Some(name) = &mut self.naming {
            if byte == b' ' && name.ends_with(':') {
                name.pop();
                let name = self.naming.take().unwrap_or_default().trim().to_owned();
                self.finish_field();
                self.field = Some((name, 0, 0xcbf2_9ce4_8422_2325));
            } else {
                name.push(char::from(byte));
            }
            return;
        }
        if let Some((_, length, hash)) = &mut self.field {
            *length += 1;
            *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        if self.quoted {
            match byte {
                _ if self.escaped => self.escaped = false,
                b'\\' => self.escaped = true,
                b'"' => self.quoted = false,
                _ => {}
            }
            return;
        }
        if self.depth == 0 && self.named.len() < 128 && !matches!(byte, b'{' | b'[' | b'(') {
            self.named.push(char::from(byte));
        }
        match byte {
            b'"' => self.quoted = true,
            b'{' | b'[' | b'(' => {
                self.depth += 1;
                let name = self.named.trim();
                let struct_name =
                    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
                if self.depth == 1 && byte == b'{' && struct_name {
                    self.braced = true;
                    self.naming = Some(String::new());
                }
            }
            b'}' | b']' | b')' => self.depth = self.depth.saturating_sub(1),
            b',' if self.depth == 1 && self.braced => self.naming = Some(String::new()),
            _ => {}
        }
    }
}

impl std::fmt::Write for FieldLines<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        as_before(self.renamed, text)
            .bytes()
            .for_each(|byte| self.step(byte));
        Ok(())
    }
}

impl<'a> FieldLines<'a> {
    fn new(out: &'a mut String, prefix: &'a str, renamed: &'a Renamed) -> Self {
        FieldLines {
            out,
            renamed,
            prefix,
            field: None,
            depth: 0,
            quoted: false,
            escaped: false,
            naming: None,
            braced: false,
            named: String::new(),
            whole: (0, 0xcbf2_9ce4_8422_2325),
            split: false,
        }
    }
}

/// A line per top-level field of `value`'s `Debug`.
pub(in crate::app::model_preview) fn field_lines(
    out: &mut String,
    prefix: &str,
    value: &impl std::fmt::Debug,
) {
    field_lines_renamed(out, prefix, value, renamed_from_env());
}

fn field_lines_renamed(
    out: &mut String,
    prefix: &str,
    value: &dyn std::fmt::Debug,
    renamed: &Renamed,
) {
    let mut lines = FieldLines::new(out, prefix, renamed);
    let _ = write!(lines, "{value:?}");
    lines.close();
}

/// A line per field of a struct that is itself a field, read as though its
/// fields were its holder's own: its closing ` }` becomes the `,` that would
/// follow them there, or stays for the holder's `last` field.
pub(super) fn inline_field_lines(
    out: &mut String,
    prefix: &str,
    value: &dyn std::fmt::Debug,
    last: bool,
) {
    /// The bytes but the last two, which are the struct's ` }`.
    struct Held<'a> {
        lines: FieldLines<'a>,
        held: std::collections::VecDeque<u8>,
    }
    impl std::fmt::Write for Held<'_> {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            for byte in as_before(self.lines.renamed, text).bytes() {
                self.held.push_back(byte);
                if self.held.len() > 2 {
                    let byte = self.held.pop_front().unwrap_or_default();
                    self.lines.step(byte);
                }
            }
            Ok(())
        }
    }
    let mut held = Held {
        lines: FieldLines::new(out, prefix, renamed_from_env()),
        held: std::collections::VecDeque::new(),
    };
    let _ = write!(held, "{value:?}");
    let Held { mut lines, held } = held;
    let ending: Vec<u8> = if held.iter().eq(b" }") && !last {
        b",".to_vec()
    } else {
        held.into()
    };
    ending.into_iter().for_each(|byte| lines.step(byte));
    lines.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_lines_name_each_top_level_field() {
        #[derive(Debug)]
        #[allow(dead_code, reason = "read through Debug")]
        struct Inner {
            deep: u8,
        }
        #[derive(Debug)]
        #[allow(dead_code, reason = "read through Debug")]
        struct Outer {
            first: Vec<u8>,
            quoted: &'static str,
            second: Inner,
        }
        let outer = |deep| Outer {
            first: vec![1, 2],
            quoted: "a \\\"}, fake: {",
            second: Inner { deep },
        };
        let mut out = String::new();
        field_lines(&mut out, "p", &outer(3));
        let names: Vec<_> = out
            .lines()
            .map(|row| row.split(' ').next().unwrap())
            .collect();
        assert_eq!(names, ["p.first", "p.quoted", "p.second"], "{out}");
        let mut changed = String::new();
        field_lines(&mut changed, "p", &outer(4));
        assert_eq!(
            out.lines().take(2).collect::<Vec<_>>(),
            changed.lines().take(2).collect::<Vec<_>>(),
            "untouched fields keep their lines"
        );
        assert_ne!(
            out.lines().nth(2),
            changed.lines().nth(2),
            "a nested change moves its field's line"
        );
        // A tuple struct or a bare map is one line under the prefix.
        #[derive(Debug)]
        struct Wrapped(
            #[allow(dead_code, reason = "read through Debug")] std::collections::BTreeMap<u8, u8>,
        );
        let wrapped = Wrapped([(1, 2)].into());
        let map = std::collections::BTreeMap::from([(3, 4)]);
        for value in [&wrapped as &dyn std::fmt::Debug, &map] {
            let mut out = String::new();
            field_lines(&mut out, "w", &value);
            assert_eq!(out.lines().count(), 1, "{out}");
            assert!(out.starts_with("w "), "{out}");
        }
    }

    /// Under a rename's map, a renamed type nested in a field hashes as its
    /// old name's did; without the map, the new name shows.
    #[test]
    fn a_renamed_type_hashes_as_its_old_name_under_the_map() {
        #[derive(Debug)]
        #[allow(dead_code, reason = "read through Debug")]
        struct Image {
            width: u8,
        }
        #[derive(Debug)]
        #[allow(dead_code, reason = "read through Debug")]
        struct H2Image {
            width: u8,
        }
        #[derive(Debug)]
        #[allow(dead_code, reason = "read through Debug")]
        struct Holder<T> {
            image: T,
            label: &'static str,
        }
        let renamed: Renamed = [("Image".to_owned(), "H2Image".to_owned())].into();
        let lines = |value: &dyn std::fmt::Debug, renamed: &Renamed| {
            let mut out = String::new();
            field_lines_renamed(&mut out, "p", value, renamed);
            out
        };
        let old = lines(
            &Holder {
                image: H2Image { width: 4 },
                label: "Images",
            },
            &Renamed::new(),
        );
        let new = Holder {
            image: Image { width: 4 },
            label: "Images",
        };
        assert_ne!(
            lines(&new, &Renamed::new()),
            old,
            "without the map the new name shows"
        );
        assert_eq!(
            lines(&new, &renamed),
            old,
            "under the map it reads as before"
        );
    }
}
