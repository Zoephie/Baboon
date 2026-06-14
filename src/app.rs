use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use blam_tags::bitmap::decode::decode_to_rgba8;
use blam_tags::paths::{derive_tags_root, group_tag_to_extension, resolve_tag_path, tag_ref_path};
use blam_tags::render_method::{
    GlobalRenderMethodFlags, RenderMethod, RenderMethodAnimatedParameter,
    RenderMethodAnimatedParameterType, RenderMethodDefinition, RenderMethodOption,
    RenderMethodOptionParameter, RenderMethodParameter, RenderMethodParameterType,
    compile_real_constant,
};
use blam_tags::{
    AssFile, Bitmap, ColorGraphType, Endian, FunctionFlags, FunctionType, JmsFile, RenderModel,
    RenderModelPreview, StringIdData, TagBlock, TagField, TagFieldData, TagFieldType, TagFile,
    TagFunction, TagReferenceData, TagResource, TagResourceKind, TagStruct, format_group_tag,
    parse_group_tag,
};
use eframe::egui::{
    self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Frame, RichText,
    ScrollArea, Sense, Stroke, TextStyle, Ui, Vec2,
};
use serde_json::{Value, json};

use crate::format::{TagNameIndex, format_value, group_label};
use crate::source::{
    DependencyRef, LoadedSourceData, ReverseDependencyIndex, TagEntry, TagEntryLocation, TagSource,
    TagTree, TagTreeNode, load_folder, load_folder_node_entries, load_monolithic_blob_index,
    load_single_file, read_entry, resolve_folder_root, scan_folder_subtree_entries,
};

pub(super) const BABOON_GITHUB_URL: &str = "https://github.com/Zoephie/Baboon";
pub(super) const BABOON_LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/Zoephie/Baboon/releases/latest";
pub(super) const BABOON_RELEASES_URL: &str = "https://github.com/Zoephie/Baboon/releases";

mod style;
use style::*;
mod state;
use state::*;
mod prefs;
use prefs::*;
mod browser;
use browser::*;
mod export;
use export::*;
mod function_editor;
use function_editor::*;
mod foundation;
use foundation::*;
mod shader;
use shader::*;
mod material;
use material::*;
mod model_preview;
use model_preview::*;
mod map_names;
use map_names::*;
mod editor;
use editor::*;
mod controller;
mod ui;

