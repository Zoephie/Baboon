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
    AssFile, Bitmap, ColorGraphType, Endian, FunctionFlags, FunctionKind, FunctionType, JmsFile,
    RenderModel, StringIdData, TagBlock, TagField, TagFieldData, TagFieldType, TagFile,
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

#[cfg(test)]
pub(super) fn test_definition_path(rel: &str) -> PathBuf {
    let local = Path::new("definitions").join(rel);
    if local.is_file() {
        return local;
    }
    let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) else {
        return local;
    };
    let checkouts = Path::new(&home).join(".cargo/git/checkouts");
    let Ok(entries) = fs::read_dir(checkouts) else {
        return local;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("blam-tags-") {
            continue;
        }
        let Ok(revs) = fs::read_dir(entry.path()) else {
            continue;
        };
        for rev in revs.flatten() {
            let candidate = rev.path().join("definitions").join(rel);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    local
}

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

/// Locate the on-disk `definitions/` folder, which carries the per-game tag
/// layouts and group name → file extension index. Source builds place it at
/// `definitions/` in the working directory (it is too large — ~1 GB — to vendor
/// in-repo, and now that blam-tags is an external crate it is no longer carried
/// by a submodule); release builds may put `definitions` beside the executable.
/// Without this the name index falls back to the small embedded meta tables, so
/// tag-reference Open and the geometry Import button still resolve common
/// groups, but full per-tag layouts won't load.
pub(super) fn locate_definitions_root() -> PathBuf {
    for candidate in [
        PathBuf::from("definitions"),
        PathBuf::from("blam-tags").join("definitions"),
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
    PathBuf::from("definitions")
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
    fn function_edit_data_field_still_emits_hex_field_edit() {
        let bytes = decode_hex(&constant_function_hex(0.25)).unwrap();
        let function = TagFunction::parse(&bytes).unwrap();
        let view = FunctionView::from_function(function).with_edit(FunctionEditPaths {
            data: FunctionDataStorage::DataField("function/data".to_owned()),
            parameter_type: String::new(),
            input_name: String::new(),
            range_name: String::new(),
            time_period: String::new(),
            block_path: String::new(),
            block_index: 0,
        });
        let previous_function =
            TagFunction::parse(&decode_hex(&constant_function_hex(0.0)).unwrap()).unwrap();
        let previous = FunctionSnapshot::from_view(&FunctionView::from_function(previous_function));

        let batch = push_function_edit(view.edit.as_ref().unwrap(), &previous, &view);

        assert_eq!(batch.data_ops.len(), 0);
        assert_eq!(batch.edits.len(), 1);
        assert_eq!(batch.edits[0].path, "function/data");
        assert_eq!(batch.edits[0].input, encode_hex(&bytes));
    }

    #[test]
    fn function_edit_halo2_byte_block_emits_data_op() {
        let bytes = decode_hex(&constant_function_hex(0.5)).unwrap();
        let function = TagFunction::parse(&bytes).unwrap();
        let view = FunctionView::from_function(function).with_edit(FunctionEditPaths {
            data: FunctionDataStorage::Halo2ByteBlock("parameters[0]/function/data".to_owned()),
            parameter_type: String::new(),
            input_name: String::new(),
            range_name: String::new(),
            time_period: String::new(),
            block_path: String::new(),
            block_index: 0,
        });
        let previous_function =
            TagFunction::parse(&decode_hex(&constant_function_hex(0.0)).unwrap()).unwrap();
        let previous = FunctionSnapshot::from_view(&FunctionView::from_function(previous_function));

        let batch = push_function_edit(view.edit.as_ref().unwrap(), &previous, &view);

        assert!(batch.edits.is_empty());
        assert_eq!(batch.data_ops.len(), 1);
        assert_eq!(batch.data_ops[0].block_path, "parameters[0]/function/data");
        assert_eq!(batch.data_ops[0].data, bytes);
    }

    #[test]
    fn halo2_function_byte_block_replacement_roundtrips_bytes() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let bytes = decode_hex(&constant_function_hex(-0.25)).unwrap();

        seed_halo2_function_byte_block_for_test(
            &mut tag,
            "parameters[0]/animation properties[0]/function/data",
            &bytes,
        );

        let mapping = tag
            .root()
            .descend("parameters[0]/animation properties[0]/function")
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), bytes);
        let function = TagFunction::parse(&bytes).unwrap();
        let reparsed =
            TagFunction::parse(&halo2_function_bytes_from_struct(mapping).unwrap()).unwrap();
        assert_eq!(reparsed.to_bytes(), function.to_bytes());
    }

    #[test]
    fn classic_halo2_shader_model_exposes_byte_block_function_row() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
        tag.container = blam_tags::file::TagContainer::Classic {
            engine: blam_tags::classic::ClassicEngine::Halo2V4,
            header: vec![0; 64],
        };
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let bytes = decode_hex(&constant_function_hex(0.75)).unwrap();
        seed_halo2_function_byte_block_for_test(
            &mut tag,
            "parameters[0]/animation properties[0]/function/data",
            &bytes,
        );

        let model = build_classic_shader_editor_model(&tag, &TagNameIndex::default()).unwrap();
        let (function_bytes, path) = first_halo2_byte_block_function_row(&model).unwrap();

        assert_eq!(function_bytes, bytes);
        assert_eq!(path, "parameters[0]/animation properties[0]/function/data");
    }

    #[test]
    fn h2ek_shader_model_routes_only_classic_halo2_shader_family() {
        let entry = h2_shader_entry(u32::from_be_bytes(*b"rmsh"));
        let mut classic = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
        classic.container = blam_tags::file::TagContainer::Classic {
            engine: blam_tags::classic::ClassicEngine::Halo2V4,
            header: vec![0; 64],
        };
        assert!(
            build_h2ek_shader_editor_model(&classic, &entry, &TagNameIndex::default(), None)
                .is_some()
        );

        let mcc = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
        assert!(
            build_h2ek_shader_editor_model(&mcc, &entry, &TagNameIndex::default(), None).is_none()
        );

        let non_shader = classic;
        let non_shader_entry = h2_shader_entry(u32::from_be_bytes(*b"bitm"));
        assert!(
            build_h2ek_shader_editor_model(
                &non_shader,
                &non_shader_entry,
                &TagNameIndex::default(),
                None
            )
            .is_none()
        );
    }

    #[test]
    fn h2ek_shader_model_exposes_schema_backed_value_rows() {
        let mut tag = h2_classic_shader_tag();
        apply_field_edit(
            &mut tag,
            "template",
            "stem:shaders/shader_templates/transparent/plasma_mask_offset",
        )
        .unwrap();
        apply_field_edit(&mut tag, "material name", "test_material").unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut tag, "parameters[0]/name", "diffuse_map").unwrap();
        apply_field_edit(&mut tag, "parameters[0]/type", "1").unwrap();
        apply_field_edit(&mut tag, "parameters[0]/const value", "0.5").unwrap();

        let model = build_h2ek_shader_editor_model(
            &tag,
            &h2_shader_entry(u32::from_be_bytes(*b"rmsh")),
            &TagNameIndex::default(),
            None,
        )
        .unwrap();

        let material_name = shader_row_edit_path_and_kind(&model, "material_name").unwrap();
        assert_eq!(material_name, ("material name".to_owned(), "string_id"));

        let template = shader_row_edit_path_and_kind(&model, "template").unwrap();
        assert_eq!(template, ("template".to_owned(), "string_id"));

        let const_value = shader_row_edit_path_and_kind(&model, "diffuse_map").unwrap();
        assert_eq!(
            const_value,
            ("parameters[0]/const value".to_owned(), "scalar")
        );
    }

    #[test]
    fn h2ek_shader_template_reference_accepts_h2ek_extension_path() {
        let mut tag = h2_classic_shader_tag();
        apply_field_edit(
            &mut tag,
            "template",
            "stem:shaders\\shader_templates\\transparent\\plasma_mask_offset.shader_template",
        )
        .unwrap();

        assert_eq!(
            h2_shader_template_reference_for_test(&tag).as_deref(),
            Some("shaders\\shader_templates\\transparent\\plasma_mask_offset")
        );
    }

    #[test]
    fn h2ek_shader_template_rows_drive_visible_parameters() {
        let mut shader = h2_classic_shader_tag();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut shader, "parameters[0]/name", "self_illum_color").unwrap();
        apply_field_edit(&mut shader, "parameters[0]/type", "2").unwrap();

        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        for index in 0..2 {
            apply_one_block_op(
                &mut template,
                &BlockOp {
                    path: "categories[0]/parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            let parameter_path = format!("categories[0]/parameters[{index}]");
            let name = if index == 0 {
                "noise_map1"
            } else {
                "plasma_mask"
            };
            apply_field_edit(&mut template, &format!("{parameter_path}/name"), name).unwrap();
            apply_field_edit(&mut template, &format!("{parameter_path}/type"), "0").unwrap();
        }
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap animation flags",
            "6",
        )
        .unwrap();

        let labels = h2_template_row_labels_for_test(&shader, &template);

        for expected in [
            "noise_map1",
            "noise_map1_scale_x",
            "noise_map1_scale_y",
            "noise_map1_translation_x",
            "noise_map1_translation_y",
            "plasma_mask",
        ] {
            assert!(
                labels.iter().any(|label| label == expected),
                "missing {expected} in {labels:?}"
            );
        }
        assert!(!labels.iter().any(|label| label == "self_illum_color"));
    }

    #[test]
    fn h2ek_shader_3d_bitmap_template_rows_include_z_transform() {
        let shader = h2_classic_shader_tag();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/name", "noyze0").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "0").unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap type",
            "3D",
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap animation flags",
            "5",
        )
        .unwrap();

        let labels = h2_template_row_labels_for_test(&shader, &template);

        for expected in [
            "noyze0",
            "noyze0_scale",
            "noyze0_translation_x",
            "noyze0_translation_y",
            "noyze0_translation_z",
        ] {
            assert!(
                labels.iter().any(|label| label == expected),
                "missing {expected} in {labels:?}"
            );
        }
    }

    #[test]
    fn h2ek_shader_missing_function_rows_are_numeric_create_fields() {
        let shader = h2_classic_shader_tag();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/name", "noyze0").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "0").unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap animation flags",
            "5",
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap scale",
            "7.5",
        )
        .unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
            Some("h2_create_function_scalar")
        );
        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
            Some("value: 7.5")
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_translation_x"),
            Some("h2_create_function_scalar")
        );
        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze0_translation_x")
                .as_deref(),
            Some("value: 0.0")
        );
    }

    #[test]
    fn h2ek_shader_existing_constant_function_rows_stay_numeric() {
        let mut shader = h2_classic_shader_tag();
        apply_one_h2_shader_param_op(
            &mut shader,
            &H2ShaderParamOp::EnsureAnimationProperty {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: "noyze0".to_owned(),
                parameter_type_index: 0,
                animation_type_index: 0,
                initial_function_data: decode_hex(&constant_function_hex(7.5)).unwrap(),
            },
        )
        .unwrap();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/name", "noyze0").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "0").unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap animation flags",
            "1",
        )
        .unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
            Some("h2_function_scalar")
        );
        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
            Some("value: 7.5")
        );
    }

    #[test]
    fn h2ek_shader_color_tint_rows_use_color_animation_type() {
        let shader = h2_classic_shader_tag();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/name",
            "color_wide",
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "2").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/flags", "1").unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/default const color",
            "1, 1, 1",
        )
        .unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "color_wide_tint"),
            Some("h2_create_function_color")
        );
        assert_eq!(
            h2_template_row_value_color_for_test(&shader, &template, "color_wide_tint"),
            Some((255, 255, 255, 255))
        );
    }

    #[test]
    fn h2ek_shader_existing_constant_color_functions_render_swatch() {
        let mut shader = h2_classic_shader_tag();
        apply_one_h2_shader_param_op(
            &mut shader,
            &H2ShaderParamOp::EnsureAnimationProperty {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: "color_sharp".to_owned(),
                parameter_type_index: 2,
                animation_type_index: 12,
                initial_function_data: decode_hex(&constant_color_function_hex(1.0, 0.0, 0.0, 1.0))
                    .unwrap(),
            },
        )
        .unwrap();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/name",
            "color_sharp",
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "2").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/flags", "1").unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "color_sharp_tint"),
            Some("h2_function_color")
        );
        assert_eq!(
            h2_template_row_value_color_for_test(&shader, &template, "color_sharp_tint"),
            Some((255, 0, 0, 255))
        );
        assert_h2_write_atomic_verifies(&shader, "h2_color_function_existing");
    }

    #[test]
    fn h2ek_shader_postprocess_constants_initialize_template_rows() {
        let mut shader = h2_classic_shader_tag();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/value properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/value properties[0]/value",
            "7.5",
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/color properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/color properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/color properties[1]/color",
            "1, 0, 0",
        )
        .unwrap();

        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        for (index, (name, ty, flags)) in [("noyze0", "0", "1"), ("color_sharp", "2", "0")]
            .into_iter()
            .enumerate()
        {
            apply_one_block_op(
                &mut template,
                &BlockOp {
                    path: "categories[0]/parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            let path = format!("categories[0]/parameters[{index}]");
            apply_field_edit(&mut template, &format!("{path}/name"), name).unwrap();
            apply_field_edit(&mut template, &format!("{path}/type"), ty).unwrap();
            apply_field_edit(&mut template, &format!("{path}/flags"), flags).unwrap();
            if name == "noyze0" {
                apply_field_edit(
                    &mut template,
                    &format!("{path}/bitmap animation flags"),
                    "1",
                )
                .unwrap();
            }
        }

        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
            Some("value: 7.5")
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
            Some("scalar")
        );
        assert_eq!(
            h2_template_row_value_color_for_test(&shader, &template, "color_sharp"),
            Some((255, 0, 0, 255))
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "color_sharp"),
            Some("color")
        );
    }

    #[test]
    fn h2ek_shader_legacy_animation_bytes_initialize_template_rows() {
        let mut shader = h2_classic_shader_tag();
        for (index, (name, ty, anim_ty, data)) in [
            ("noyze0", "0", "0", {
                let mut data = vec![0; 28];
                data[0] = 1;
                data[4..8].copy_from_slice(&7.5f32.to_le_bytes());
                data[8..12].copy_from_slice(&1.0f32.to_le_bytes());
                data
            }),
            ("color_sharp", "2", "12", {
                let mut data = vec![0; 28];
                data[0] = 1;
                data[1] = 0x20;
                data[4] = 0;
                data[5] = 0;
                data[6] = 255;
                data[7] = 255;
                data
            }),
        ]
        .into_iter()
        .enumerate()
        {
            apply_one_block_op(
                &mut shader,
                &BlockOp {
                    path: "parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            let path = format!("parameters[{index}]");
            apply_field_edit(&mut shader, &format!("{path}/name"), name).unwrap();
            apply_field_edit(&mut shader, &format!("{path}/type"), ty).unwrap();
            apply_one_block_op(
                &mut shader,
                &BlockOp {
                    path: format!("{path}/animation properties"),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            apply_field_edit(
                &mut shader,
                &format!("{path}/animation properties[0]/type"),
                anim_ty,
            )
            .unwrap();
            seed_halo2_raw_function_byte_block_for_test(
                &mut shader,
                &format!("{path}/animation properties[0]/function/data"),
                &data,
            );
        }

        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        for (index, (name, ty, flags, bitmap_flags)) in
            [("noyze0", "0", "1", "1"), ("color_sharp", "2", "1", "0")]
                .into_iter()
                .enumerate()
        {
            apply_one_block_op(
                &mut template,
                &BlockOp {
                    path: "categories[0]/parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            let path = format!("categories[0]/parameters[{index}]");
            apply_field_edit(&mut template, &format!("{path}/name"), name).unwrap();
            apply_field_edit(&mut template, &format!("{path}/type"), ty).unwrap();
            apply_field_edit(&mut template, &format!("{path}/flags"), flags).unwrap();
            apply_field_edit(
                &mut template,
                &format!("{path}/bitmap animation flags"),
                bitmap_flags,
            )
            .unwrap();
        }

        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
            Some("value: 7.5")
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
            Some("h2_function_scalar")
        );
        assert_eq!(
            h2_template_row_value_color_for_test(&shader, &template, "color_sharp"),
            Some((255, 0, 0, 255))
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "color_sharp"),
            Some("h2_function_color")
        );

        let scale_edit = h2_constant_scalar_function_data(5.0, Some(&[1, 0, 0, 0, 0, 0, 240, 64]));
        assert_eq!(scale_edit.len(), 8);
        assert_eq!(
            f32::from_le_bytes(scale_edit[4..8].try_into().unwrap()),
            5.0
        );
        let color_edit = h2_constant_color_function_data(
            0.0,
            1.0,
            0.0,
            1.0,
            Some(&[1, 0x20, 0, 0, 0, 0, 255, 255]),
        );
        assert_eq!(color_edit, vec![1, 0x20, 0, 0, 0, 255, 0, 255]);
    }

    #[test]
    fn h2ek_shader_postprocess_color_overlay_initializes_tint_swatch() {
        let mut shader = h2_classic_shader_tag();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/overlays".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/overlay references".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/overlay references[0]/overlay index",
            "0",
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/overlay references[0]/transform index",
            "0",
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/animated parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/animated parameters[0]/overlay references/block index data",
            "0",
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/animated parameter references".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/animated parameter references[0]/parameter index",
            "0",
        )
        .unwrap();
        seed_halo2_wrapped_function_byte_block_for_test(
            &mut shader,
            "postprocess definition[0]/overlays[0]/function",
            &decode_hex(&constant_color_function_hex(1.0, 1.0, 0.0, 1.0)).unwrap(),
        );

        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/name",
            "center_line",
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "2").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/flags", "1").unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "center_line_tint"),
            Some("h2_function_color")
        );
        assert_eq!(
            h2_template_row_value_color_for_test(&shader, &template, "center_line_tint"),
            Some((255, 255, 0, 255))
        );

        apply_one_h2_shader_param_op(
            &mut shader,
            &H2ShaderParamOp::EditFunctionData {
                block_path: "postprocess definition[0]/overlays[0]/function/function/data"
                    .to_owned(),
                data: decode_hex(&constant_color_function_hex(0.0, 0.0, 0.0, 1.0)).unwrap(),
            },
        )
        .unwrap();
        let overlay = shader
            .root()
            .descend("postprocess definition[0]/overlays[0]/function")
            .unwrap();
        let function_struct = overlay
            .fields()
            .find(|field| field.name() == "function" && field.field_type() == TagFieldType::Struct)
            .and_then(|field| field.as_struct())
            .unwrap();
        let bytes = halo2_function_bytes_from_struct(function_struct).unwrap();
        let function = TagFunction::parse(&bytes).unwrap();
        assert_eq!(
            extract_constant_color(&function),
            Some([0.0, 0.0, 0.0, 1.0])
        );
    }

    #[test]
    fn h2_shader_color_function_create_and_edit_reparse() {
        let mut tag = h2_classic_shader_tag();
        let red = decode_hex(&constant_color_function_hex(1.0, 0.0, 0.0, 1.0)).unwrap();
        apply_one_h2_shader_param_op(
            &mut tag,
            &H2ShaderParamOp::EnsureAnimationProperty {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: "center_line".to_owned(),
                parameter_type_index: 2,
                animation_type_index: 12,
                initial_function_data: red,
            },
        )
        .unwrap();

        let parameters = tag
            .root()
            .field("parameters")
            .and_then(|field| field.as_block())
            .unwrap();
        let animation = parameters
            .element(0)
            .unwrap()
            .field("animation properties")
            .and_then(|field| field.as_block())
            .and_then(|block| block.element(0))
            .unwrap();
        assert_eq!(animation.read_int_any("type"), Some(12));
        let data_path = "parameters[0]/animation properties[0]/function/data";
        let grey = decode_hex(&constant_color_function_hex(0.5, 0.5, 0.5, 1.0)).unwrap();
        apply_one_h2_shader_param_op(
            &mut tag,
            &H2ShaderParamOp::EditFunctionData {
                block_path: data_path.to_owned(),
                data: grey.clone(),
            },
        )
        .unwrap();

        let mapping = tag
            .root()
            .descend("parameters[0]/animation properties[0]/function")
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), grey);
        let function =
            TagFunction::parse(&halo2_function_bytes_from_struct(mapping).unwrap()).unwrap();
        let color = extract_constant_color(&function).unwrap();
        for (actual, expected) in
            color
                .iter()
                .zip([128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0])
        {
            assert!((actual - expected).abs() < 0.0001);
        }
        assert_h2_write_atomic_verifies(&tag, "h2_color_function_edit");
    }

    #[test]
    fn h2ek_shader_missing_template_value_row_is_create_editable() {
        let shader = h2_classic_shader_tag();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/name",
            "plasma_factor",
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "1").unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/default const value",
            "0.35",
        )
        .unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "plasma_factor"),
            Some("h2_create_template_value")
        );
    }

    #[test]
    fn h2_shader_template_value_edit_creates_single_parameter() {
        let mut tag = h2_classic_shader_tag();
        apply_one_h2_shader_param_op(
            &mut tag,
            &H2ShaderParamOp::EditTemplateBackedValue {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: "plasma_brightness".to_owned(),
                parameter_type_index: 1,
                field: "const value".to_owned(),
                input: "1.25".to_owned(),
            },
        )
        .unwrap();

        let parameters = tag
            .root()
            .field("parameters")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(parameters.len(), 1);
        let parameter = parameters.element(0).unwrap();
        assert_eq!(
            parameter.read_string_id("name").as_deref(),
            Some("plasma_brightness")
        );
        assert_eq!(parameter.read_int_any("type"), Some(1));
        assert_eq!(parameter.read_real("const value"), Some(1.25));
    }

    #[test]
    fn h2_shader_template_function_create_materializes_backing_data() {
        let mut tag = h2_classic_shader_tag();
        let bytes = decode_hex(&constant_function_hex(0.5)).unwrap();
        apply_one_h2_shader_param_op(
            &mut tag,
            &H2ShaderParamOp::EnsureAnimationProperty {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: "noise_map1".to_owned(),
                parameter_type_index: 0,
                animation_type_index: 5,
                initial_function_data: bytes.clone(),
            },
        )
        .unwrap();

        let parameters = tag
            .root()
            .field("parameters")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(parameters.len(), 1);
        let parameter = parameters.element(0).unwrap();
        assert_eq!(
            parameter.read_string_id("name").as_deref(),
            Some("noise_map1")
        );
        assert_eq!(parameter.read_int_any("type"), Some(0));
        let animation = parameter
            .field("animation properties")
            .and_then(|field| field.as_block())
            .and_then(|block| block.element(0))
            .unwrap();
        assert_eq!(animation.read_int_any("type"), Some(5));
        let mapping = animation
            .field("function")
            .and_then(|field| field.as_struct())
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), bytes);
        assert_h2_write_atomic_verifies(&tag, "h2_function_create");
    }

    #[test]
    fn h2ek_shader_function_row_exposes_byte_block_and_wrapper_paths() {
        let mut tag = h2_classic_shader_tag();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut tag, "parameters[0]/name", "animated_scalar").unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut tag, "parameters[0]/animation properties[0]/type", "8").unwrap();
        apply_field_edit(
            &mut tag,
            "parameters[0]/animation properties[0]/input name",
            "time",
        )
        .unwrap();
        apply_field_edit(
            &mut tag,
            "parameters[0]/animation properties[0]/range name",
            "random",
        )
        .unwrap();
        apply_field_edit(
            &mut tag,
            "parameters[0]/animation properties[0]/time period",
            "2.5",
        )
        .unwrap();
        let bytes = decode_hex(&constant_function_hex(0.75)).unwrap();
        seed_halo2_function_byte_block_for_test(
            &mut tag,
            "parameters[0]/animation properties[0]/function/data",
            &bytes,
        );

        let model = build_h2ek_shader_editor_model(
            &tag,
            &h2_shader_entry(u32::from_be_bytes(*b"rmsh")),
            &TagNameIndex::default(),
            None,
        )
        .unwrap();
        let summary = first_h2_function_edit_summary(&model).expect("function row");

        assert_eq!(summary.bytes, bytes);
        assert_eq!(summary.output_index, Some(8));
        assert_eq!(summary.input_name, "time");
        assert_eq!(summary.range_name, "random");
        assert_eq!(summary.time_period, 2.5);
        assert_eq!(
            summary.data_path,
            "parameters[0]/animation properties[0]/function/data"
        );
        assert_eq!(
            summary.parameter_type_path,
            "parameters[0]/animation properties[0]/type"
        );
        assert_eq!(
            summary.input_name_path,
            "parameters[0]/animation properties[0]/input name"
        );
        assert_eq!(
            summary.range_name_path,
            "parameters[0]/animation properties[0]/range name"
        );
        assert_eq!(
            summary.time_period_path,
            "parameters[0]/animation properties[0]/time period"
        );
    }

    #[test]
    fn h2ek_shader_input_name_edit_writes_without_truncated_struct_panic() {
        let mut tag = h2_classic_shader_tag();
        for _ in 0..2 {
            apply_one_block_op(
                &mut tag,
                &BlockOp {
                    path: "parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
        }
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[1]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let bytes = decode_hex(&constant_function_hex(0.75)).unwrap();
        seed_halo2_function_byte_block_for_test(
            &mut tag,
            "parameters[1]/animation properties[0]/function/data",
            &bytes,
        );

        apply_field_edit(
            &mut tag,
            "parameters[1]/animation properties[0]/input name",
            "shield_strength",
        )
        .unwrap();

        assert_h2_write_atomic_verifies(&tag, "h2_input_name_edit");
        let written = tag.write_to_bytes().expect("write edited h2 shader");
        assert!(
            written
                .windows("shield_strength".len())
                .any(|window| { window == "shield_strength".as_bytes() })
        );
    }

    #[test]
    fn halo2_function_byte_block_rejects_invalid_mapping_function_before_clear() {
        let mut tag = h2_classic_shader_tag();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let original = decode_hex(&constant_function_hex(0.25)).unwrap();
        let block_path = "parameters[0]/animation properties[0]/function/data";
        seed_halo2_function_byte_block_for_test(&mut tag, block_path, &original);

        assert!(replace_halo2_function_byte_block(&mut tag, block_path, &[1, 2, 3]).is_err());

        let mapping = tag
            .root()
            .descend("parameters[0]/animation properties[0]/function")
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), original);
    }

    #[test]
    fn halo2_function_byte_block_same_length_edit_writes_in_place() {
        let mut tag = h2_classic_shader_tag();
        for _ in 0..7 {
            apply_one_block_op(
                &mut tag,
                &BlockOp {
                    path: "parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
        }
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[6]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let block_path = "parameters[6]/animation properties[0]/function/data";
        let original = decode_hex(&constant_function_hex(0.25)).unwrap();
        seed_halo2_function_byte_block_for_test(&mut tag, block_path, &original);
        let edited = decode_hex(&constant_function_hex(0.75)).unwrap();

        replace_halo2_function_byte_block(&mut tag, block_path, &edited).unwrap();

        let mapping = tag
            .root()
            .descend("parameters[6]/animation properties[0]/function")
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), edited);
        assert_h2_write_atomic_verifies(&tag, "h2_function_same_len");
    }

    #[test]
    fn halo2_function_byte_block_existing_length_change_rebuilds_block() {
        let mut tag = h2_classic_shader_tag();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let block_path = "parameters[0]/animation properties[0]/function/data";
        seed_halo2_function_byte_block_for_test(
            &mut tag,
            block_path,
            &decode_hex(&constant_function_hex(0.25)).unwrap(),
        );
        let mut linear_key = vec![0u8; 32];
        linear_key[0] = 5;
        linear_key[4..8].copy_from_slice(&0.0f32.to_le_bytes());
        linear_key[8..12].copy_from_slice(&1.0f32.to_le_bytes());
        for &(x, y) in &[(0.0_f32, 0.0_f32), (0.25, 1.0), (0.75, 1.0), (1.0, 0.0)] {
            linear_key.extend_from_slice(&x.to_le_bytes());
            linear_key.extend_from_slice(&y.to_le_bytes());
        }
        for _ in 0..12 {
            linear_key.extend_from_slice(&0.0_f32.to_le_bytes());
        }

        replace_halo2_function_byte_block(&mut tag, block_path, &linear_key).unwrap();

        let mapping = tag
            .root()
            .descend("parameters[0]/animation properties[0]/function")
            .unwrap();
        assert_eq!(
            halo2_function_bytes_from_struct(mapping).unwrap(),
            linear_key
        );
        assert_h2_write_atomic_verifies(&tag, "h2_function_resize");
    }

    #[test]
    fn halo2_function_byte_block_empty_creation_rebuilds_block() {
        let mut tag = h2_classic_shader_tag();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let bytes = decode_hex(&constant_function_hex(0.25)).unwrap();
        replace_halo2_function_byte_block(
            &mut tag,
            "parameters[0]/animation properties[0]/function/data",
            &bytes,
        )
        .unwrap();

        let mapping = tag
            .root()
            .descend("parameters[0]/animation properties[0]/function")
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), bytes);
        assert_h2_write_atomic_verifies(&tag, "h2_function_empty_create");
    }

    fn h2_classic_shader_tag() -> TagFile {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
        let mut header = vec![0; 64];
        header[36..40].copy_from_slice(b"hsmr");
        header[56..58].copy_from_slice(&0u16.to_le_bytes());
        header[60..64].copy_from_slice(b"!MLB");
        tag.container = blam_tags::file::TagContainer::Classic {
            engine: blam_tags::classic::ClassicEngine::Halo2V4,
            header,
        };
        tag
    }

    fn assert_h2_write_atomic_verifies(tag: &TagFile, name: &str) {
        let mut path = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!(
            "baboon_{name}_{}_{}.shader",
            std::process::id(),
            stamp
        ));
        let _ = fs::remove_file(&path);
        tag.write_atomic(&path).unwrap_or_else(|error| {
            panic!(
                "write_atomic verification failed for {}: {error}",
                path.display()
            )
        });
        let _ = fs::remove_file(&path);
    }

    fn seed_halo2_function_byte_block_for_test(tag: &mut TagFile, block_path: &str, data: &[u8]) {
        TagFunction::parse(data).unwrap();
        seed_halo2_raw_function_byte_block_for_test(tag, block_path, data);
    }

    fn seed_halo2_raw_function_byte_block_for_test(
        tag: &mut TagFile,
        block_path: &str,
        data: &[u8],
    ) {
        apply_one_block_op(
            tag,
            &BlockOp {
                path: block_path.to_owned(),
                kind: BlockOpKind::DeleteAll,
            },
        )
        .unwrap();
        for (index, byte) in data.iter().copied().enumerate() {
            apply_one_block_op(
                tag,
                &BlockOp {
                    path: block_path.to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            apply_field_edit(
                tag,
                &format!("{block_path}[{index}]/Value"),
                &(byte as i8).to_string(),
            )
            .unwrap();
        }
    }

    fn seed_halo2_wrapped_function_byte_block_for_test(
        tag: &mut TagFile,
        wrapper_path: &str,
        data: &[u8],
    ) {
        TagFunction::parse(data).unwrap();
        let mut root = tag.root_mut();
        let mut wrapper_field = root.field_path_mut(wrapper_path).unwrap();
        let mut wrapper = wrapper_field.as_struct_mut().unwrap();
        let mut wrote = false;
        wrapper.for_each_field_mut(|mut field| {
            if wrote
                || field.as_ref().name() != "function"
                || field.as_ref().field_type() != TagFieldType::Struct
            {
                return;
            }
            let Some(mut mapping) = field.as_struct_mut() else {
                return;
            };
            let Some(mut data_field) = mapping.field_mut("data") else {
                return;
            };
            let Some(mut block) = data_field.as_block_mut() else {
                return;
            };
            block.clear();
            for byte in data.iter().copied() {
                let index = block.add_element();
                let mut element = block.element_mut(index).unwrap();
                element
                    .field_mut("Value")
                    .unwrap()
                    .set(TagFieldData::CharInteger(byte as i8))
                    .unwrap();
            }
            wrote = true;
        });
        assert!(wrote, "failed to seed wrapped H2 function bytes");
    }

    fn h2_shader_entry(group_tag: u32) -> TagEntry {
        TagEntry {
            key: "objects/test/example.shader".into(),
            display_path: "objects/test/example.shader".into(),
            group_tag,
            group_name: Some("shader".into()),
            location: TagEntryLocation::LooseFile(PathBuf::from("example.shader")),
        }
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
