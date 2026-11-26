//! Themes: every colour Baboon paints, named as a [`Token`], with one value
//! per token in each theme. The accessors in `style.rs` read the installed
//! theme through [`color`]; a theme is installed as a whole, on the UI thread,
//! before anything draws.
//!
//! The table lives in `table.rs`, one row per token: its id in a theme file,
//! a sentence saying where it appears, and its value in each built-in theme.

use eframe::egui::Color32;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[cfg(test)]
mod golden;
#[cfg(test)]
mod tests_files;

/// A row's rules, or none.
macro_rules! rules_or_none {
    () => {
        super::rules::Rules::NONE
    };
    ($rules:expr) => {
        $rules
    };
}

/// Declares the tokens: the [`Token`] enum, its ids and descriptions, the
/// built-in columns, and the installed theme's storage, which starts as the
/// light built-in, as Baboon did before it read its preferences.
macro_rules! tokens {
    ($( group $group:literal { $( $variant:ident $id:literal [$dark:literal, $light:literal, $classic:literal] $description:literal $(=> $rules:expr)?; )* } )*) => {
        /// A named colour the UI asks for.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub(in crate::app) enum Token {
            $($($variant,)*)*
        }

        impl Token {
            /// Every token, in the order the table declares them.
            pub(in crate::app) const ALL: &'static [Token] = &[$($(Token::$variant,)*)*];

            /// The id a theme file names it by.
            pub(in crate::app) const fn id(self) -> &'static str {
                match self {
                    $($(Token::$variant => $id,)*)*
                }
            }

            /// One sentence saying where it appears.
            pub(in crate::app) const fn description(self) -> &'static str {
                match self {
                    $($(Token::$variant => $description,)*)*
                }
            }

            /// How it follows the tokens it is derived from, per appearance.
            pub(in crate::app) fn rules(self) -> super::rules::Rules {
                #[allow(unused_imports)]
                use super::rules::*;
                use Token::*;
                match self {
                    $($($variant => rules_or_none!($($rules)?),)*)*
                }
            }

            /// The group the table and the authoring guide list it under.
            pub(in crate::app) const fn group(self) -> &'static str {
                match self {
                    $($(Token::$variant => $group,)*)*
                }
            }

            /// The token a theme file's id names, if any.
            pub(in crate::app) fn from_id(id: &str) -> Option<Token> {
                match id {
                    $($($id => Some(Token::$variant),)*)*
                    _ => None,
                }
            }
        }

        /// How many tokens there are.
        pub(in crate::app) const TOKEN_COUNT: usize = [$($($id,)*)*].len();

        pub(super) const DARK: [::eframe::egui::Color32; TOKEN_COUNT] = [$($(super::hex($dark),)*)*];
        pub(super) const LIGHT: [::eframe::egui::Color32; TOKEN_COUNT] = [$($(super::hex($light),)*)*];
        pub(super) const CLASSIC_LIGHT: [::eframe::egui::Color32; TOKEN_COUNT] = [$($(super::hex($classic),)*)*];

        /// The installed theme, one packed colour per token.
        pub(super) static INSTALLED: [::std::sync::atomic::AtomicU32; TOKEN_COUNT] =
            [$($(::std::sync::atomic::AtomicU32::new(super::pack(super::hex($light))),)*)*];
    };
}

mod table;
pub(in crate::app) use table::{TOKEN_COUNT, Token};

pub(in crate::app) mod export;
pub(in crate::app) mod file;
pub(in crate::app) mod library;
pub(in crate::app) mod rules;
pub(in crate::app) mod state;

/// A colour written as `#RRGGBB` or `#RRGGBBAA`, with straight alpha as in
/// CSS and in theme files, at compile time.
pub(in crate::app) const fn hex(text: &str) -> Color32 {
    let bytes = text.as_bytes();
    assert!(
        bytes.len() == 7 || bytes.len() == 9,
        "a colour is #RRGGBB or #RRGGBBAA"
    );
    assert!(bytes[0] == b'#', "a colour starts with #");
    const fn digit(byte: u8) -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => panic!("not a hex digit"),
        }
    }
    const fn pair(bytes: &[u8], at: usize) -> u8 {
        digit(bytes[at]) * 16 + digit(bytes[at + 1])
    }
    let alpha = if bytes.len() == 9 {
        pair(bytes, 7)
    } else {
        255
    };
    Color32::from_rgba_unmultiplied_const(pair(bytes, 1), pair(bytes, 3), pair(bytes, 5), alpha)
}