pub struct Baboon {
    default_names: TagNameIndex,
    names: TagNameIndex,
    tx: Sender<WorkerMessage>,
    rx: Receiver<WorkerMessage>,
    source: Option<LoadedSourceData>,
    parsed_tags: HashMap<String, TagDocument>,
    tag_cache_order: VecDeque<String>,
    loading_tags: HashSet<String>,
    selected_key: Option<String>,
    open_tabs: Vec<String>,
    floating_tabs: HashSet<String>,
    bitmap_previews: HashMap<String, BitmapPreviewState>,
    model_previews: HashMap<String, ModelPreviewState>,
    edit_buffers: HashMap<String, String>,
    rmdf_cache: HashMap<String, Option<RenderMethodDefinition>>,
    rmop_cache: HashMap<String, Option<RenderMethodOption>>,
    filter: String,
    /// Per-tag "Search fields" query (keyed by tag key). Collapses the tag
    /// editor down to the matching block(s) and their ancestors.
    field_search: HashMap<String, String>,
    /// The last query actually applied per tag, so the collapse is a one-shot
    /// on change rather than a per-frame override the user can't fight.
    field_search_applied: HashMap<String, String>,
    /// Cached search results, recomputed only when the query or the underlying
    /// entry set changes — never per frame. See [`FilterCache`].
    filter_cache: FilterCache,
    /// Bumped whenever the active source or its `all_entries` set is replaced,
    /// so [`filter_cache`] knows to recompute against fresh data.
    source_generation: u64,
    browser_mode: BrowserMode,
    show_browser_prefixes: bool,
    double_click_to_open_tags: bool,
    show_block_sizes: bool,
    expert_mode: bool,
    dark_mode: bool,
    ui_scale: f32,
    pending_ui_scale: f32,
    model_preview_size: f32,
    saved_prefs: GuiPrefs,
    settings_open: bool,
    new_tag_open: bool,
    new_tag_dialog: NewTagDialog,
    about_open: bool,
    help_panel_tab: HelpPanelTab,
    map_names_game_tab: MapNamesGameTab,
    blender_path: Option<PathBuf>,
    blender_path_input: String,
    color_popup: Option<MaterialColorPopup>,
    function_popup: Option<FunctionPopup>,
    status: String,
    folder_refactor: Option<FolderRefactorUiState>,
    /// True while a background full-scan of a loose-folder source is running.
    scanning_entries: bool,
    terminal: TerminalState,
    terminal_open: bool,
    /// Working directory for terminal commands (game kit root, parent of tags/).
    terminal_work_dir: Option<std::path::PathBuf>,
    /// Game identifiers (e.g. "halo3_mcc") for which the user has chosen to
    /// keep the terminal open. Persisted in prefs.json and restored per kit.
    terminal_open_games: HashSet<String>,
    saved_terminal_open_games: HashSet<String>,
    dragging_floating_tab: Option<String>,
    tab_rack_rect: Option<egui::Rect>,
    /// Pending destructive block op (delete / delete all) awaiting confirm.
    block_confirm: Option<BlockConfirm>,
    /// Pending "open referenced tag in a new tab" request.
    pending_open: Option<OpenTagRequest>,
    /// Pending "import geometry via tool" request from an Import button.
    pending_tool_import: Option<ToolImportRequest>,
    /// Toolbar launcher icons (decoded from embedded .ico at startup).
    sapien_icon: Option<egui::TextureHandle>,
    tag_test_icon: Option<egui::TextureHandle>,
    /// Clipboard for copy/paste of a block element between identical tags.
    block_clipboard: Option<BlockClipboard>,
}

impl Baboon {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_fonts(foundation_fonts());
        cc.egui_ctx.set_style(foundation_style());
        let prefs = load_gui_prefs();
        let terminal_open_games = load_terminal_open_games();
        set_dark_mode(prefs.dark_mode);
        cc.egui_ctx.set_visuals(foundation_visuals());
        let mut names = TagNameIndex::load_from_definitions(&locate_definitions_root());
        // Fill any gaps (or the whole index, if the folder wasn't found) from
        // the group-name tables baked into the binary.
        names.merge_missing(TagNameIndex::embedded_fallback());
        let (tx, rx) = mpsc::channel();
        Self {
            default_names: names.clone(),
            names,
            tx,
            rx,
            source: None,
            parsed_tags: HashMap::new(),
            tag_cache_order: VecDeque::new(),
            loading_tags: HashSet::new(),
            selected_key: None,
            open_tabs: Vec::new(),
            floating_tabs: HashSet::new(),
            bitmap_previews: HashMap::new(),
            model_previews: HashMap::new(),
            edit_buffers: HashMap::new(),
            rmdf_cache: HashMap::new(),
            rmop_cache: HashMap::new(),
            filter: String::new(),
            field_search: HashMap::new(),
            field_search_applied: HashMap::new(),
            filter_cache: FilterCache::default(),
            source_generation: 0,
            browser_mode: prefs.browser_mode,
            show_browser_prefixes: prefs.show_browser_prefixes,
            double_click_to_open_tags: prefs.double_click_to_open_tags,
            show_block_sizes: prefs.show_block_sizes,
            expert_mode: prefs.expert_mode,
            dark_mode: prefs.dark_mode,
            ui_scale: prefs.ui_scale,
            pending_ui_scale: prefs.ui_scale,
            model_preview_size: prefs.model_preview_size,
            saved_prefs: prefs.clone(),
            settings_open: false,
            new_tag_open: false,
            new_tag_dialog: NewTagDialog::default(),
            about_open: false,
            help_panel_tab: HelpPanelTab::About,
            map_names_game_tab: MapNamesGameTab::HaloCe,
            blender_path_input: prefs
                .blender_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            blender_path: prefs.blender_path,
            color_popup: None,
            function_popup: None,
            status: "Ready".to_owned(),
            folder_refactor: None,
            scanning_entries: false,
            terminal: TerminalState {
                input: String::new(),
                lines: Vec::new(),
                history: Vec::new(),
                history_cursor: None,
                refocus_input: false,
                running: false,
                scroll_to_bottom: false,
            },
            terminal_open: false,
            terminal_work_dir: None,
            saved_terminal_open_games: terminal_open_games.clone(),
            terminal_open_games,
            dragging_floating_tab: None,
            tab_rack_rect: None,
            block_confirm: None,
            pending_open: None,
            pending_tool_import: None,
            sapien_icon: load_ico_texture(
                &cc.egui_ctx,
                "sapien_icon",
                include_bytes!("../icons/sapien.ico"),
            ),
            tag_test_icon: load_ico_texture(
                &cc.egui_ctx,
                "tag_test_icon",
                include_bytes!("../icons/tag_test.ico"),
            ),
            block_clipboard: None,
        }
    }
}

