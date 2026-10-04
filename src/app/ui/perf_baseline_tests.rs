//! Frame-time baselines: whole application frames, run headless.
//!
//! Every scenario drives [`Baboon::run_frame`] — the body of
//! `eframe::App::logic` then `App::ui` — on an egui context configured by
//! [`Baboon::configure_context`], exactly as the window does, then
//! tessellates the output as eframe would before handing it to the GPU. What
//! is timed is therefore the CPU side of a real frame: input, every panel,
//! the worker drain, the prefs throttle, autosave checks, and tessellation.
//! The GPU upload and draw are not (there is no GPU here).
//!
//! All data is synthetic: tags are built from this repository's own
//! `definitions/` schemas, never read from a kit, so this runs anywhere.
//!
//! Run (release is the number that matters; debug only proves it works):
//!
//! ```text
//! cargo test --release perf_baseline -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Comparing against a baseline: run once on the commit to compare against
//! and once on the change, on the same machine, quiet, with the same
//! `BABOON_PERF_PPP`, appending to one CSV under two labels:
//!
//! ```text
//! BABOON_PERF_LABEL=before BABOON_PERF_CSV=perf.csv cargo test --release perf_baseline -- --ignored --nocapture --test-threads=1
//! BABOON_PERF_LABEL=after  BABOON_PERF_CSV=perf.csv cargo test --release perf_baseline -- --ignored --nocapture --test-threads=1
//! ```
//!
//! then compare rows by `scenario`: `median_ms` and `p95_ms` for time, and
//! the counter columns (rows laid out, labels built, ...) for work, which
//! do not depend on the machine and should match exactly unless the change
//! meant to alter them. A run on a loaded machine is noise; differences of
//! a few percent in time are noise anyway. A failed scenario check (the
//! state did not show what the scenario claims) fails the test; the
//! numbers of the scenarios that passed are still printed.
//!
//! Environment:
//! - `BABOON_PERF_WARMUP`  warm-up frames per scenario (default 20)
//! - `BABOON_PERF_FRAMES`  measured frames per scenario (default 120)
//! - `BABOON_PERF_ONLY`    comma-separated substrings; run only scenarios
//!   whose name contains one of them
//! - `BABOON_PERF_PPP`     pixels per point (default 1.0; 2.0 for Retina)
//! - `BABOON_PERF_CSV`     append one CSV row per scenario to this file
//! - `BABOON_PERF_LABEL`   the CSV's label column, e.g. `before` / `after`
//!
//! Surviving a restructure: scenarios only describe *what is on screen*
//! through the [`fixture`] module below, which is the one place that knows
//! how app state is laid out (kits, documents, the terminal, caches). When
//! that layout changes, only `fixture` needs porting; scenario definitions,
//! measurement and reporting stay as they are, so before/after numbers stay
//! comparable. The layout counters are `#[cfg(test)]` thread-locals the app
//! already keeps; [`Counters`] is the one place that reads them.

use super::*;
use std::time::{Duration, Instant};

const SCREEN: egui::Vec2 = egui::vec2(1600.0, 1000.0);
/// Inside the kit's browser side panel (330 points wide by default).
const BROWSER_POINT: egui::Pos2 = egui::pos2(160.0, 600.0);
/// Inside the tag pane, right of the browser.
const PANE_POINT: egui::Pos2 = egui::pos2(1000.0, 600.0);
/// One wheel notch, in points.
const WHEEL_STEP: f32 = 120.0;

// ---------------------------------------------------------------------------
// Counters
// ---------------------------------------------------------------------------

/// Layout work the app reports through its `#[cfg(test)]` counters. Each is
/// reset before a frame and read after it.
#[derive(Clone, Copy, Default)]
struct Counters {
    /// Browser rows (tags and folder headers) laid out.
    tree_rows: usize,
    /// Read-only function previews built.
    function_previews: usize,
    /// Block-element dropdown labels built.
    dropdown_labels: usize,
    /// Terminal output lines laid out.
    terminal_lines: usize,
    /// Shader editor models built (should be 0 once warm: it is memoized).
    shader_models: usize,
}

impl Counters {
    fn reset() {
        crate::app::browser::TREE_ROWS_LAID_OUT.with(|c| c.set(0));
        crate::app::editor::fields::FUNCTION_PREVIEWS_BUILT.with(|c| c.set(0));
        crate::app::editor::fields::DROPDOWN_LABELS_BUILT.with(|c| c.set(0));
        super::shell::terminal_output_tests::LINES_BUILT.with(|c| c.set(0));
        crate::app::editor::material::SHADER_MODELS_BUILT.with(|c| c.set(0));
    }

    fn read() -> Self {
        Self {
            tree_rows: crate::app::browser::TREE_ROWS_LAID_OUT.with(std::cell::Cell::get),
            function_previews: crate::app::editor::fields::FUNCTION_PREVIEWS_BUILT
                .with(std::cell::Cell::get),
            dropdown_labels: crate::app::editor::fields::DROPDOWN_LABELS_BUILT
                .with(std::cell::Cell::get),
            terminal_lines: super::shell::terminal_output_tests::LINES_BUILT
                .with(std::cell::Cell::get),
            shader_models: crate::app::editor::material::SHADER_MODELS_BUILT.with(std::cell::Cell::get),
        }
    }
}

