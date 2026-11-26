//! Foreground and background pairs a theme should keep readable, and the
//! WCAG contrast ratio of each. Warnings only: a theme under a threshold
//! still applies.
//!
//! Text and status colours are held to 4.5:1 and muted text to 3:1, which
//! every built-in meets. `link.text` and `field.help.text` are not listed:
//! the light built-ins draw them at about 2.2 to 2.6:1, so every theme
//! extending one would warn.

use super::{ThemeColors, Token};

/// A foreground that is drawn on a background, and the least contrast it
/// should have.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct Pair {
    pub(in crate::app) foreground: Token,
    pub(in crate::app) background: Token,
    pub(in crate::app) minimum: f32,
}

const fn pair(foreground: Token, background: Token, minimum: f32) -> Pair {
    Pair {
        foreground,
        background,
        minimum,
    }
}

const BODY: f32 = 4.5;
const MUTED: f32 = 3.0;

pub(in crate::app) const PAIRS: &[Pair] = &[
    pair(Token::Text, Token::Background, BODY),
    pair(Token::Text, Token::PanelBackground, BODY),
    pair(Token::Text, Token::RowBackground, BODY),
    pair(Token::Text, Token::GroupBackground, BODY),
    pair(Token::Text, Token::ControlBackground, BODY),
    pair(Token::Text, Token::Accent, BODY),
    pair(Token::TextStrong, Token::Background, BODY),
    pair(Token::TextMuted, Token::Background, MUTED),
    pair(Token::TextMuted, Token::PanelBackground, MUTED),
    pair(Token::AccentText, Token::Background, BODY),
    pair(Token::BadgeText, Token::BadgeBackground, BODY),
    pair(Token::BlockHeaderText, Token::BlockHeaderBackground, BODY),
    pair(Token::StatusError, Token::Background, BODY),
    pair(Token::StatusError, Token::PanelBackground, BODY),
    pair(Token::StatusError, Token::StatusErrorBackground, BODY),
    pair(Token::StatusWarning, Token::Background, BODY),
    pair(Token::StatusWarning, Token::PanelBackground, BODY),
    pair(Token::StatusWarning, Token::StatusWarningBackground, BODY),
    pair(Token::StatusSuccess, Token::Background, BODY),
    pair(Token::StatusSuccess, Token::PanelBackground, BODY),
    pair(Token::ShaderText, Token::ShaderPanelBackground, BODY),
    pair(Token::ShaderTextMuted, Token::ShaderPanelBackground, MUTED),
    pair(Token::MaterialText, Token::MaterialBoxBackground, BODY),
    pair(
        Token::MaterialTextMuted,
        Token::MaterialBoxBackground,
        MUTED,
    ),
    pair(Token::TerminalText, Token::TerminalBackground, BODY),
    pair(Token::SearchMatchText, Token::SearchMatchBackground, BODY),
];

/// A pair under its threshold.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct Warning {
    pub(in crate::app) foreground: Token,
    pub(in crate::app) background: Token,
    pub(in crate::app) ratio: f32,
    pub(in crate::app) minimum: f32,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} on {} is {:.1}:1, under {}:1",
            self.foreground.id(),
            self.background.id(),
            self.ratio,
            self.minimum
        )
    }
}

/// The WCAG contrast ratio of `foreground` drawn on `background`, each
/// translucent colour laid over what is under it: the background over the
/// page, the foreground over the background.
pub(in crate::app) fn ratio(colors: &ThemeColors, foreground: Token, background: Token) -> f32 {
    let over = |top: [u8; 4], under: [f32; 3]| -> [f32; 3] {
        let alpha = f32::from(top[3]) / 255.0;
        std::array::from_fn(|channel| {
            f32::from(top[channel]) * alpha + under[channel] * (1.0 - alpha)
        })
    };
    let page = colors.get(Token::Background).to_srgba_unmultiplied();
    let page = over(page, [0.0; 3]);
    let back = over(colors.get(background).to_srgba_unmultiplied(), page);
    let front = over(colors.get(foreground).to_srgba_unmultiplied(), back);
    let (light, dark) = {
        let (a, b) = (luminance(front), luminance(back));
        if a >= b { (a, b) } else { (b, a) }
    };
    (light + 0.05) / (dark + 0.05)
}

/// Relative luminance of an sRGB colour with channels out of 255.
fn luminance(rgb: [f32; 3]) -> f32 {
    let linear = |channel: f32| {
        let channel = channel / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
}

/// Every pair of `colors` under its threshold, in the order listed.
pub(in crate::app) fn warnings(colors: &ThemeColors) -> Vec<Warning> {
    PAIRS
        .iter()
        .filter_map(|pair| {
            let ratio = ratio(colors, pair.foreground, pair.background);
            (ratio < pair.minimum).then_some(Warning {
                foreground: pair.foreground,
                background: pair.background,
                ratio,
                minimum: pair.minimum,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::BuiltIn;
    use super::*;
    use eframe::egui::Color32;

    #[test]
    fn the_built_ins_are_readable() {
        for built_in in BuiltIn::ALL {
            assert_eq!(
                warnings(&built_in.colors()),
                Vec::new(),
                "{}",
                built_in.id()
            );
        }
    }

    #[test]
    fn text_the_colour_of_its_page_is_reported() {
        let mut colors = BuiltIn::Dark.colors();
        colors.set(Token::Text, colors.get(Token::Background));
        let warnings = warnings(&colors);
        assert_eq!(
            warnings[0].to_string(),
            "text on background is 1.0:1, under 4.5:1"
        );
        assert!(
            warnings
                .iter()
                .all(|warning| warning.foreground == Token::Text)
        );
    }

    #[test]
    fn ratios_match_wcag() {
        let mut colors = BuiltIn::Dark.colors();
        colors.set(Token::Background, Color32::BLACK);
        colors.set(Token::Text, Color32::WHITE);
        assert!((ratio(&colors, Token::Text, Token::Background) - 21.0).abs() < 0.01);
        // Half-transparent white over black is mid-grey, about 5.3:1.
        colors.set(
            Token::Text,
            Color32::from_rgba_unmultiplied(255, 255, 255, 128),
        );
        let grey = ratio(&colors, Token::Text, Token::Background);
        assert!((5.0..5.6).contains(&grey), "{grey}");
    }
}