/// Locate the bundled `definitions/` folder, which carries the per-game group
/// name → file extension index. Source builds use the submodule copy at
/// `blam-tags/definitions`; release builds may put `definitions` beside the
/// executable. Without this the name index is empty, which breaks tag-reference
/// Open and the geometry Import button (both rely on resolving the referenced
/// group's extension).
pub(super) fn locate_definitions_root() -> PathBuf {
    for candidate in [
        PathBuf::from("blam-tags").join("definitions"),
        PathBuf::from("definitions"),
    ] {
        if candidate.is_dir() {
            return candidate;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        for _ in 0..4 {
            let Some(d) = dir else { break };
            for candidate in [
                d.join("definitions"),
                d.join("blam-tags").join("definitions"),
            ] {
                if candidate.is_dir() {
                    return candidate;
                }
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    PathBuf::from("blam-tags").join("definitions")
}

/// Decode an embedded `.ico` into an egui texture for a toolbar button.
fn load_ico_texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Ico).ok()?;
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR))
}

#[cfg(test)]
mod tests {
    use crate::app::controller::{new_tag_output_path, new_tag_output_path_from_dialog};

    use super::*;

    #[test]
    fn strip_node_indices_drops_element_subscripts() {
        // Search-fields paths must be element-independent so a match resolves
        // regardless of which block element happens to be selected.
        assert_eq!(strip_node_indices("contact points"), "contact points");
        assert_eq!(
            strip_node_indices("contact points[0]/markers[12]"),
            "contact points/markers"
        );
        assert_eq!(strip_node_indices("unit/object"), "unit/object");
    }

    #[test]
    fn tag_ref_path_helpers() {
        // Null terminator stripped so the ref resolves on disk.
        assert_eq!(
            sanitize_ref_path("objects\\characters\\masterchief\\masterchief\u{0}"),
            "objects\\characters\\masterchief\\masterchief"
        );
        // tool source dir is the parent of the tag path.
        assert_eq!(
            model_source_dir("objects\\characters\\masterchief\\masterchief"),
            "objects\\characters\\masterchief"
        );
        assert_eq!(model_source_dir("solo"), "solo");
    }

    #[test]
    fn color_channel_parsing_matches_swatch_output() {
        // The color picker writes "r, g, b" / "a, r, g, b"; the field parser
        // must accept exactly that and reject the wrong channel count.
        let [r, g, b] = parse_color_channels::<3>("0.14902, 0.662745, 0.145098").unwrap();
        assert!((r - 0.14902).abs() < 1e-6);
        assert!((g - 0.662745).abs() < 1e-6);
        assert!((b - 0.145098).abs() < 1e-6);

        let [a, r, _, _] = parse_color_channels::<4>("0.5, 1.0, 0.0, 0.25").unwrap();
        assert_eq!(a, 0.5);
        assert_eq!(r, 1.0);
        assert_eq!(
            parse_rgb_or_argb_color_channels("1.0, 0.0, 0.25").unwrap(),
            (1.0, 1.0, 0.0, 0.25)
        );
        assert_eq!(
            parse_rgb_or_argb_color_channels("0.5, 1.0, 0.0, 0.25").unwrap(),
            (0.5, 1.0, 0.0, 0.25)
        );
        assert_eq!(color_float_to_u8(1.0), 255);
        assert_eq!(color_float_to_u8(0.0), 0);
        assert_eq!(color_float_to_u8(0.5), 128);

        assert!(parse_color_channels::<3>("0.1, 0.2").is_err());
        assert!(parse_color_channels::<3>("0.1, 0.2, x").is_err());
        assert!(parse_rgb_or_argb_color_channels("0.1, 0.2").is_err());
    }

