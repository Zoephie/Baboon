//! How a token follows the tokens it is derived from: when a theme sets one
//! of them, a token it leaves unset moves with it (resolution step 6) rather
//! than keeping its built-in value.
//!
//! A rule is fitted to the dark and the warm light built-ins, one per
//! appearance; a token no rule fits within [`TOLERANCE`] of its built-in
//! value has none, and keeps its built-in value whatever a theme sets.

use super::Token;
use eframe::egui::Color32;

/// How far, per channel out of 255, a rule's colour may be from the
/// built-in's hand-picked one, so moving between them is not a visible jump.
#[cfg(test)]
pub(in crate::app) const TOLERANCE: u8 = 4;

/// One colour computed from others, in sRGB with straight alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) enum Rule {
    /// The same colour as the token.
    Reference(Token),
    /// The token at this opacity. A form the table may use; no token does yet.
    #[cfg_attr(not(test), allow(dead_code))]
    Alpha(Token, f32),
    /// The token moved this far toward white.
    Tint(Token, f32),
    /// The token moved this far toward black.
    Shade(Token, f32),
    /// The first token moved this far toward the second.
    Mix(Token, Token, f32),
}

impl std::fmt::Display for Rule {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rule::Reference(token) => write!(formatter, "{}", token.id()),
            Rule::Alpha(token, alpha) => write!(formatter, "{} at {alpha}", token.id()),
            Rule::Tint(token, t) => write!(formatter, "{} {t} toward white", token.id()),
            Rule::Shade(token, t) => write!(formatter, "{} {t} toward black", token.id()),
            Rule::Mix(from, toward, t) => {
                write!(formatter, "{} {t} toward {}", from.id(), toward.id())
            }
        }
    }
}

impl Rule {
    /// The tokens it is computed from.
    pub(in crate::app) fn inputs(self) -> Vec<Token> {
        match self {
            Rule::Reference(token)
            | Rule::Alpha(token, _)
            | Rule::Tint(token, _)
            | Rule::Shade(token, _) => vec![token],
            Rule::Mix(from, toward, _) => vec![from, toward],
        }
    }

    /// Its colour, reading its inputs through `get`.
    pub(in crate::app) fn evaluate(self, get: impl Fn(Token) -> Color32) -> Color32 {
        let straight = |token: Token| get(token).to_srgba_unmultiplied();
        let round = |value: f32| (value + 0.5).clamp(0.0, 255.0) as u8;
        let toward = |from: [u8; 4], to: [u8; 4], t: f32| -> [u8; 4] {
            std::array::from_fn(|channel| {
                round(
                    f32::from(from[channel])
                        + (f32::from(to[channel]) - f32::from(from[channel])) * t,
                )
            })
        };
        let [r, g, b, a] = match self {
            Rule::Reference(token) => straight(token),
            Rule::Alpha(token, alpha) => {
                let [r, g, b, _] = straight(token);
                [r, g, b, round(alpha * 255.0)]
            }
            Rule::Tint(token, t) => {
                let from = straight(token);
                let [r, g, b, _] = toward(from, [255; 4], t);
                [r, g, b, from[3]]
            }
            Rule::Shade(token, t) => {
                let from = straight(token);
                let [r, g, b, _] = toward(from, [0, 0, 0, 255], t);
                [r, g, b, from[3]]
            }
            Rule::Mix(from, to, t) => toward(straight(from), straight(to), t),
        };
        Color32::from_rgba_unmultiplied(r, g, b, a)
    }
}

/// A token's rule in each appearance; `None` where no rule fits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) struct Rules {
    pub(in crate::app) dark: Option<Rule>,
    pub(in crate::app) light: Option<Rule>,
}

impl Rules {
    pub(in crate::app) const NONE: Rules = Rules {
        dark: None,
        light: None,
    };

    pub(in crate::app) fn of(self, appearance: super::Appearance) -> Option<Rule> {
        if appearance.is_dark() {
            self.dark
        } else {
            self.light
        }
    }
}

