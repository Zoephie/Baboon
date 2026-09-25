//! The browser tree lays out only the rows it can show.
//!
//! Tag rows below or above the clip rect are reserved as one block of space,
//! and a folder that was wholly off screen is reserved at the height it had
//! when last drawn. Neither may move a row: every test here compares what the
//! viewport shows against the same frames drawn with skipping turned off,
//! which lays out every row as the tree always did.

use std::time::Instant;

use eframe::egui;

use super::{
    Reveal, TREE_ROW_TOPS, TREE_ROWS_LAID_OUT, TREE_SKIPS_ROWS, draw_tree, draw_tree_lazy,
};
use crate::app::BrowserSort;
use crate::source::{TagEntry, TagEntryLocation, TagTree};

/// 40 top folders × 10 subfolders × 150 tags: 60,000 tags, 440 folders.
fn synthetic_entries() -> Vec<TagEntry> {
    let mut entries = Vec::new();
    for top in 0..40 {
        for sub in 0..10 {
            for tag in 0..150 {
                let path = format!("folder_{top:02}/sub_{sub:02}/tag_{tag:03}.biped");
                entries.push(TagEntry {
                    key: format!("file:{path}"),
                    display_path: path.clone(),
                    group_tag: u32::from_be_bytes(*b"bipd"),
                    group_name: Some("biped".to_owned()),
                    location: TagEntryLocation::LooseFile(path.into()),
                });
            }
        }
    }
    entries
}

struct Frame {
    /// The rows the viewport shows, as (label or key, top).
    visible: Vec<(String, f32)>,
    /// Every row laid out, visible or not.
    laid_out: usize,
    content_height: f32,
}

/// One browser, drawn frame by frame, with skipping on or off.
struct Browser {
    ctx: egui::Context,
    tree: TagTree,
    entries: Vec<TagEntry>,
    /// A loose folder drawn through the lazy tree, which loads each folder's
    /// tags the first time it opens. `None` draws `tree` from `entries`.
    lazy_root: Option<std::path::PathBuf>,
    skips: bool,
    time: f64,
    /// Where the previous frame put each row, for aiming a click.
    last: Vec<(String, f32)>,
    content_height: f32,
    /// The offset the last frame was drawn at. A frame given no offset
    /// holds it, rather than leaving the scroll area free to move while a
    /// click is aimed at where a row was.
    offset: f32,
}

impl Browser {
    fn new(skips: bool) -> Self {
        let entries = synthetic_entries();
        let ctx = egui::Context::default();
        ctx.style_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
        Self {
            ctx,
            tree: crate::source::build_tree(&entries),
            entries,
            lazy_root: None,
            skips,
            time: 0.0,
            last: Vec::new(),
            content_height: 0.0,
            offset: 0.0,
        }
    }

    fn frame(
        &mut self,
        offset: Option<f32>,
        filter: &str,
        events: Vec<egui::Event>,
        reveal: Option<Reveal<'_>>,
    ) -> Frame {
        self.time += 1.0 / 60.0;
        // A reveal scrolls the area itself, so its frames are left free.
        let offset = match (offset, reveal) {
            (Some(offset), _) => Some(self.resolve(offset)),
            (None, None) => Some(self.offset),
            (None, Some(_)) => None,
        };
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 800.0),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        TREE_SKIPS_ROWS.with(|skips| skips.set(self.skips));
        TREE_ROWS_LAID_OUT.with(|count| count.set(0));
        TREE_ROW_TOPS.with(|tops| tops.borrow_mut().clear());
        let mut viewport = egui::Rect::NOTHING;
        let mut content_height = 0.0;
        let mut shown_offset = 0.0;
        let (tree, entries, lazy_root) = (&mut self.tree, &mut self.entries, &self.lazy_root);
        let names = crate::format::TagNameIndex::default();
        let _ = self.ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut area = egui::ScrollArea::vertical();
                if let Some(offset) = offset {
                    area = area.vertical_scroll_offset(offset);
                }
                let output = area.show(ui, |ui| {
                    if let Some(root) = lazy_root {
                        draw_tree_lazy(
                            ui,
                            tree,
                            entries,
                            None,
                            root,
                            &names,
                            None,
                            filter,
                            false,
                            false,
                            &mut None,
                            reveal,
                            BrowserSort::Natural,
                            true,
                            None,
                        );
                        return;
                    }
                    draw_tree(
                        ui,
                        tree,
                        entries,
                        None,
                        filter,
                        true,
                        false,
                        false,
                        false,
                        reveal,
                        BrowserSort::Natural,
                        true,
                        None,
                        false,
                    );
                });
                viewport = output.inner_rect;
                shown_offset = output.state.offset.y;
                content_height = output.content_size.y;
            });
        });
        TREE_SKIPS_ROWS.with(|skips| skips.set(true));
        let tops = TREE_ROW_TOPS.with(|tops| std::mem::take(&mut *tops.borrow_mut()));
        let row_height = self.ctx.style().spacing.interact_size.y;
        let visible = tops
            .iter()
            .filter(|(_, top)| top + row_height > viewport.top() && *top < viewport.bottom())
            .cloned()
            .collect();
        self.last = tops;
        self.content_height = content_height;
        self.offset = offset.unwrap_or(shown_offset);
        Frame {
            visible,
            laid_out: TREE_ROWS_LAID_OUT.with(|count| count.get()),
            content_height,
        }
    }

    /// The lazy tree over `root`, every folder closed and unloaded.
    fn lazy(root: &std::path::Path, skips: bool) -> Self {
        let mut browser = Self::new(skips);
        browser.entries.clear();
        browser.tree = crate::source::build_folder_directory_tree(root).unwrap();
        browser.lazy_root = Some(root.to_path_buf());
        browser
    }

    fn scrolled(&mut self, offset: f32) -> Frame {
        self.frame(Some(offset), "", Vec::new(), None)
    }

    /// `END` resolved against the last frame's content height.
    fn resolve(&self, offset: f32) -> f32 {
        if offset == END {
            (self.content_height - 790.0).max(0.0)
        } else {
            offset
        }
    }

    /// Press and release on the first row labelled `label`.
    fn click_row(&mut self, label: &str) {
        let (_, top) = self
            .last
            .iter()
            .find(|(name, _)| name == label)
            .cloned()
            .unwrap_or_else(|| panic!("no `{label}` row was laid out"));
        let pos = egui::pos2(120.0, top + 6.0);
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(
            None,
            "",
            vec![egui::Event::PointerMoved(pos), button(true)],
            None,
        );
        self.frame(None, "", vec![button(false)], None);
    }
}