// ---------------------------------------------------------------------------
// Harness: one app on one headless context, driven frame by frame
// ---------------------------------------------------------------------------

pub(super) struct FrameSample {
    /// `ctx.run` around the app's frame.
    run: Duration,
    /// `ctx.tessellate` of that frame's shapes.
    tessellate: Duration,
    counters: Counters,
}

impl FrameSample {
    fn total(&self) -> Duration {
        self.run + self.tessellate
    }
}

pub(super) struct Harness {
    pub(super) ctx: egui::Context,
    pub(super) app: Baboon,
    /// Seconds; advanced 1/60 s a frame so animations, tooltips and the
    /// app's own throttles see time pass as they would at 60 Hz.
    time: f64,
    pixels_per_point: f32,
    /// Every text painted by the last frame, for the scenario checks.
    pub(super) painted: Vec<String>,
    /// The same texts with where they were painted, for clicking them.
    pub(super) painted_rects: Vec<(String, egui::Rect)>,
    /// What the last frame asked the platform to do.
    pub(super) commands: Vec<egui::OutputCommand>,
}

impl Harness {
    pub(super) fn new() -> Self {
        let ctx = egui::Context::default();
        Baboon::configure_context(&ctx);
        let names = TagNameIndex::load_from_definitions(&locate_definitions_root());
        let mut prefs = GuiPrefs::default();
        // A wheel over a dropdown would otherwise cycle it, editing the tag
        // under a scroll scenario.
        prefs.scroll_to_cycle_dropdowns = false;
        let app = Baboon::assemble(
            &ctx,
            crate::window_state::WindowStateTracker::for_test(),
            prefs,
            HashSet::new(),
            None,
            names,
            None,
        );
        let pixels_per_point = std::env::var("BABOON_PERF_PPP")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1.0);
        Self {
            ctx,
            app,
            time: 0.0,
            pixels_per_point,
            painted: Vec::new(),
            painted_rects: Vec::new(),
            commands: Vec::new(),
        }
    }

    /// Run one whole application frame with `events`, timed.
    pub(super) fn frame(&mut self, events: Vec<egui::Event>) -> FrameSample {
        self.time += 1.0 / 60.0;
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
            time: Some(self.time),
            focused: true,
            events,
            ..Default::default()
        };
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(self.pixels_per_point);
        Counters::reset();
        let app = &mut self.app;
        let started = Instant::now();
        let output = crate::app::run_ui_test(&self.ctx, input, |ui| app.run_frame(ui));
        let run = started.elapsed();
        let counters = Counters::read();
        let started = Instant::now();
        let shapes = output.shapes.clone();
        let primitives = self.ctx.tessellate(shapes, output.pixels_per_point);
        let tessellate = started.elapsed();
        std::hint::black_box(primitives);
        // Outside the timed spans.
        self.painted_rects = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect();
        self.painted = self.painted_rects.iter().map(|(text, _)| text.clone()).collect();
        self.commands = output.platform_output.commands;
        FrameSample {
            run,
            tessellate,
            counters,
        }
    }

    fn idle(&mut self, frames: usize) {
        for _ in 0..frames {
            self.frame(Vec::new());
        }
    }

    fn painted_contains(&self, needle: &str) -> bool {
        self.painted.iter().any(|text| text.contains(needle))
    }

    /// Slide onto the `nth` painting of exactly `text` over a few frames,
    /// press and release, and return everything those frames asked the
    /// platform to do.
    pub(super) fn click(&mut self, text: &str, nth: usize) -> Vec<egui::OutputCommand> {
        let rect = self
            .painted_rects
            .iter()
            .filter(|(painted, _)| painted == text)
            .nth(nth)
            .map(|(_, rect)| *rect)
            .unwrap_or_else(|| panic!("{text:?} is not painted: {:?}", self.painted));
        let target = rect.center();
        let from = target - egui::vec2(30.0, 30.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: target,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut steps: Vec<Vec<egui::Event>> = (1..=3)
            .map(|step| vec![pointer_at(from + (target - from) * step as f32 / 3.0)])
            .collect();
        steps.extend([vec![button(true)], vec![button(false)], Vec::new()]);
        let mut commands = Vec::new();
        for events in steps {
            self.frame(events);
            commands.extend(self.commands.iter().cloned());
        }
        commands
    }
}

fn pointer_at(point: egui::Pos2) -> egui::Event {
    egui::Event::PointerMoved(point)
}

fn wheel(dy: f32) -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, dy),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    }
}

/// A wheel that scrolls down for 40 frames, then up for 40, so a long
/// measurement neither runs out of content nor sits at one end.
fn ping_pong_wheel(frame: usize) -> f32 {
    if (frame / 40) % 2 == 0 {
        -WHEEL_STEP
    } else {
        WHEEL_STEP
    }
}