// The table's words for rules.

pub(in crate::app) const fn reference(token: Token) -> Rule {
    Rule::Reference(token)
}

pub(in crate::app) const fn tint(token: Token, t: f32) -> Rule {
    Rule::Tint(token, t)
}

pub(in crate::app) const fn shade(token: Token, t: f32) -> Rule {
    Rule::Shade(token, t)
}

pub(in crate::app) const fn mix(from: Token, toward: Token, t: f32) -> Rule {
    Rule::Mix(from, toward, t)
}

/// The same rule in both appearances.
pub(in crate::app) const fn both(rule: Rule) -> Rules {
    Rules {
        dark: Some(rule),
        light: Some(rule),
    }
}

pub(in crate::app) const fn by_appearance(dark: Rule, light: Rule) -> Rules {
    Rules {
        dark: Some(dark),
        light: Some(light),
    }
}

/// A rule in dark themes only; light themes keep the built-in value.
pub(in crate::app) const fn dark(rule: Rule) -> Rules {
    Rules {
        dark: Some(rule),
        light: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::library::ThemeLibrary;
    use super::super::{Appearance, BuiltIn, ThemeColors};
    use super::*;

    /// How far `rule` lands from `built_in`'s value of `token`, worst channel.
    fn error(token: Token, rule: Rule, built_in: BuiltIn) -> u8 {
        let colors = built_in.colors();
        let derived = rule
            .evaluate(|input| colors.get(input))
            .to_srgba_unmultiplied();
        let picked = colors.get(token).to_srgba_unmultiplied();
        (0..4)
            .map(|channel| derived[channel].abs_diff(picked[channel]))
            .max()
            .unwrap()
    }

    /// Each rule gives its built-in's hand-picked colour within the tolerance,
    /// in the dark built-in and the warm light one it was fitted to.
    #[test]
    fn every_rule_reproduces_its_built_in() {
        let mut ruled = 0;
        for token in Token::ALL {
            for (appearance, built_in) in [
                (Appearance::Dark, BuiltIn::Dark),
                (Appearance::Light, BuiltIn::Light),
            ] {
                if let Some(rule) = token.rules().of(appearance) {
                    ruled += 1;
                    let error = error(*token, rule, built_in);
                    assert!(
                        error <= TOLERANCE,
                        "{} under {}: {rule:?} is {error} away",
                        token.id(),
                        built_in.id()
                    );
                }
            }
        }
        assert!(ruled > 40, "the table has its rules");
    }

    /// The check notices a wrong rule: one with its fraction doubled.
    #[test]
    fn a_rule_with_its_fraction_doubled_misses() {
        let rule = Token::RowBackground
            .rules()
            .dark
            .expect("row.background follows the page");
        let Rule::Tint(input, t) = rule else {
            panic!("{rule:?}")
        };
        assert!(
            error(
                Token::RowBackground,
                Rule::Tint(input, t * 2.0),
                BuiltIn::Dark
            ) > TOLERANCE
        );
    }

    /// A rule reads only tokens declared before it, so rules resolve in one
    /// pass in table order and cannot go round in a circle.
    #[test]
    fn rules_read_only_earlier_tokens() {
        for (at, token) in Token::ALL.iter().enumerate() {
            for rule in [token.rules().dark, token.rules().light]
                .into_iter()
                .flatten()
            {
                for input in rule.inputs() {
                    assert!(
                        (input as usize) < at,
                        "{} reads {}, declared after it",
                        token.id(),
                        input.id()
                    );
                }
            }
        }
        assert!(
            !(Token::Background as usize > Token::RowBackground as usize),
            "the check orders by declaration"
        );
    }

    /// The tokens whose rule in `appearance` reads `from`, directly or
    /// through another such token.
    fn dependents(from: Token, appearance: Appearance) -> std::collections::BTreeSet<Token> {
        let mut moved = std::collections::BTreeSet::from([from]);
        for token in Token::ALL {
            if let Some(rule) = token.rules().of(appearance)
                && rule.inputs().iter().any(|input| moved.contains(input))
            {
                moved.insert(*token);
            }
        }
        moved.remove(&from);
        moved
    }

    /// Setting only `accent` over the dark built-in moves exactly the tokens
    /// whose rules read it, each to its rule's colour, and leaves every other
    /// token at the built-in's value.
    #[test]
    fn setting_only_the_accent_moves_exactly_its_dependents() {
        let orange = r##"{ "name": "Orange", "extends": "baboon-dark", "colors": { "accent": "#E8862A" } }"##;
        let library = ThemeLibrary::from_texts(&[("orange", orange)]);
        let resolved = library.resolve("orange").unwrap();
        let built_in = BuiltIn::Dark.colors();
        let dependents = dependents(Token::Accent, Appearance::Dark);
        assert!(!dependents.is_empty(), "something follows the accent");
        assert_eq!(resolved.following, dependents);
        for token in Token::ALL {
            let now = resolved.colors.get(*token);
            if *token == Token::Accent {
                assert_eq!(now, Color32::from_rgb(0xE8, 0x86, 0x2A));
            } else if dependents.contains(token) {
                let rule = token.rules().dark.unwrap();
                assert_eq!(
                    now,
                    rule.evaluate(|input| resolved.colors.get(input)),
                    "{}",
                    token.id()
                );
                assert_ne!(now, built_in.get(*token), "{} did not move", token.id());
            } else {
                assert_eq!(
                    now,
                    built_in.get(*token),
                    "{} moved though nothing it reads did",
                    token.id()
                );
            }
        }
    }

    /// A theme of its own that sets its page and text has everything derived
    /// from them follow, the rest from the built-in of its appearance, and
    /// reports exactly the tokens it left unset.
    #[test]
    fn a_theme_of_its_own_follows_what_it_sets() {
        let text = r##"{ "name": "Deep", "appearance": "dark", "colors": { "background": "#102030", "text": "#E0F0FF" } }"##;
        let library = ThemeLibrary::from_texts(&[("deep", text)]);
        let resolved = library.resolve("deep").unwrap();
        let expected: std::collections::BTreeSet<Token> =
            dependents(Token::Background, Appearance::Dark)
                .union(&dependents(Token::Text, Appearance::Dark))
                .copied()
                .collect();
        assert_eq!(resolved.following, expected);
        assert_eq!(
            resolved.colors.get(Token::Accent),
            BuiltIn::Dark.colors().get(Token::Accent)
        );
        let unset: Vec<&str> = Token::ALL
            .iter()
            .filter(|token| !matches!(token, Token::Background | Token::Text))
            .map(|token| token.id())
            .collect();
        let report = resolved
            .reports
            .iter()
            .find(|report| report.key.as_deref() == Some("colors"))
            .unwrap();
        assert!(
            report.message.ends_with(&unset.join(", ")),
            "{}",
            report.message
        );
        assert!(
            report
                .message
                .starts_with(&format!("leaves {} token(s) unset", unset.len()))
        );
    }

    #[test]
    fn rules_compute_in_straight_srgb() {
        let mut colors = BuiltIn::Dark.colors();
        colors.set(Token::Background, Color32::from_rgb(100, 100, 100));
        colors.set(Token::Text, Color32::from_rgb(200, 0, 0));
        let get = |token: Token| ThemeColors::get(&colors, token);
        assert_eq!(
            Rule::Tint(Token::Background, 0.5).evaluate(get),
            Color32::from_rgb(178, 178, 178)
        );
        assert_eq!(
            Rule::Shade(Token::Background, 0.5).evaluate(get),
            Color32::from_rgb(50, 50, 50)
        );
        assert_eq!(
            Rule::Mix(Token::Background, Token::Text, 0.5).evaluate(get),
            Color32::from_rgb(150, 50, 50)
        );
        assert_eq!(
            Rule::Alpha(Token::Text, 0.5).evaluate(get),
            Color32::from_rgba_unmultiplied(200, 0, 0, 128)
        );
    }
}