fn assert_same_view(skipping: &Frame, full: &Frame, context: &str) {
    assert!(
        !full.visible.is_empty(),
        "{context}: the viewport showed nothing"
    );
    assert_eq!(
        skipping.content_height, full.content_height,
        "{context}: skipping changed the tree's total height"
    );
    assert_eq!(
        skipping.visible, full.visible,
        "{context}: skipping moved or dropped a visible row"
    );
}

/// `END` is the last screenful, whatever the content height is.
const END: f32 = f32::MAX;
const OFFSETS: [f32; 6] = [0.0, 1234.5, 90_000.0, 250_000.0, 500_000.0, END];

#[test]
fn skipping_rows_shows_what_laying_out_every_row_shows() {
    let mut skipping = Browser::new(true);
    let mut full = Browser::new(false);
    for offset in OFFSETS.into_iter().chain(OFFSETS.into_iter().rev()) {
        // Twice: the first frame at an offset may still lay out folders it
        // has no height for; the second must use them.
        for pass in 0..2 {
            let context = format!("offset {offset}, frame {pass}");
            assert_same_view(&skipping.scrolled(offset), &full.scrolled(offset), &context);
        }
    }
}

/// 60,440 rows, of which about 40 fit in the viewport. Laid out, every one
/// cost 18 ms a frame in a release build.
#[test]
fn an_expanded_tree_lays_out_only_the_rows_it_shows() {
    let mut browser = Browser::new(true);
    for offset in OFFSETS {
        browser.scrolled(offset);
        let start = Instant::now();
        let frame = browser.scrolled(offset);
        let elapsed = start.elapsed();
        eprintln!(
            "offset {offset}: {} rows laid out, {} visible, in {elapsed:?}",
            frame.laid_out,
            frame.visible.len()
        );
        assert!(
            frame.laid_out <= frame.visible.len() + 16,
            "offset {offset}: laid out {} rows to show {}",
            frame.laid_out,
            frame.visible.len()
        );
    }
}

/// A new query changes every folder's height, so a height cached under the
/// old one must not be reused.
#[test]
fn a_new_filter_does_not_reuse_heights_from_the_old_one() {
    let mut skipping = Browser::new(true);
    let mut full = Browser::new(false);
    for offset in OFFSETS {
        skipping.scrolled(offset);
    }
    for offset in [0.0, 5_000.0, END] {
        for pass in 0..2 {
            let context = format!("filtered, offset {offset}, frame {pass}");
            assert_same_view(
                &skipping.frame(Some(offset), "tag_01", Vec::new(), None),
                &full.frame(Some(offset), "tag_01", Vec::new(), None),
                &context,
            );
        }
    }
}

/// A folder collapsed and then scrolled away mid-animation: the heights its
/// ancestors had while it shrank are not the heights they end with, so none
/// of them may be cached.
#[test]
fn a_folder_scrolled_away_mid_animation_is_measured_again() {
    let mut skipping = Browser::new(true);
    let mut full = Browser::new(false);
    for browser in [&mut skipping, &mut full] {
        browser.scrolled(0.0);
        browser.click_row("sub_00");
        // One frame into the close animation, then scroll far past it.
        browser.frame(None, "", Vec::new(), None);
        browser.scrolled(200_000.0);
        // Let the animation finish while the folder is off screen.
        browser.time += 1.0;
    }
    for offset in [200_000.0, 0.0, 200_000.0, END] {
        assert_same_view(
            &skipping.scrolled(offset),
            &full.scrolled(offset),
            &format!("after the collapse, offset {offset}"),
        );
    }
}