// ---------------------------------------------------------------------------
// Fixture: the only code that knows how app state is laid out
// ---------------------------------------------------------------------------

pub(super) mod fixture {
    use super::*;
    use crate::core::source::{LoadedSourceData, TagEntry, TagEntryLocation, TagSource};
    use blam_tags::render_method::{
        BitmapAddressMode, BitmapComparisonFunction, BitmapFilterMode, RenderMethod,
        RenderMethodDefinition, RenderMethodDefinitionCategory,
        RenderMethodDefinitionCategoryOption, RenderMethodOption, RenderMethodOptionParameter,
        RenderMethodParameterType,
    };
    use blam_tags::{Enum, TagFieldData, TagReferenceData, TagStructMut};

    pub(in crate::app::ui) const GAME: &str = "halo3_mcc";

    /// `folders` × `subfolders` × `tags` loose-file entries, as
    /// `folder_NN/sub_NN/tag_NNN.biped`. The defaults (40 × 10 × 150) are
    /// the 60,000 tags the browser virtualization tests use.
    pub(in crate::app::ui) fn synthetic_entries(
        folders: usize,
        subfolders: usize,
        tags: usize,
    ) -> Vec<TagEntry> {
        let mut entries = Vec::with_capacity(folders * subfolders * tags);
        for top in 0..folders {
            for sub in 0..subfolders {
                for tag in 0..tags {
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

    pub(in crate::app::ui) fn entry_key(display_path: &str) -> String {
        format!("file:{display_path}")
    }

    /// The browser entry for a document built in memory.
    pub(in crate::app::ui) fn document_entry(display_path: &str, tag: &TagFile) -> TagEntry {
        TagEntry {
            key: entry_key(display_path),
            display_path: display_path.to_owned(),
            group_tag: tag.header.group_tag,
            group_name: display_path
                .rsplit_once('.')
                .map(|(_, extension)| extension.to_owned()),
            location: TagEntryLocation::LooseFile(display_path.into()),
        }
    }

    /// Install `entries` as the active kit's source: an in-memory source,
    /// so the browser draws the full (non-lazy) tree and nothing is read
    /// off disk. This is the tree container and monolithic sources draw,
    /// which are the ones that reach tens of thousands of tags.
    pub(in crate::app::ui) fn install_kit(app: &mut Baboon, entries: Vec<TagEntry>) {
        install_kit_for_game(app, entries, GAME);
    }

    pub(in crate::app::ui) fn install_kit_for_game(
        app: &mut Baboon,
        entries: Vec<TagEntry>,
        game: &str,
    ) {
        app.install_loaded_source(LoadedSourceData {
            label: "perf".to_owned(),
            source: TagSource::SingleFile {
                path: PathBuf::from("perf-synthetic"),
            },
            names: app.default_names.clone(),
            game: GameId::from_id(game),
            tree: crate::core::source::build_tree(&entries),
            group_tree: crate::core::source::build_group_tree(&entries),
            all_entries: entries.clone(),
            entries,
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: true,
            chosen_kit_layout: None,
        });
        let kit = &mut app.kits[app.active];
        kit.browser_mode = BrowserMode::Folders;
    }

    /// Open `tag` in a tab, as if it had just finished loading. Its entry
    /// must already be in the kit (see [`document_entry`]).
    pub(in crate::app::ui) fn open_document(
        app: &mut Baboon,
        display_path: &str,
        tag: TagFile,
    ) -> String {
        let key = entry_key(display_path);
        let kit = &mut app.kits[app.active];
        kit.parsed_tags.insert(key.clone(), TagDocument::clean(tag));
        kit.open_tag_pane(&key);
        key
    }

    /// The tag pane's "Expand all" for `key`, applied on its next draw.
    pub(in crate::app::ui) fn expand_all(app: &mut Baboon, key: &str) {
        app.kits[app.active]
            .pending_expand
            .insert(key.to_owned(), true);
    }

    /// The browser search box's contents, as if typed.
    pub(in crate::app::ui) fn set_filter(app: &mut Baboon, text: &str) {
        app.kits[app.active].filter = text.to_owned();
    }

    /// "Reveal in browser": opens the tag's folders and scrolls to it.
    pub(in crate::app::ui) fn reveal(app: &mut Baboon, key: &str) {
        app.reveal_in_browser(key);
    }

    pub(in crate::app::ui) fn open_terminal(
        app: &mut Baboon,
        lines: impl IntoIterator<Item = String>,
    ) {
        app.kits[app.active].terminal_open = true;
        app.terminal.lines = lines.into_iter().map(TerminalLineEntry::new).collect();
        app.terminal.scroll_to_bottom = true;
    }

    /// One line of tool output arriving, with the app's own cap and
    /// autoscroll (see `push_terminal_line`).
    pub(in crate::app::ui) fn push_terminal_line(app: &mut Baboon, line: String) {
        app.terminal.lines.push(TerminalLineEntry::new(line));
        if app.terminal.lines.len() > 20_000 {
            let remove = app.terminal.lines.len() - 18_000;
            app.terminal.lines.drain(..remove);
        }
        app.terminal.scroll_to_bottom = true;
    }

    pub(in crate::app::ui) fn last_terminal_line(app: &Baboon) -> Option<String> {
        app.terminal.lines.last().map(|line| line.text.clone())
    }

    pub(in crate::app::ui) fn terminal_line(index: usize) -> String {
        format!(
            "{index}: tool.exe: importing C:\\Halo\\tags\\objects\\weapons\\rifle_{index}\\\
             render\\rifle_{index}.render_model from data\\objects\\weapons ... done"
        )
    }

    /// A new tag of `group` from this repository's definitions.
    pub(in crate::app::ui) fn new_tag(group: &str) -> TagFile {
        new_tag_for(GAME, group)
    }

    pub(in crate::app::ui) fn new_tag_for(game: &str, group: &str) -> TagFile {
        TagFile::new(
            locate_definitions_root()
                .join(game)
                .join(format!("{group}.json")),
        )
        .unwrap_or_else(|error| panic!("{game}/{group}.json: {error:?}"))
    }

    /// A Halo CE sound whose one pitch range holds `permutations`
    /// permutations of `seconds` of inline 16-bit PCM (the schema's default
    /// compression, big-endian; mono; 22 kHz): a sine sweep, so the
    /// waveform has shape. Inline samples are what CE plays from, so the
    /// player and its waveform work with no sound bank or audio files.
    pub(in crate::app::ui) fn synthetic_ce_sound(permutations: usize, seconds: f32) -> TagFile {
        let mut tag = new_tag_for("haloce_mcc", "sound");
        let frames = (22_050.0 * seconds) as usize;
        let mut root = tag.root_mut();
        let mut field = root.field_path_mut("pitch ranges").expect("pitch ranges");
        let mut ranges = field.as_block_mut().expect("pitch ranges is a block");
        ranges.add_element();
        let mut range = ranges.element_mut(0).expect("the pitch range");
        let mut field = range.field_path_mut("permutations").expect("permutations");
        let mut block = field.as_block_mut().expect("permutations is a block");
        for index in 0..permutations {
            block.add_element();
            let mut permutation = block.element_mut(index).expect("the permutation");
            permutation
                .field_path_mut("name")
                .expect("permutation name")
                .set(TagFieldData::String(format!("perm_{index}")))
                .expect("set the permutation name");
            let mut samples = Vec::with_capacity(frames * 2);
            for frame in 0..frames {
                let t = frame as f32 / 22_050.0;
                let envelope = 0.5 + 0.5 * (t * 0.7 + index as f32).sin();
                let value = (t * (220.0 + 40.0 * index as f32) * std::f32::consts::TAU).sin()
                    * envelope
                    * 20_000.0;
                samples.extend_from_slice(&(value as i16).to_be_bytes());
            }
            permutation
                .field_path_mut("samples")
                .expect("permutation samples")
                .set(TagFieldData::Data(samples))
                .expect("set the samples");
        }
        tag
    }

    /// Give every block in `tag_struct` `counts[0]` elements, and every
    /// block in each block's first element `counts[1]`, and so on down.
    pub(in crate::app::ui) fn populate_blocks(
        tag_struct: &mut TagStructMut<'_>,
        counts: &[usize],
    ) -> usize {
        let Some((&count, deeper)) = counts.split_first() else {
            return 0;
        };
        let ordinals: Vec<usize> = tag_struct
            .as_ref()
            .fields()
            .filter(|field| field.as_block().is_some())
            .map(|field| field.ordinal())
            .collect();
        let mut added = 0;
        for ordinal in ordinals {
            let Some(mut field) = tag_struct.field_at_mut(ordinal) else {
                continue;
            };
            let Some(mut block) = field.as_block_mut() else {
                continue;
            };
            let max = block.definition().max_count().max(1) as usize;
            for _ in 0..count.min(max) {
                block.add_element();
                added += 1;
            }
            if let Some(mut first) = block.element_mut(0) {
                added += populate_blocks(&mut first, deeper);
            }
        }
        added
    }

    /// A scenario whose every top-level block holds `counts[0]` elements and
    /// so on down (see [`populate_blocks`]). Returns it with its element
    /// count.
    pub(in crate::app::ui) fn large_scenario(counts: &[usize]) -> (TagFile, usize) {
        let mut tag = new_tag("scenario");
        let added = populate_blocks(&mut tag.root_mut(), counts);
        (tag, added)
    }

    /// A shader naming a render method definition of `categories`
    /// categories, each option declaring `parameters` parameters. The
    /// definition and options are put straight into the kit's caches — where
    /// the editor finds them once loaded — so nothing is read off disk.
    /// Returns the shader; [`install_render_method`] must run after the kit
    /// is installed.
    pub(in crate::app::ui) fn synthetic_shader(categories: usize) -> TagFile {
        let mut tag = new_tag("shader");
        {
            let mut root = tag.root_mut();
            root.field_path_mut("render_method/definition")
                .expect("shader has render_method/definition")
                .set(TagFieldData::TagReference(TagReferenceData {
                    group_tag_and_name: Some((
                        u32::from_be_bytes(*b"rmdf"),
                        "shaders\\perf_shader".to_owned(),
                    )),
                }))
                .expect("set the shader's definition");
            let mut field = root
                .field_path_mut("render_method/options")
                .expect("shader has render_method/options");
            let mut options = field.as_block_mut().expect("options is a block");
            for _ in 0..categories {
                options.add_element();
            }
        }
        tag
    }

    pub(in crate::app::ui) fn install_render_method(
        app: &mut Baboon,
        shader: &TagFile,
        categories: usize,
        options_per_category: usize,
        parameters: usize,
    ) {
        let render_method = RenderMethod::from_tag(shader).expect("synthetic shader parses");
        let kit = &mut app.kits[app.active];
        let mut definition_categories = Vec::new();
        for category in 0..categories {
            let mut options = Vec::new();
            for option in 0..options_per_category {
                let option_path = format!("shaders\\perf_options\\cat{category}_opt{option}");
                options.push(RenderMethodDefinitionCategoryOption {
                    option_name: format!("option_{option}"),
                    option_path: option_path.clone(),
                    vertex_function: String::new(),
                    pixel_function: String::new(),
                });
                let kinds = [
                    RenderMethodParameterType::Bitmap,
                    RenderMethodParameterType::Color,
                    RenderMethodParameterType::Real,
                    RenderMethodParameterType::Int,
                    RenderMethodParameterType::Bool,
                    RenderMethodParameterType::ArgbColor,
                ];
                let option_parameters = (0..parameters)
                    .map(|index| RenderMethodOptionParameter {
                        parameter_name: format!("perf_param_{category}_{index}"),
                        parameter_type: Some(Enum::from_variant(kinds[index % kinds.len()])),
                        source_extern: None,
                        default_bitmap_path: String::new(),
                        default_real_value: index as f32,
                        default_int_bool_value: 0,
                        flags: 0,
                        default_filter_mode: Enum::from_variant(BitmapFilterMode::Trilinear),
                        default_comparison_function: Enum::from_variant(
                            BitmapComparisonFunction::Never,
                        ),
                        default_address_mode: Enum::from_variant(BitmapAddressMode::Wrap),
                        anisotropy_amount: 0,
                        default_color: blam_tags::math::ArgbColor(0xff80_4020),
                        default_bitmap_scale: 1.0,
                        help_text: String::new(),
                    })
                    .collect();
                kit.rmop_cache.insert(
                    format!("rmop:{option_path}"),
                    Some(Arc::new(RenderMethodOption {
                        parameters: option_parameters,
                    })),
                );
            }
            definition_categories.push(RenderMethodDefinitionCategory {
                category_name: format!("perf_category_{category}"),
                vertex_function: String::new(),
                pixel_function: String::new(),
                options,
            });
        }
        kit.rmdf_cache.insert(
            format!("rmdf:{}", render_method.definition_path),
            Some(Arc::new(RenderMethodDefinition {
                global_options_path: String::new(),
                categories: definition_categories,
                shared_pixel_shaders_path: String::new(),
                shared_vertex_shaders_path: String::new(),
                flags: 0,
                version: 0,
            })),
        );
    }
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// What one scenario puts on screen, and what each measured frame does.
struct Scenario {
    name: &'static str,
    what: &'static str,
    /// Build the state, and settle it over as many frames as it needs. Not
    /// timed.
    setup: fn(&mut Harness),
    /// The events for measured (and warm-up) frame `index`; may also change
    /// app state, as typing or streaming output would.
    step: fn(&mut Harness, usize) -> Vec<egui::Event>,
    /// Evidence the scenario showed what it claims, from the last frame.
    /// A scenario whose setup silently drew nothing would otherwise time an
    /// empty window and report it as a baseline.
    check: fn(&Harness, &Measured) -> Result<(), String>,
}

const MIDDLE_TAG: &str = "folder_20/sub_05/tag_075.biped";

fn no_events(_: &mut Harness, _: usize) -> Vec<egui::Event> {
    Vec::new()
}

fn setup_kit_60k(h: &mut Harness) {
    fixture::install_kit(&mut h.app, fixture::synthetic_entries(40, 10, 150));
    h.idle(3);
}

/// Every one of the 400 subfolders opened (by revealing a tag in each, the
/// way "Reveal in browser" does), then the middle tag revealed: 60,440 rows
/// expanded, the viewport in the middle of them.
fn setup_browser_expanded(h: &mut Harness) {
    setup_kit_60k(h);
    for top in 0..40 {
        for sub in 0..10 {
            let key = fixture::entry_key(&format!("folder_{top:02}/sub_{sub:02}/tag_000.biped"));
            fixture::reveal(&mut h.app, &key);
            h.idle(2);
        }
    }
    fixture::reveal(&mut h.app, &fixture::entry_key(MIDDLE_TAG));
    h.idle(4);
}

fn setup_large_tag(h: &mut Harness) -> String {
    let (tag, _) = fixture::large_scenario(&[64, 8]);
    let path = "levels/perf/perf.scenario";
    let mut entries = fixture::synthetic_entries(4, 5, 50);
    entries.push(fixture::document_entry(path, &tag));
    fixture::install_kit(&mut h.app, entries);
    let key = fixture::open_document(&mut h.app, path, tag);
    h.idle(2);
    key
}

fn setup_large_tag_expanded(h: &mut Harness) {
    let key = setup_large_tag(h);
    fixture::expand_all(&mut h.app, &key);
    h.idle(3);
}

/// Typing `tag_075`, deleting back to `t`, then `tag_14`: every frame is a
/// new query, so every frame re-filters all 60,000 entries.
const FILTER_KEYSTROKES: [&str; 16] = [
    "t", "ta", "tag", "tag_", "tag_0", "tag_07", "tag_075", "tag_07", "tag_0", "tag_", "tag", "ta",
    "tag", "tag_", "tag_1", "tag_14",
];

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "idle_welcome",
            what: "no kit loaded; the welcome screen",
            setup: |h| h.idle(3),
            step: no_events,
            check: |h, _| {
                (!h.painted.is_empty())
                    .then_some(())
                    .ok_or_else(|| "nothing was painted".to_owned())
            },
        },
        Scenario {
            name: "idle_kit_60k",
            what: "60,000-tag kit loaded, folders collapsed, no tabs, no input",
            setup: setup_kit_60k,
            step: no_events,
            check: |h, _| {
                h.painted_contains("folder_00")
                    .then_some(())
                    .ok_or_else(|| "browser did not show folder_00".to_owned())
            },
        },
        Scenario {
            name: "browser_60k_expanded_static",
            what: "60,440 rows expanded, viewport at the middle, no input",
            setup: setup_browser_expanded,
            step: no_events,
            check: |h, _| {
                h.painted_contains("tag_075")
                    .then_some(())
                    .ok_or_else(|| format!("{MIDDLE_TAG} not in view after reveal"))
            },
        },
        Scenario {
            name: "browser_60k_wheel_scroll",
            what: "60,440 rows expanded, mouse wheel over the browser every frame",
            setup: setup_browser_expanded,
            step: |_, index| vec![pointer_at(BROWSER_POINT), wheel(ping_pong_wheel(index))],
            check: |_, measured| measured.scrolled(),
        },
        Scenario {
            name: "browser_filter_typing",
            what: "60,000-tag kit, the search query changes every frame",
            setup: setup_kit_60k,
            step: |h, index| {
                fixture::set_filter(
                    &mut h.app,
                    FILTER_KEYSTROKES[index % FILTER_KEYSTROKES.len()],
                );
                Vec::new()
            },
            check: |h, _| {
                (!h.painted_contains("No matching tags") && h.painted_contains("folder_"))
                    .then_some(())
                    .ok_or_else(|| "the filtered tree showed no matches".to_owned())
            },
        },
        Scenario {
            name: "tag_pane_scenario_static",
            what: "H3 scenario, every block 64 elements (first element's blocks 8), collapsed defaults, no input",
            setup: |h| {
                setup_large_tag(h);
            },
            step: no_events,
            check: |h, _| {
                h.painted_contains("perf.scenario")
                    .then_some(())
                    .ok_or_else(|| "the scenario tab did not draw".to_owned())
            },
        },
        Scenario {
            name: "tag_pane_scenario_expanded_static",
            what: "same scenario after Expand All, no input",
            setup: setup_large_tag_expanded,
            step: no_events,
            check: |h, _| {
                h.painted_contains("perf.scenario")
                    .then_some(())
                    .ok_or_else(|| "the scenario tab did not draw".to_owned())
            },
        },
        Scenario {
            name: "tag_pane_scenario_expanded_scroll",
            what: "same scenario after Expand All, mouse wheel over the pane every frame",
            setup: setup_large_tag_expanded,
            step: |_, index| vec![pointer_at(PANE_POINT), wheel(ping_pong_wheel(index))],
            check: |_, measured| measured.scrolled(),
        },
        Scenario {
            name: "shader_editor",
            what: "H3 shader: 12 categories x 4 options, 12 parameters per option (memoized model)",
            setup: |h| {
                let shader = fixture::synthetic_shader(12);
                let path = "shaders/perf.shader";
                let mut entries = fixture::synthetic_entries(4, 5, 50);
                entries.push(fixture::document_entry(path, &shader));
                fixture::install_kit(&mut h.app, entries);
                fixture::install_render_method(&mut h.app, &shader, 12, 4, 12);
                fixture::open_document(&mut h.app, path, shader);
                Counters::reset();
                h.idle(3);
            },
            step: no_events,
            check: |h, _| {
                h.painted_contains("PERF_CATEGORY_0")
                    .then_some(())
                    .ok_or_else(|| "the shader grid did not draw (raw-field fallback?)".to_owned())
            },
        },
        Scenario {
            name: "sound_player_ce_inline",
            what: "Halo CE sound, 24 permutations x 10 s inline PCM, player + waveform, no input",
            setup: |h| {
                let sound = fixture::synthetic_ce_sound(24, 10.0);
                let path = "sound/perf/perf.sound";
                let mut entries = fixture::synthetic_entries(4, 5, 50);
                entries.push(fixture::document_entry(path, &sound));
                fixture::install_kit_for_game(&mut h.app, entries, "haloce_mcc");
                fixture::open_document(&mut h.app, path, sound);
                // The idle player asks the audio worker for its waveform;
                // give the decode time to land.
                for _ in 0..20 {
                    h.idle(1);
                    std::thread::sleep(Duration::from_millis(25));
                }
            },
            step: no_events,
            check: |h, _| {
                // The clip's length is read from its inline samples: proof the
                // player found 10 s of PCM, not an empty tag.
                (h.painted_contains("24 permutations") && h.painted_contains("0:10.000"))
                    .then_some(())
                    .ok_or_else(|| "the sound player did not find the inline samples".to_owned())
            },
        },
        Scenario {
            name: "terminal_20k_static",
            what: "terminal open at the bottom of 20,000 lines (the app's cap), no input",
            setup: |h| {
                fixture::install_kit(&mut h.app, fixture::synthetic_entries(4, 5, 50));
                fixture::open_terminal(&mut h.app, (0..20_000).map(fixture::terminal_line));
                h.idle(4);
            },
            step: no_events,
            check: |h, _| {
                h.painted_contains("19999: ")
                    .then_some(())
                    .ok_or_else(|| "the last terminal line is not in view".to_owned())
            },
        },
        Scenario {
            name: "terminal_20k_streaming",
            what: "terminal at its 20,000-line cap, one new line and autoscroll every frame",
            setup: |h| {
                fixture::install_kit(&mut h.app, fixture::synthetic_entries(4, 5, 50));
                fixture::open_terminal(&mut h.app, (0..20_000).map(fixture::terminal_line));
                h.idle(4);
            },
            step: |h, index| {
                fixture::push_terminal_line(&mut h.app, fixture::terminal_line(1_000_000 + index));
                Vec::new()
            },
            check: |h, _| {
                // Autoscroll lands a frame after the line arrives, so the
                // newest line itself may be just below the view; one of the
                // last few streamed ones must be in it (scroll animation trails a
                // line-per-frame stream by ~3 lines).
                let last = fixture::last_terminal_line(&h.app).unwrap_or_default();
                let newest: usize = last
                    .split(':')
                    .next()
                    .and_then(|number| number.parse().ok())
                    .unwrap_or_default();
                (newest >= 1_000_000
                    && (newest - 8..=newest).any(|n| h.painted_contains(&format!("{n}: "))))
                .then_some(())
                .ok_or_else(|| format!("none of the newest lines (to {newest}) is in view"))
            },
        },
    ]
}

// ---------------------------------------------------------------------------
// Measurement and report
// ---------------------------------------------------------------------------

struct Measured {
    samples: Vec<FrameSample>,
    /// Painted text of the first and last measured frames.
    first_painted: Vec<String>,
    last_painted: Vec<String>,
}

impl Measured {
    /// For scroll scenarios: the view changed while measuring.
    fn scrolled(&self) -> Result<(), String> {
        (self.first_painted != self.last_painted)
            .then_some(())
            .ok_or_else(|| "the wheel did not move the view".to_owned())
    }

    fn millis(&self, of: impl Fn(&FrameSample) -> Duration) -> Vec<f64> {
        let mut values: Vec<f64> = self
            .samples
            .iter()
            .map(|sample| of(sample).as_secs_f64() * 1000.0)
            .collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        values
    }

    fn mean_counter(&self, of: impl Fn(&Counters) -> usize) -> f64 {
        let total: usize = self.samples.iter().map(|sample| of(&sample.counters)).sum();
        total as f64 / self.samples.len().max(1) as f64
    }
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

/// Nearest-rank percentile of sorted `values`.
fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * values.len() as f64).ceil() as usize;
    values[rank.clamp(1, values.len()) - 1]
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn run_scenario(scenario: &Scenario, warmup: usize, frames: usize) -> (Measured, Duration) {
    let setup_started = Instant::now();
    let mut harness = Harness::new();
    (scenario.setup)(&mut harness);
    // No hover unless a step puts the pointer somewhere: a pointer resting
    // on a field raises its tooltip after a delay, which would time the
    // tooltip rather than the scenario.
    harness.frame(vec![egui::Event::PointerGone]);
    let setup = setup_started.elapsed();
    for index in 0..warmup {
        let events = (scenario.step)(&mut harness, index);
        harness.frame(events);
    }
    let mut samples = Vec::with_capacity(frames);
    let mut first_painted = Vec::new();
    for index in 0..frames {
        let events = (scenario.step)(&mut harness, warmup + index);
        samples.push(harness.frame(events));
        if index == 0 {
            first_painted = harness.painted.clone();
        }
    }
    let measured = Measured {
        samples,
        first_painted,
        last_painted: harness.painted.clone(),
    };
    if std::env::var_os("BABOON_PERF_DUMP").is_some() {
        eprintln!(
            "[perf] {}: last frame painted {} texts: {:?}",
            scenario.name,
            harness.painted.len(),
            harness.painted
        );
    }
    if let Err(problem) = (scenario.check)(&harness, &measured) {
        panic!("{}: {problem}", scenario.name);
    }
    (measured, setup)
}

/// Frame-time table for every scenario. `#[ignore]`d: it takes minutes in a
/// debug build and its numbers mean nothing there or on a busy machine.
#[test]
#[ignore]
fn perf_baseline() {
    let warmup = env_usize("BABOON_PERF_WARMUP", 20);
    let frames = env_usize("BABOON_PERF_FRAMES", 120).max(1);
    let only: Vec<String> = std::env::var("BABOON_PERF_ONLY")
        .map(|value| value.split(',').map(str::trim).map(str::to_owned).collect())
        .unwrap_or_default();
    let label = std::env::var("BABOON_PERF_LABEL").unwrap_or_default();
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let ppp = Harness::new().pixels_per_point;

    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for scenario in scenarios() {
        if !only.is_empty()
            && !only
                .iter()
                .any(|part| scenario.name.contains(part.as_str()))
        {
            continue;
        }
        eprintln!("[perf] {} — {}", scenario.name, scenario.what);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_scenario(&scenario, warmup, frames)
        }));
        match result {
            Ok((measured, setup)) => rows.push((scenario.name, measured, setup)),
            Err(panic) => {
                let message = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_else(|| "panicked".to_owned());
                eprintln!("[perf] {} FAILED: {message}", scenario.name);
                failures.push(format!("{}: {message}", scenario.name));
            }
        }
    }

    eprintln!();
    eprintln!(
        "Baboon frame-time baseline — {profile} build, {warmup} warm-up + {frames} measured frames, \
         {}x{} points @ {ppp} ppp{}",
        SCREEN.x,
        SCREEN.y,
        if label.is_empty() {
            String::new()
        } else {
            format!(", label `{label}`")
        }
    );
    eprintln!(
        "{:<34} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} | {:>8} {:>7} {:>7} {:>7} {:>6} | {:>8}",
        "scenario",
        "frames",
        "mean",
        "median",
        "p95",
        "max",
        "run",
        "tess",
        "treeRows",
        "fnPrev",
        "ddLbl",
        "termLn",
        "shader",
        "setup s"
    );
    let csv_path = std::env::var("BABOON_PERF_CSV").ok();
    let mut csv = String::new();
    for (name, measured, setup) in &rows {
        let total = measured.millis(FrameSample::total);
        let run = measured.millis(|sample| sample.run);
        let tess = measured.millis(|sample| sample.tessellate);
        let line = format!(
            "{:<34} {:>6} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} | {:>8.1} {:>7.1} {:>7.1} {:>7.1} {:>6.2} | {:>8.1}",
            name,
            measured.samples.len(),
            mean(&total),
            percentile(&total, 50.0),
            percentile(&total, 95.0),
            total.last().copied().unwrap_or_default(),
            mean(&run),
            mean(&tess),
            measured.mean_counter(|c| c.tree_rows),
            measured.mean_counter(|c| c.function_previews),
            measured.mean_counter(|c| c.dropdown_labels),
            measured.mean_counter(|c| c.terminal_lines),
            measured.mean_counter(|c| c.shader_models),
            setup.as_secs_f64(),
        );
        eprintln!("{line}");
        csv.push_str(&format!(
            "{label},{profile},{ppp},{name},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.2},{:.2},{:.2},{:.2},{:.3}\n",
            measured.samples.len(),
            mean(&total),
            percentile(&total, 50.0),
            percentile(&total, 95.0),
            total.last().copied().unwrap_or_default(),
            mean(&run),
            mean(&tess),
            measured.mean_counter(|c| c.tree_rows),
            measured.mean_counter(|c| c.function_previews),
            measured.mean_counter(|c| c.dropdown_labels),
            measured.mean_counter(|c| c.terminal_lines),
            measured.mean_counter(|c| c.shader_models),
        ));
    }
    eprintln!(
        "(ms per frame; run = ctx.run around Baboon::run_frame, tess = ctx.tessellate; counters are per-frame means)"
    );
    if let Some(path) = csv_path {
        use std::io::Write;
        let exists = std::path::Path::new(&path).exists();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("open BABOON_PERF_CSV");
        if !exists {
            writeln!(
                file,
                "label,profile,ppp,scenario,frames,mean_ms,median_ms,p95_ms,max_ms,run_ms,tess_ms,\
                 tree_rows,function_previews,dropdown_labels,terminal_lines,shader_models"
            )
            .unwrap();
        }
        file.write_all(csv.as_bytes()).unwrap();
        eprintln!("appended to {path}");
    }
    assert!(failures.is_empty(), "scenarios failed: {failures:#?}");
}