    #[test]
    fn hex_blob_ferry_round_trips() {
        // The shader function editor ships edited blobs through the string
        // edit channel as hex. encode_hex -> decode_hex must be lossless for
        // arbitrary bytes, or saved shader functions would corrupt.
        let blob: Vec<u8> = (0u16..=255).map(|b| b as u8).collect();
        let encoded = encode_hex(&blob);
        assert_eq!(encoded.len(), blob.len() * 2);
        let decoded = decode_hex(&encoded).expect("decode hex");
        assert_eq!(decoded, blob);
        // Odd-length / invalid input is rejected, not silently truncated.
        assert!(decode_hex("abc").is_err());
        assert!(decode_hex("zz").is_err());
    }

    #[test]
    fn bitmap_channel_filter_supports_alpha_only_view() {
        let data = BitmapPreviewData {
            width: 1,
            height: 1,
            image_count: 1,
            format_name: "a8r8g8b8".to_owned(),
            type_name: "2D texture".to_owned(),
            rgba: vec![10, 20, 30, 128],
        };
        let mut preview = BitmapPreviewState::default();
        preview.show_red = false;
        preview.show_green = false;
        preview.show_blue = false;
        preview.show_alpha = true;

        assert_eq!(
            filtered_bitmap_rgba(&data, &preview),
            vec![128, 128, 128, 255]
        );
    }

    #[test]
    fn folder_bitmap_collector_finds_nested_bitmap_entries() {
        let entries = vec![
            TagEntry {
                key: "bitmap".into(),
                display_path: "objects/test/diffuse.bitmap".into(),
                group_tag: u32::from_be_bytes(*b"bitm"),
                group_name: Some("bitmap".into()),
                location: TagEntryLocation::LooseFile(PathBuf::from("diffuse.bitmap")),
            },
            TagEntry {
                key: "model".into(),
                display_path: "objects/test/object.model".into(),
                group_tag: u32::from_be_bytes(*b"hlmt"),
                group_name: Some("model".into()),
                location: TagEntryLocation::LooseFile(PathBuf::from("object.model")),
            },
        ];
        let node = TagTreeNode {
            label: "objects".into(),
            rel_path: PathBuf::from("objects"),
            entries: vec![],
            children: vec![TagTreeNode {
                label: "test".into(),
                rel_path: PathBuf::from("objects/test"),
                entries: vec![0, 1],
                children: vec![],
                children_loaded: true,
                entries_loaded: true,
            }],
            children_loaded: true,
            entries_loaded: true,
        };

        assert_eq!(collect_bitmap_keys(&node, &entries), vec!["bitmap"]);
    }

    #[test]
    fn folder_json_path_preserves_tag_extension_under_source_tree() {
        let entry = TagEntry {
            key: "model".into(),
            display_path: "objects/test/spartans.model".into(),
            group_tag: u32::from_be_bytes(*b"hlmt"),
            group_name: Some("model".into()),
            location: TagEntryLocation::LooseFile(PathBuf::from("spartans.model")),
        };

        assert_eq!(
            tag_json_relative_path(&entry),
            PathBuf::from("objects/test/spartans.model.json")
        );
    }