/// Revealing a tag scrolls to it even when its row lies in a run that was
/// reserved rather than laid out.
#[test]
fn revealing_a_skipped_tag_scrolls_it_into_view() {
    let mut browser = Browser::new(true);
    browser.scrolled(0.0);
    let key = "file:folder_30/sub_05/tag_100.biped";
    let ancestors = vec!["folder_30".to_owned(), "sub_05".to_owned()];
    let reveal = Reveal {
        key,
        remaining: &ancestors,
    };
    let mut shown = false;
    for _ in 0..4 {
        let frame = browser.frame(None, "", Vec::new(), Some(reveal));
        shown = frame.visible.iter().any(|(name, _)| name == key);
    }
    assert!(shown, "the revealed tag never came into view");
}

/// Revealing a tag in a folder the user collapsed, while that folder is off
/// screen: the reveal opens it, so it must be drawn, not reserved at the
/// height it had closed.
#[test]
fn revealing_into_a_collapsed_off_screen_folder_opens_it() {
    let mut browser = Browser::new(true);
    browser.scrolled(0.0);
    let offset = browser
        .last
        .iter()
        .find(|(name, _)| name == "folder_30")
        .map(|(_, top)| *top)
        .expect("folder_30 was laid out on the first frame");
    browser.scrolled(offset);
    browser.click_row("folder_30");
    browser.time += 1.0;
    let collapsed = browser.scrolled(0.0);
    assert!(
        !collapsed
            .visible
            .iter()
            .any(|(name, _)| name == "folder_30"),
        "folder_30 should be off screen before the reveal"
    );

    let key = "file:folder_30/sub_05/tag_100.biped";
    let ancestors = vec!["folder_30".to_owned(), "sub_05".to_owned()];
    let reveal = Reveal {
        key,
        remaining: &ancestors,
    };
    let mut shown = false;
    for _ in 0..4 {
        let frame = browser.frame(None, "", Vec::new(), Some(reveal));
        shown = frame.visible.iter().any(|(name, _)| name == key);
    }
    assert!(shown, "the revealed tag never came into view");
}

/// The lazy tree (a loose folder with no index) skips the same way. Its
/// folders open closed and load on first open, so each is clicked open,
/// bottom up so the rows above keep their places.
#[test]
fn the_lazy_tree_skips_rows_without_moving_any() {
    let root = crate::test_kits::unique_temp_dir("baboon-lazy-virtualization");
    let mut header = [0u8; 64];
    header[48..52].copy_from_slice(b"bipd");
    header[60..64].copy_from_slice(b"BLAM");
    for folder in 0..25 {
        let dir = root.join(format!("folder_{folder:02}"));
        std::fs::create_dir_all(&dir).unwrap();
        for tag in 0..80 {
            std::fs::write(dir.join(format!("tag_{tag:03}.biped")), header).unwrap();
        }
    }

    let mut skipping = Browser::lazy(&root, true);
    let mut full = Browser::lazy(&root, false);
    for browser in [&mut skipping, &mut full] {
        browser.scrolled(0.0);
        for folder in (0..25).rev() {
            browser.click_row(&format!("folder_{folder:02}"));
        }
        browser.time += 1.0;
    }
    full.scrolled(END);
    let opened = full.scrolled(END);
    assert!(
        opened
            .visible
            .iter()
            // Keys are platform paths: `\` on Windows.
            .any(|(name, _)| name.replace('\\', "/").ends_with("folder_24/tag_079.biped")),
        "the last folder did not open: {:?}",
        opened.visible.last()
    );
    for offset in [0.0, 1234.5, 20_000.0, 35_000.0, END, 20_000.0, 0.0] {
        for pass in 0..2 {
            let context = format!("lazy, offset {offset}, frame {pass}");
            assert_same_view(&skipping.scrolled(offset), &full.scrolled(offset), &context);
        }
    }
    let frame = skipping.scrolled(20_000.0);
    assert!(
        frame.laid_out <= frame.visible.len() + 16,
        "laid out {} rows to show {}",
        frame.laid_out,
        frame.visible.len()
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Tags removed from folders far above the viewport — a delete, or a
/// refresh that found fewer — rebuild the tree. Heights cached for the old
/// tree must not reserve space for rows that are gone.
#[test]
fn a_rebuilt_tree_does_not_reuse_heights_from_the_old_one() {
    let mut skipping = Browser::new(true);
    let mut full = Browser::new(false);
    for offset in OFFSETS {
        skipping.scrolled(offset);
    }
    for browser in [&mut skipping, &mut full] {
        browser
            .entries
            .retain(|entry| !entry.display_path.starts_with("folder_02/sub_03/tag_1"));
        browser.tree = crate::source::build_tree(&browser.entries);
    }
    for offset in [500_000.0, END] {
        for pass in 0..2 {
            let context = format!("rebuilt, offset {offset}, frame {pass}");
            assert_same_view(&skipping.scrolled(offset), &full.scrolled(offset), &context);
        }
    }
}
