//! Every way out of the app to a web page asks the platform to open it.
//! egui only reports the request; eframe opens it only with its `links`
//! feature, which egui 0.29's eframe turned on by itself and 0.36's does not.

use super::perf_baseline::Harness;
use super::*;

/// Slide onto `text`'s `nth` painting over a few frames, press and release,
/// and return every URL the frames asked to open.
fn click(h: &mut Harness, text: &str, nth: usize) -> Vec<egui::OpenUrl> {
    let rect = h
        .painted_rects
        .iter()
        .filter(|(painted, _)| painted == text)
        .nth(nth)
        .map(|(_, rect)| *rect)
        .unwrap_or_else(|| panic!("{text:?} is not painted: {:?}", h.painted));
    let target = rect.center();
    let mut opened = Vec::new();
    let mut frame = |h: &mut Harness, events: Vec<egui::Event>| {
        h.frame(events);
        opened.extend(h.commands.iter().filter_map(|command| match command {
            egui::OutputCommand::OpenUrl(open) => Some(open.clone()),
            _ => None,
        }));
    };
    for step in 1..=3 {
        let from = target - egui::vec2(30.0, 30.0);
        frame(h, vec![egui::Event::PointerMoved(from + (target - from) * step as f32 / 3.0)]);
    }
    let button = |pressed| egui::Event::PointerButton {
        pos: target,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(h, vec![button(true)]);
    frame(h, vec![button(false)]);
    frame(h, Vec::new());
    opened
}

fn urls(opened: &[egui::OpenUrl]) -> Vec<&str> {
    opened.iter().map(|open| open.url.as_str()).collect()
}

fn idle(h: &mut Harness) {
    for _ in 0..4 {
        h.frame(Vec::new());
    }
}

/// The tests below see the request; only eframe's `links` feature turns it
/// into an open browser tab, and nothing fails without it but the user's click.
#[test]
fn eframe_is_built_with_its_links_feature() {
    let manifest = include_str!("../../../Cargo.toml");
    let eframe = manifest
        .lines()
        .find(|line| line.starts_with("eframe = "))
        .expect("Cargo.toml names eframe");
    assert!(eframe.contains("\"links\""), "{eframe}");
}

/// The welcome screen's GitHub and Discord buttons.
#[test]
fn the_welcome_links_ask_to_open_their_pages() {
    let mut h = Harness::new();
    idle(&mut h);
    assert_eq!(urls(&click(&mut h, "Baboon GitHub", 0)), [BABOON_GITHUB_URL]);
    assert_eq!(
        urls(&click(&mut h, "Halo Mods Discord", 0)),
        ["https://discord.com/invite/4pKEpNW"]
    );
}

/// The Help window's source link and a tutorial's "Watch on YouTube".
#[test]
fn the_help_window_links_ask_to_open_their_pages() {
    let mut h = Harness::new();
    h.app.about_open = true;
    h.app.help_panel_tab = HelpPanelTab::About;
    idle(&mut h);
    assert_eq!(urls(&click(&mut h, BABOON_GITHUB_URL, 0)), [BABOON_GITHUB_URL]);

    h.app.help_panel_tab = HelpPanelTab::Tutorials;
    idle(&mut h);
    let opened = click(&mut h, "Watch on YouTube", 0);
    assert_eq!(opened.len(), 1, "{opened:?}");
    assert!(opened[0].url.starts_with("https://"), "{opened:?}");
    assert!(opened[0].new_tab);
}

/// An available update is linked from the status bar and the Help menu.
#[test]
fn the_update_links_ask_to_open_the_release() {
    let release = "https://github.com/Zoephie/Baboon/releases/tag/v9.9.9";
    let mut h = Harness::new();
    h.app.available_update = Some(UpdateCheckResult {
        channel: UpdateChannel::Stable,
        latest_tag: "v9.9.9".to_owned(),
        release_url: release.to_owned(),
        commit: "0123456789abcdef".to_owned(),
    });
    idle(&mut h);
    assert_eq!(urls(&click(&mut h, "Update available: v9.9.9", 0)), [release]);

    assert!(click(&mut h, "Help", 0).is_empty(), "opening the menu opens nothing");
    assert_eq!(urls(&click(&mut h, "Update available: v9.9.9...", 0)), [release]);
}