    #[test]
    fn shader_function_summary_uses_curve_points_instead_of_placeholder() {
        let mut blob = vec![0u8; 32];
        blob[0] = 5; // LinearKey
        blob[4..8].copy_from_slice(&0.0f32.to_le_bytes());
        blob[8..12].copy_from_slice(&1.0f32.to_le_bytes());
        for &(x, y) in &[(0.0_f32, 0.0_f32), (0.25, 1.0), (0.75, 1.0), (1.0, 0.0)] {
            blob.extend_from_slice(&x.to_le_bytes());
            blob.extend_from_slice(&y.to_le_bytes());
        }
        for _ in 0..4 {
            blob.extend_from_slice(&0.0_f32.to_le_bytes());
        }
        for _ in 0..4 {
            blob.extend_from_slice(&0.0_f32.to_le_bytes());
        }
        for &v in &[1.0_f32, 0.0, -1.0, 0.0] {
            blob.extend_from_slice(&v.to_le_bytes());
        }

        let function = TagFunction::parse(&blob).unwrap();
        let summary = shader_function_grid_text(&function);

        assert!(summary.contains("curve:"));
        assert!(summary.contains("(0.25, 1.0)"));
        assert!(!summary.contains("function data goes here"));
    }

    #[test]
    fn shader_function_summary_reads_static_color_data() {
        let mut blob = [0u8; 32];
        blob[0] = 1; // Constant
        blob[2] = ColorGraphType::OneColor as u8;
        blob[4..8].copy_from_slice(&0xFF_33_66_99u32.to_le_bytes());

        let function = TagFunction::parse(&blob).unwrap();
        let summary = shader_function_grid_text(&function);

        assert!(summary.contains("RGB"));
        assert!(summary.contains("0.2, 0.4, 0.6"));
    }

    #[test]
    fn shader_constant_color_with_unset_alpha_displays_opaque() {
        let mut blob = [0u8; 32];
        blob[0] = 1; // Constant
        blob[2] = ColorGraphType::TwoColor as u8;
        blob[4..8].copy_from_slice(&0x00_C0_C0_C0u32.to_le_bytes());
        blob[16..20].copy_from_slice(&0x00_00_00_00u32.to_le_bytes());

        let function = TagFunction::parse(&blob).unwrap();
        let [r, g, b, a] = extract_constant_color(&function).unwrap();

        assert_eq!(
            (r, g, b, a),
            (192.0 / 255.0, 192.0 / 255.0, 192.0 / 255.0, 1.0)
        );
    }

    #[test]
    fn generated_constant_color_functions_use_render_method_gpu_flag() {
        let bytes = decode_hex(&constant_color_function_hex(1.0, 0.0, 0.0, 1.0)).unwrap();
        let function = TagFunction::parse(&bytes).unwrap();

        assert_eq!(function.color_graph_type(), ColorGraphType::OneColor);
        assert!(function.flags().is_gpu());
        assert_eq!(function.header().colors[0], 0xFFFF0000);
    }

    #[test]
    fn new_tag_output_path_stays_under_tags_root() {
        let root = Path::new("tags");

        assert_eq!(
            new_tag_output_path(root, "objects/test/example.shader", "shader").unwrap(),
            PathBuf::from("tags/objects/test/example.shader")
        );
        assert_eq!(
            new_tag_output_path(root, "objects\\test\\example", "shader").unwrap(),
            PathBuf::from("tags/objects/test/example.shader")
        );
        assert!(new_tag_output_path(root, "../escape", "shader").is_err());
        assert!(new_tag_output_path(root, "C:/escape", "shader").is_err());
    }

    #[test]
    fn new_tag_dialog_path_uses_selected_file_name_inside_tags_root() {
        let root = Path::new("C:/kit/tags");

        let (output, display) =
            new_tag_output_path_from_dialog(root, Path::new("C:/kit/tags/objects/foo"), "shader")
                .unwrap();
        assert_eq!(output, PathBuf::from("C:/kit/tags/objects/foo.shader"));
        assert_eq!(display, "objects/foo.shader");

        let (output, display) = new_tag_output_path_from_dialog(
            root,
            Path::new("C:/kit/tags/objects/foo.model"),
            "shader",
        )
        .unwrap();
        assert_eq!(output, PathBuf::from("C:/kit/tags/objects/foo.shader"));
        assert_eq!(display, "objects/foo.shader");

        assert!(
            new_tag_output_path_from_dialog(root, Path::new("C:/other/foo.shader"), "shader")
                .is_err()
        );
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