const fn pack(color: Color32) -> u32 {
    u32::from_be_bytes(color.to_array())
}

const fn unpack(packed: u32) -> Color32 {
    let [r, g, b, a] = packed.to_be_bytes();
    Color32::from_rgba_premultiplied(r, g, b, a)
}

/// Whether a theme is dark or light. It picks egui's base visuals and the few
/// things Baboon draws differently in light themes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) enum Appearance {
    Dark,
    Light,
}

impl Appearance {
    pub(in crate::app) fn is_dark(self) -> bool {
        self == Appearance::Dark
    }

    /// The name a theme file writes it as.
    pub(in crate::app) fn key(self) -> &'static str {
        match self {
            Appearance::Dark => "dark",
            Appearance::Light => "light",
        }
    }

    pub(in crate::app) fn from_key(key: &str) -> Option<Appearance> {
        match key {
            "dark" => Some(Appearance::Dark),
            "light" => Some(Appearance::Light),
            _ => None,
        }
    }
}

/// The themes built into Baboon, one column each in the token table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) enum BuiltIn {
    Dark,
    Light,
    ClassicLight,
}

impl BuiltIn {
    pub(in crate::app) const ALL: [BuiltIn; 3] =
        [BuiltIn::Dark, BuiltIn::Light, BuiltIn::ClassicLight];

    /// Its id, reserved: a user theme file may not take it.
    pub(in crate::app) fn id(self) -> &'static str {
        match self {
            BuiltIn::Dark => "baboon-dark",
            BuiltIn::Light => "baboon-light",
            BuiltIn::ClassicLight => "baboon-light-classic",
        }
    }

    /// The name Settings shows.
    pub(in crate::app) fn name(self) -> &'static str {
        match self {
            BuiltIn::Dark => "Dark",
            BuiltIn::Light => "Light",
            BuiltIn::ClassicLight => "Classic Light",
        }
    }

    pub(in crate::app) fn appearance(self) -> Appearance {
        match self {
            BuiltIn::Dark => Appearance::Dark,
            BuiltIn::Light | BuiltIn::ClassicLight => Appearance::Light,
        }
    }

    pub(in crate::app) fn from_id(id: &str) -> Option<BuiltIn> {
        BuiltIn::ALL
            .into_iter()
            .find(|built_in| built_in.id() == id)
    }

    /// The built-in a theme of `appearance` falls back to.
    pub(in crate::app) fn for_appearance(appearance: Appearance) -> BuiltIn {
        match appearance {
            Appearance::Dark => BuiltIn::Dark,
            Appearance::Light => BuiltIn::Light,
        }
    }

    fn column(self) -> &'static [Color32; TOKEN_COUNT] {
        match self {
            BuiltIn::Dark => &table::DARK,
            BuiltIn::Light => &table::LIGHT,
            BuiltIn::ClassicLight => &table::CLASSIC_LIGHT,
        }
    }

    pub(in crate::app) fn colors(self) -> ThemeColors {
        ThemeColors {
            appearance: self.appearance(),
            colors: *self.column(),
        }
    }

    /// Its value for `token`.
    #[cfg(test)]
    pub(in crate::app) fn color(self, token: Token) -> Color32 {
        self.column()[token as usize]
    }
}

/// One colour per token, and whether the theme is dark or light: what a
/// theme resolves to, and what is installed.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct ThemeColors {
    pub(in crate::app) appearance: Appearance,
    colors: [Color32; TOKEN_COUNT],
}

impl ThemeColors {
    pub(in crate::app) fn get(&self, token: Token) -> Color32 {
        self.colors[token as usize]
    }

    pub(in crate::app) fn set(&mut self, token: Token, color: Color32) {
        self.colors[token as usize] = color;
    }
}

static DARK_APPEARANCE: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    /// A theme drawn in on this thread alone: see [`with_colors`].
    static TEST_COLORS: std::cell::RefCell<Option<ThemeColors>> = const { std::cell::RefCell::new(None) };
}

/// `token`'s colour in the installed theme.
pub(in crate::app) fn color(token: Token) -> Color32 {
    #[cfg(test)]
    if let Some(color) =
        TEST_COLORS.with(|colors| colors.borrow().as_ref().map(|colors| colors.get(token)))
    {
        return color;
    }
    unpack(table::INSTALLED[token as usize].load(Ordering::Relaxed))
}

/// The installed theme's appearance.
pub(in crate::app) fn appearance() -> Appearance {
    #[cfg(test)]
    if let Some(appearance) =
        TEST_COLORS.with(|colors| colors.borrow().as_ref().map(|colors| colors.appearance))
    {
        return appearance;
    }
    if DARK_APPEARANCE.load(Ordering::Relaxed) {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// The installed theme, read back.
#[cfg(test)]
pub(in crate::app) fn installed() -> ThemeColors {
    let mut colors = [Color32::TRANSPARENT; TOKEN_COUNT];
    for token in Token::ALL {
        colors[*token as usize] = color(*token);
    }
    ThemeColors {
        appearance: appearance(),
        colors,
    }
}

/// Counts installs that changed a colour, so what is built from the theme
/// (egui's visuals, recoloured icons) knows when to build again.
pub(in crate::app) fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Installs `colors`, on the UI thread before anything draws. Installing the
/// theme already installed does nothing, so the frame can push its theme
/// every pass; an install that changes it clears the recoloured-icon caches.
/// Returns whether anything changed.
pub(in crate::app) fn install(colors: &ThemeColors) -> bool {
    let changed = colors.appearance != appearance()
        || Token::ALL
            .iter()
            .any(|token| colors.get(*token) != color(*token));
    if !changed {
        return false;
    }
    for token in Token::ALL {
        table::INSTALLED[*token as usize].store(pack(colors.get(*token)), Ordering::Relaxed);
    }
    DARK_APPEARANCE.store(colors.appearance.is_dark(), Ordering::Relaxed);
    GENERATION.fetch_add(1, Ordering::Relaxed);
    super::forget_recoloured_icons();
    true
}

/// Runs `f` with this thread drawing in `colors`, whatever the app's frames
/// on other threads install meanwhile.
#[cfg(test)]
pub(in crate::app) fn with_colors<R>(colors: &ThemeColors, f: impl FnOnce() -> R) -> R {
    let before = TEST_COLORS.with(|cell| cell.replace(Some(colors.clone())));
    let result = f();
    TEST_COLORS.with(|cell| cell.replace(before));
    result
}

/// Whether this thread draws in a theme of its own ([`with_colors`]).
#[cfg(test)]
pub(in crate::app) fn has_test_colors() -> bool {
    TEST_COLORS.with(|cell| cell.borrow().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ids are unique, lowercase and dotted: `group.role[.state]`, with `_`
    /// only inside a segment.
    #[test]
    fn token_ids_are_unique_and_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for token in Token::ALL {
            let id = token.id();
            assert!(seen.insert(id), "{id} is declared twice");
            assert!(well_formed(id), "{id} is not group.role[.state]");
            assert_eq!(Token::from_id(id), Some(*token));
            assert!(!token.description().is_empty(), "{id} has no description");
        }
        assert_eq!(seen.len(), TOKEN_COUNT);
        assert!(!well_formed("Block.Header"), "the check refuses capitals");
        assert!(
            !well_formed("block..header"),
            "the check refuses empty segments"
        );
    }

    fn well_formed(id: &str) -> bool {
        id.split('.').all(|segment| {
            segment.starts_with(|c: char| c.is_ascii_lowercase())
                && segment.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        })
    }

    /// `#RRGGBBAA` is straight alpha, as in CSS: white at 10% is the
    /// premultiplied [26, 26, 26, 26] a hand-built overlay used.
    #[test]
    fn hex_alpha_is_straight() {
        assert_eq!(
            hex("#FFFFFF1A"),
            Color32::from_rgba_premultiplied(26, 26, 26, 26)
        );
        assert_eq!(hex("#406C86"), Color32::from_rgb(64, 108, 134));
    }

    /// Installing the theme already installed changes nothing and builds
    /// nothing again; installing another does, and reads back exactly.
    #[test]
    fn installing_changes_only_what_differs() {
        let _lock = INSTALL_LOCK.lock().unwrap();
        let before = installed();
        install(&BuiltIn::Dark.colors());
        let generation = generation();
        assert!(!install(&BuiltIn::Dark.colors()));
        assert_eq!(super::generation(), generation);
        assert!(install(&BuiltIn::ClassicLight.colors()));
        assert_eq!(installed(), BuiltIn::ClassicLight.colors());
        assert!(super::generation() > generation);
        install(&before);
    }

    /// Tests that install a theme take this, since the installed theme is
    /// global.
    pub(in crate::app) static INSTALL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
