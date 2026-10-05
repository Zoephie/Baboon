//! editing application state.
//! It owns state types only; drawing them and acting on them belong to the feature's other modules.

use super::*;

pub(in crate::app) enum DeferredFileAction {
    SaveCurrentTag,
    /// Write this workspace's Baboon project to the `.baboon` it is associated
    /// with, asking for one when it has none. Deferred like the other saves, so
    /// an edit still focused in the editor is committed before the capture.
    SaveProject,
    SaveProjectAs,
    ExportMod,
    /// Write every tag the mounted containers ship to a folder. Deferred like
    /// the other actions that open a native dialog straight from the menu.
    ExtractAllContainerTags,
    PokeCurrentTag,
    Close(PendingCloseAction),
    /// Close whatever the active workspace currently has selected — a tag tab,
    /// or a Chimp package when that surface is up. Which one it is cannot be
    /// decided at `Ctrl+W` time without duplicating the surface check that
    /// `SaveCurrentTag` already defers, so this resolves alongside it.
    CloseCurrentTab,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct EditDraft {
    pub(in crate::app) text: String,
    baseline: String,
    pub(in crate::app) changed: bool,
    /// The pass the draft's row was last drawn on.
    drawn_pass: u64,
    /// How to commit it without its row, set while it holds a change.
    commit: Option<DraftCommit>,
}

/// How a changed draft becomes edits when it has to be committed without its
/// row: a save, a close, or a pass that no longer draws it (a collapsed
/// section, another sub-tab, another block element).
///
/// A row commits through its text box losing focus, and that is only seen by
/// drawing the box. Saving and closing run before the next draw, and a box
/// that stops being drawn never sees its loss, so each lost the edit. The row
/// leaves this behind instead, built from the same code its own commit runs.
#[derive(Clone)]
pub(in crate::app) struct DraftCommit {
    tag_key: String,
    /// Every draft the commit reads, the row's own included; committing one
    /// commits them all.
    members: Rc<[String]>,
    build: Rc<DraftCommitBuild>,
}

/// Builds a [`DraftCommit`]'s edits from the drafts as they stand.
type DraftCommitBuild = dyn Fn(&EditDrafts) -> Result<DeferredOps, String>;

impl std::fmt::Debug for DraftCommit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DraftCommit")
            .field("tag_key", &self.tag_key)
            .field("members", &self.members)
            .finish_non_exhaustive()
    }
}

impl DraftCommit {
    /// A commit that reads the drafts `members`, in order.
    pub(in crate::app) fn new(
        tag_key: &str,
        members: Vec<String>,
        build: impl Fn(&[&str]) -> Result<DeferredOps, String> + 'static,
    ) -> Self {
        let members: Rc<[String]> = members.into();
        let read = members.clone();
        Self {
            tag_key: tag_key.to_owned(),
            members,
            build: Rc::new(move |drafts: &EditDrafts| {
                let texts = read
                    .iter()
                    .map(|key| drafts.entries.get(key).map_or("", |draft| draft.text.as_str()))
                    .collect::<Vec<_>>();
                build(&texts)
            }),
        }
    }
}

impl EditDraft {
    fn new(value: impl Into<String>) -> Self {
        let text = value.into();
        Self {
            baseline: text.clone(),
            text,
            changed: false,
            drawn_pass: 0,
            commit: None,
        }
    }

    fn synchronize(&mut self, value: &str) {
        if self.changed {
            if self.text.trim() == value.trim() {
                self.text = value.to_owned();
                self.baseline = value.to_owned();
                self.changed = false;
                self.commit = None;
            }
        } else if self.baseline != value {
            self.text = value.to_owned();
            self.baseline = value.to_owned();
        }
    }

    pub(in crate::app) fn note_response(&mut self, response: &egui::Response) {
        self.changed |= response.changed();
    }

    /// Whether the draft's change commits now: its box lost focus, or Enter
    /// was pressed in it. A committed draft holds no change any more, so the
    /// box shows the tag's value from then on, as the tag displays it (`1.10`
    /// becomes `1.1`), and an undo shows through it.
    ///
    /// Escape gives up focus too, but throws the change away: the box goes
    /// back to the tag's value and nothing commits.
    pub(in crate::app) fn should_commit(&mut self, ui: &egui::Ui, response: &egui::Response) -> bool {
        if !self.changed {
            return false;
        }
        let lost_focus = lost_focus_once(response);
        if lost_focus && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.text = self.baseline.clone();
            self.mark_committed();
            return false;
        }
        let commit = lost_focus
            || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if commit {
            self.mark_committed();
        }
        commit
    }

    /// Record that the draft's change has gone to the tag (or was thrown
    /// away), so nothing commits it again.
    pub(in crate::app) fn mark_committed(&mut self) {
        self.changed = false;
        self.commit = None;
    }

    /// Leave behind how to commit this draft's change without its row. Only
    /// a draft holding a change keeps one.
    pub(in crate::app) fn keep_commit(&mut self, commit: impl FnOnce() -> DraftCommit) {
        if self.changed {
            self.commit = Some(commit());
        }
    }

    pub(in crate::app) fn set_clean(&mut self, value: impl Into<String>) {
        let value = value.into();
        self.text = value.clone();
        self.baseline = value;
        self.mark_committed();
    }
}

/// Which changed drafts [`EditDrafts::take_uncommitted`] commits.
#[derive(Clone, Copy)]
pub(in crate::app) enum DraftFlush {
    /// Every one: a save or a close is about to read the tags.
    All,
    /// Those whose row was not drawn on this pass, so it will not see its
    /// box lose focus.
    NotDrawnOn(u64),
}

#[derive(Default)]
pub(in crate::app) struct EditDrafts {
    entries: HashMap<String, EditDraft>,
    /// The pass rows are being drawn on, stamped on each draft they touch.
    pass: u64,
}

impl EditDrafts {
    /// Start stamping drafts with `pass`, egui's pass number. Called before
    /// any row draws.
    pub(in crate::app) fn begin_pass(&mut self, pass: u64) {
        self.pass = pass;
    }

    pub(in crate::app) fn draft_mut(&mut self, key: &str, value: &str) -> &mut EditDraft {
        let pass = self.pass;
        // Looked up before inserting, so a row drawn every frame doesn't
        // allocate its key every frame.
        if !self.entries.contains_key(key) {
            self.entries.insert(key.to_owned(), EditDraft::new(value));
        }
        let draft = self.entries.get_mut(key).expect("inserted above");
        draft.synchronize(value);
        draft.drawn_pass = pass;
        draft
    }

    pub(in crate::app) fn take(&mut self, key: &str, value: &str) -> EditDraft {
        let mut draft = self
            .entries
            .remove(key)
            .unwrap_or_else(|| EditDraft::new(value));
        draft.synchronize(value);
        draft.drawn_pass = self.pass;
        draft
    }

    pub(in crate::app) fn put(&mut self, key: String, draft: EditDraft) {
        self.entries.insert(key, draft);
    }

    pub(in crate::app) fn insert_clean(&mut self, key: String, value: String) {
        self.entries.insert(key, EditDraft::new(value));
    }

    /// The edits of every changed draft `which` names, by tag, as their rows
    /// would have committed them; each is then marked committed. A draft
    /// whose row left no way to commit it is left alone.
    pub(in crate::app) fn take_uncommitted(
        &mut self,
        which: DraftFlush,
    ) -> Vec<(String, Result<DeferredOps, String>)> {
        let mut commits: Vec<DraftCommit> = Vec::new();
        for draft in self.entries.values() {
            let Some(commit) = &draft.commit else {
                continue;
            };
            let due = draft.changed
                && match which {
                    DraftFlush::All => true,
                    DraftFlush::NotDrawnOn(pass) => draft.drawn_pass != pass,
                };
            if due && !commits.iter().any(|seen| Rc::ptr_eq(&seen.members, &commit.members)) {
                commits.push(commit.clone());
            }
        }
        let mut out = Vec::with_capacity(commits.len());
        for commit in commits {
            out.push((commit.tag_key.clone(), (commit.build)(self)));
            for key in commit.members.iter() {
                if let Some(draft) = self.entries.get_mut(key) {
                    draft.mark_committed();
                }
            }
        }
        out
    }

    /// Drop every draft belonging to one tag. Drafts are keyed `tag|path`, so
    /// discarding a tag's edits has to take its half-typed values with it or
    /// they would be re-applied over the reloaded document.
    pub(in crate::app) fn forget_tag(&mut self, tag_key: &str) {
        let prefix = format!("{tag_key}|");
        self.entries.retain(|key, _| !key.starts_with(&prefix));
    }

    pub(in crate::app) fn accept_successful_edits(
        &mut self,
        tag_key: &str,
        outcomes: &[FieldEditOutcome],
    ) {
        for outcome in outcomes.iter().filter(|outcome| outcome.result.is_ok()) {
            let key = format!("{tag_key}|{}", outcome.path);
            let Some(draft) = self.entries.get_mut(&key) else {
                continue;
            };
            if draft.text.trim() == outcome.input.trim() {
                draft.set_clean(outcome.input.clone());
            }
        }
    }

    pub(in crate::app) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(in crate::app) fn retain(&mut self, mut keep: impl FnMut(&String, &mut EditDraft) -> bool) {
        self.entries.retain(|key, value| keep(key, value));
    }
}

/// A copied block element, held on the app so it can be pasted into a block of
/// the same shape in another open tag. `group_tag` + `block_path` gate which
/// blocks accept the paste (same group, same block); the library re-validates
/// element compatibility before inserting.
#[derive(Clone)]
pub(in crate::app) struct BlockClipboard {
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) block_path: String,
    /// Human label for the menu, e.g. "initial permutation".
    pub(in crate::app) label: String,
    /// On-disk byte size of one copied element — the engine's version-
    /// authoritative paste key (`raw_data / count`; e.g. H2 `bitmap_data`
    /// v1 = 116 vs latest = 140). `Some` for block copies; `None` for inline
    /// arrays, which aren't FieldSet-versioned and expose no size accessor.
    /// Gates paste so a different struct version can't be pasted (and corrupt
    /// the tag) until upgrade/downgrade exists.
    pub(in crate::app) element_size: Option<usize>,
    /// One element (Copy element) or every element (Copy entire block).
    pub(in crate::app) elements: Vec<blam_tags::TagBlockElement>,
}

/// A pending destructive block op awaiting user confirmation. Lives on the
/// app (persists across frames) and is shown as a modal.
pub(in crate::app) struct BlockConfirm {
    /// Workspace whose pane raised this, stamped by that pane after the
    /// field render that created it — the field renderers are shared by
    /// every pane and have no kit of their own. `None` only inside that one
    /// render; the apply drops a confirm that somehow never got stamped.
    pub(in crate::app) kit: Option<KitId>,
    pub(in crate::app) tag_key: String,
    pub(in crate::app) path: String,
    pub(in crate::app) kind: BlockOpKind,
    pub(in crate::app) message: String,
    /// Label for the confirm button (e.g. "Delete", "Replace").
    pub(in crate::app) confirm_label: String,
}

/// A request to open a referenced tag in a new tab (from an "Open" button on
/// a tag-reference row). Resolved against the loose-folder tags root.
#[derive(Clone)]
pub(in crate::app) struct OpenTagRequest {
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) rel_path: String,
    /// When true, open the tag in a floating (torn-off) window instead of the
    /// docked tab rack. Set by Alt-clicking a reference's Open button.
    pub(in crate::app) float: bool,
}

/// A request to play or extract a `.sound` a container-source tag only *refers*
/// to (`sound_looping` tracks, dialogue vocalizations).
///
/// On a loose folder the player loads the referenced tag and reads its samples
/// on the spot. A Campaign Evolved tag has no samples to read: the audio is
/// reached by walking the referenced tag's own package imports out to Wwise,
/// which is the app's job, not the field renderer's. So the click is recorded
/// here and resolved after the frame.
#[derive(Clone)]
pub(in crate::app) struct CeSoundRefRequest {
    pub(in crate::app) group_tag: u32,
    /// The reference as stored in the tag (Halo-relative, `\`-separated).
    pub(in crate::app) reference: String,
    /// Label for the player's status line.
    pub(in crate::app) label: String,
    /// Extract every permutation to a chosen folder instead of playing one.
    pub(in crate::app) extract: bool,
    /// The player clip a play is for, carried to the play it becomes.
    pub(in crate::app) clip: Option<String>,
    /// Decode it for the clip's waveform rather than play it.
    pub(in crate::app) preview: bool,
}

/// Read-only tag catalog exposed to reference pickers for sources whose tags do
/// not exist as ordinary files. The group tree's indices address `entries`.
#[derive(Clone, Copy)]
pub(in crate::app) struct TagReferenceCatalog<'a> {
    pub(in crate::app) entries: &'a [TagEntry],
    pub(in crate::app) group_tree: &'a TagTree,
    pub(in crate::app) expert_mode: bool,
}

/// Cross-frame state for the Campaign Evolved tag-reference picker window.
pub(in crate::app) struct TagReferencePickerState {
    pub(in crate::app) tag_key: String,
    pub(in crate::app) field_path: String,
    pub(in crate::app) allowed_groups: Vec<u32>,
    pub(in crate::app) search: String,
}

/// A request to (re)import a geometry tag via `tool` (from the Import button on
/// a render/collision/physics-model or animation-graph reference).
#[derive(Clone)]
pub(in crate::app) struct ToolImportRequest {
    /// `tool` verb: "render" / "collision" / "physics" /
    /// "model-animations-uncompressed".
    pub(in crate::app) verb: &'static str,
    /// Source directory argument, e.g. `objects\characters\masterchief`.
    pub(in crate::app) source_dir: String,
}

/// What the user clicked in a block header this frame.
#[derive(Default)]
pub(in crate::app) struct BlockHeaderActions {
    pub(in crate::app) add: bool,
    pub(in crate::app) insert: bool,
    pub(in crate::app) duplicate: bool,
    pub(in crate::app) delete: bool,
    pub(in crate::app) delete_all: bool,
    pub(in crate::app) new_selection: Option<usize>,
    pub(in crate::app) reorganize: bool,
    /// Right-click → "Copy element" on the selected element.
    pub(in crate::app) copy: bool,
    /// Right-click → "Copy entire block".
    pub(in crate::app) copy_block: bool,
    /// Right-click → "Copy block as TSV" (plaintext, Excel-friendly).
    pub(in crate::app) copy_block_tsv: bool,
    /// Right-click → "Paste TSV…" (open the import window for this block).
    pub(in crate::app) paste_tsv: bool,
    /// Right-click → "Paste" (insert clipboard element(s) after the selection).
    pub(in crate::app) paste: bool,
    /// Right-click → "Replace selected element" with the clipboard.
    pub(in crate::app) replace_element: bool,
    /// Right-click → "Replace entire block" with the clipboard.
    pub(in crate::app) replace_block: bool,
}

/// Emitted by a block header when the user picks "Paste TSV…" — the app hoists
/// it into `tsv_paste` and opens the import window.
pub(in crate::app) struct TsvPasteRequest {
    pub(in crate::app) block_path: String,
    pub(in crate::app) block_label: String,
    pub(in crate::app) element_count: usize,
}

pub(in crate::app) struct BlockTableRequest {
    pub(in crate::app) path: String,
    pub(in crate::app) label: String,
    pub(in crate::app) view_scope: String,
    pub(in crate::app) selected: usize,
}

#[derive(Clone)]
pub(in crate::app) struct BlockTableRow {
    pub(in crate::app) id: u64,
    pub(in crate::app) original_index: Option<usize>,
    pub(in crate::app) name_field: Option<String>,
    pub(in crate::app) name: String,
    pub(in crate::app) stored_name: String,
}

/// A private tag copy owns all staged data and reference changes. The live
/// document is untouched until the window's Confirm Changes action succeeds.
pub(in crate::app) struct BlockTableState {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) tag_key: String,
    pub(in crate::app) request: BlockTableRequest,
    pub(in crate::app) stamp: (u64, u64),
    pub(in crate::app) game: Option<GameId>,
    pub(in crate::app) definitions_root: Option<PathBuf>,
    pub(in crate::app) tag: TagFile,
    pub(in crate::app) baseline_bytes: Vec<u8>,
    pub(in crate::app) rows: Vec<BlockTableRow>,
    pub(in crate::app) next_id: u64,
    pub(in crate::app) status: Option<String>,
    pub(in crate::app) changed: bool,
}

/// The open TSV-import window: the user pastes tab-separated rows and applies
/// them to the target block's existing elements (per-cell, via `apply_field_edit`).
pub(in crate::app) struct TsvPasteState {
    /// Workspace this was raised from. The confirm applies against the active
    /// kit, and a modeless dialog outlives the frame that opened it, so the
    /// user can focus another game in between; resolving this first is what
    /// keeps the action on the workspace it was started in.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) tag_key: String,
    pub(in crate::app) block_path: String,
    pub(in crate::app) block_label: String,
    pub(in crate::app) element_count: usize,
    pub(in crate::app) text: String,
    pub(in crate::app) status: Option<String>,
}

/// Per-frame capability bundle passed through schema-driven field rendering.
///
/// Renderers append deferred operations instead of mutating the borrowed tag.
/// Optional services deliberately make secondary/read-only views usable without
/// inventing source roots, definitions, or writable status.
pub(in crate::app) struct FieldEditContext<'a> {
    pub(in crate::app) view_scope: &'a str,
    pub(in crate::app) tag_key: &'a str,
    /// Group tag of the tag being rendered — gates block paste compatibility.
    pub(in crate::app) group_tag: u32,
    /// Root struct of the tag being rendered — used to resolve block-index
    /// fields whose target block is an ancestor (not a sibling). `None` in
    /// read-only/secondary contexts where ancestor resolution isn't needed.
    pub(in crate::app) root: Option<blam_tags::TagStruct<'a>>,
    pub(in crate::app) game: Option<GameId>,
    pub(in crate::app) definitions_root: Option<&'a Path>,
    pub(in crate::app) names: Option<&'a TagNameIndex>,
    pub(in crate::app) tags_root: Option<&'a Path>,
    /// The loaded kit's root, tags and data folders, for anything that writes
    /// into the kit's data folder (sound extraction). `None` outside a loose
    /// editing kit.
    pub(in crate::app) kit_layout: Option<&'a KitLayout>,
    /// Entries available to source-aware bitmap hover previews. Loose sources
    /// can also synthesize an entry from `tags_root` when their lazy browser
    /// tree has not visited the referenced folder yet.
    pub(in crate::app) bitmap_hover_entries: Option<&'a [TagEntry]>,
    /// Populated only for Campaign Evolved container sources. Loose editing
    /// kits continue to use `tags_root` and the native file picker.
    pub(in crate::app) tag_reference_catalog: Option<TagReferenceCatalog<'a>>,
    /// Shared state for the movable Campaign Evolved reference-picker window.
    pub(in crate::app) tag_reference_picker: &'a mut Option<TagReferencePickerState>,
    pub(in crate::app) status: Option<&'a mut String>,
    pub(in crate::app) editable: bool,
    pub(in crate::app) show_block_sizes: bool,
    pub(in crate::app) buffers: &'a mut EditDrafts,
    pub(in crate::app) pending: &'a mut Vec<PendingFieldEdit>,
    pub(in crate::app) block_ops: &'a mut Vec<BlockOp>,
    pub(in crate::app) block_confirm: &'a mut Option<BlockConfirm>,
    /// Set when the user clicks "Open" on a tag-reference row.
    pub(in crate::app) open_request: &'a mut Option<OpenTagRequest>,
    /// Set when the user clicks a Play/Stop control in the sound-player panel;
    /// the app drains it after rendering to drive FMOD bank playback.
    /// Each action is stamped with this pane's tab.
    pub(in crate::app) sound_play_request: super::audio::SoundRequests<'a>,
    /// Last sound-player status line (bank/resolve/playback result), for display.
    pub(in crate::app) sound_status: Option<&'a str>,
    /// Current playback volume (linear, 0.0..=1.0), for the sound-player slider.
    pub(in crate::app) sound_volume: f32,
    /// Playback speed (a multiple of the recorded rate), for its slider.
    pub(in crate::app) sound_speed: f32,
    /// The sound this pane's tab has loaded (playing, paused or finished), for
    /// the transport; `None` while the loaded sound, if any, is another tab's.
    pub(in crate::app) sound_playback: Option<super::audio::PlaybackView>,
    /// Whether sounds loop, for the transport's loop toggle.
    pub(in crate::app) sound_looping: bool,
    /// This tab's preview of its selected clip, for the waveform before it
    /// plays.
    pub(in crate::app) sound_preview: Option<super::audio::Preview>,
    /// Whether this pane is the focused tab, which is where the player's
    /// keyboard shortcuts act.
    pub(in crate::app) sound_has_focus: bool,
    /// Set when the user extracts sound audio to disk (per-perm or whole-tag);
    /// the app drains it to decode + write the files.
    pub(in crate::app) sound_extract_request:
        &'a mut Option<crate::app::export::sound_extract::ExtractRequest>,
    /// Selected localized sound language (`None` = default), for the player's
    /// language selector + `data_<lang>\` extraction routing.
    pub(in crate::app) sound_language: Option<&'a str>,
    /// Campaign Evolved only: the Wwise media this `sound` tag resolves to,
    /// already walked out through its package imports. `None` for every other
    /// game; `Some` but empty for a tag that binds to no event.
    pub(in crate::app) ce_sound: Option<&'a crate::core::source::ce_audio::CeSoundBinding>,
    /// The container source's `Paks` directory, where the legacy `.pak`
    /// containers holding Campaign Evolved's Wwise media live.
    pub(in crate::app) ce_paks_root: Option<&'a Path>,
    /// Set when a player that only holds a `.sound` *reference* (sound_looping,
    /// dialogue) is asked to play or extract one on a container source. The
    /// referenced tag carries no audio itself, so the app resolves the
    /// reference's own Wwise binding after rendering rather than here.
    pub(in crate::app) ce_sound_ref_request: &'a mut Option<CeSoundRefRequest>,
    /// Set when the user clicks "Import" on a geometry tag-reference row.
    pub(in crate::app) tool_import: &'a mut Option<ToolImportRequest>,
    /// Shader-specific deferred ops (add animated parameter + init).
    pub(in crate::app) shader_ops: &'a mut Vec<ShaderOp>,
    /// Shader-specific deferred ops (create parameter entry + set real value).
    pub(in crate::app) shader_param_ops: &'a mut Vec<ShaderParamOp>,
    /// H2EK-specific deferred ops (create classic shader parameters/animations).
    pub(in crate::app) h2_shader_param_ops: &'a mut Vec<H2ShaderParamOp>,
    /// Model-preview variant edits queued from the render model tab.
    pub(in crate::app) model_variant_ops: &'a mut Vec<ModelVariantOp>,
    /// Set when the user clicks a color swatch on a value row; the caller hoists
    /// it into `self.color_popup` after rendering so the shared popup handler
    /// can show the picker and apply the edit.
    pub(in crate::app) color_request: &'a mut Option<MaterialColorPopup>,
    /// Set when the user clicks a function row; the caller hoists it into
    /// `self.function_popup` after rendering so the shared popup handler can
    /// show the graph editor and apply function-data edits.
    pub(in crate::app) function_request: &'a mut Option<FunctionPopup>,
    /// Documentation overlay (help/units + explanation blocks) for this tag's
    /// group, parsed from the JSON definition. Used to restore field tooltips
    /// and explanation rows that shipped tags strip from their layout.
    pub(in crate::app) docs: Option<&'a DefDocs>,
    /// Set when the user picks "Paste TSV…" on a block; the caller hoists it
    /// into `self.tsv_paste` to open the import window.
    pub(in crate::app) tsv_paste_request: &'a mut Option<TsvPasteRequest>,
    pub(in crate::app) block_table_request: &'a mut Option<BlockTableRequest>,
    /// The current block clipboard (read), for gating "Paste" in block menus.
    pub(in crate::app) block_clipboard: Option<&'a BlockClipboard>,
    /// Set when the user clicks "Copy element"; the caller hoists it into
    /// `self.block_clipboard` after rendering.
    pub(in crate::app) block_clip_request: &'a mut Option<BlockClipboard>,
    /// Find's current visual filter action. While filtering, it keeps matching
    /// paths visible and their containers open; disabling the filter emits one
    /// restore-defaults pass.
    pub(in crate::app) field_filter: Option<&'a FieldFilterAction>,
    /// Active reference-jump navigation. When set for this tag, its target
    /// field's ancestor blocks are force-opened and the field is glowed.
    pub(in crate::app) field_nav: Option<&'a FieldNav>,
    /// One-shot "expand everything" / "collapse everything" for this tag,
    /// consumed by the pane that draws it. egui remembers each container's
    /// state, so a single frame of forcing is enough to make it stick.
    pub(in crate::app) expand_all: Option<bool>,
    /// How nested containers start out, from preferences.
    pub(in crate::app) nested_default: NestedDefault,
}

/// Owned storage for every request and op a [`FieldEditContext`] can raise,
/// for a context whose caller discards them: a read-only view that must
/// still hand the renderer somewhere to write.
#[derive(Default)]
pub(in crate::app) struct EditSinks {
    buffers: EditDrafts,
    pending: Vec<PendingFieldEdit>,
    block_ops: Vec<BlockOp>,
    block_confirm: Option<BlockConfirm>,
    open_request: Option<OpenTagRequest>,
    sound_play_request: std::collections::VecDeque<super::audio::SoundRequest>,
    sound_extract_request: Option<crate::app::export::sound_extract::ExtractRequest>,
    ce_sound_ref_request: Option<CeSoundRefRequest>,
    tool_import: Option<ToolImportRequest>,
    shader_ops: Vec<ShaderOp>,
    shader_param_ops: Vec<ShaderParamOp>,
    h2_shader_param_ops: Vec<H2ShaderParamOp>,
    model_variant_ops: Vec<ModelVariantOp>,
    color_request: Option<MaterialColorPopup>,
    function_request: Option<FunctionPopup>,
    tsv_paste_request: Option<TsvPasteRequest>,
    block_table_request: Option<BlockTableRequest>,
    block_clip_request: Option<BlockClipboard>,
    tag_reference_picker: Option<TagReferencePickerState>,
}

impl<'a> FieldEditContext<'a> {
    /// A context that edits nothing: `editable` off, every optional input
    /// absent, and every request written into `sinks`, which the caller drops.
    /// Callers set what their view does have (a root, names, a filter) on the
    /// result.
    pub(in crate::app) fn read_only(
        sinks: &'a mut EditSinks,
        view_scope: &'a str,
        tag_key: &'a str,
    ) -> Self {
        Self {
            view_scope,
            tag_key,
            group_tag: 0,
            root: None,
            game: None,
            definitions_root: None,
            names: None,
            tags_root: None,
            kit_layout: None,
            bitmap_hover_entries: None,
            tag_reference_catalog: None,
            tag_reference_picker: &mut sinks.tag_reference_picker,
            status: None,
            editable: false,
            show_block_sizes: false,
            buffers: &mut sinks.buffers,
            pending: &mut sinks.pending,
            block_ops: &mut sinks.block_ops,
            block_confirm: &mut sinks.block_confirm,
            open_request: &mut sinks.open_request,
            sound_play_request: super::audio::SoundRequests::new(
                &mut sinks.sound_play_request,
                None,
            ),
            sound_status: None,
            sound_volume: 1.0,
            sound_speed: 1.0,
            sound_playback: None,
            sound_looping: false,
            sound_preview: None,
            sound_has_focus: false,
            sound_extract_request: &mut sinks.sound_extract_request,
            sound_language: None,
            ce_sound: None,
            ce_paks_root: None,
            ce_sound_ref_request: &mut sinks.ce_sound_ref_request,
            tool_import: &mut sinks.tool_import,
            shader_ops: &mut sinks.shader_ops,
            shader_param_ops: &mut sinks.shader_param_ops,
            h2_shader_param_ops: &mut sinks.h2_shader_param_ops,
            model_variant_ops: &mut sinks.model_variant_ops,
            color_request: &mut sinks.color_request,
            function_request: &mut sinks.function_request,
            docs: None,
            tsv_paste_request: &mut sinks.tsv_paste_request,
            block_table_request: &mut sinks.block_table_request,
            block_clipboard: None,
            block_clip_request: &mut sinks.block_clip_request,
            field_filter: None,
            field_nav: None,
            expand_all: None,
            nested_default: NestedDefault::default(),
        }
    }
}

impl FieldEditContext<'_> {
    /// Queue `ops`, as a row's own commit does: the same ops a
    /// [`DraftCommit`] builds when the row cannot commit itself.
    pub(in crate::app) fn push_ops(&mut self, ops: DeferredOps) {
        let DeferredOps {
            pending,
            block_ops,
            shader_ops,
            shader_param_ops,
            h2_shader_param_ops,
            model_variant_ops,
            function_data_ops,
        } = ops;
        // Function-data writes come only from the function popup, never from
        // a text row.
        debug_assert!(function_data_ops.is_empty());
        self.pending.extend(pending);
        self.block_ops.extend(block_ops);
        self.shader_ops.extend(shader_ops);
        self.shader_param_ops.extend(shader_param_ops);
        self.h2_shader_param_ops.extend(h2_shader_param_ops);
        self.model_variant_ops.extend(model_variant_ops);
    }

    pub(in crate::app) fn widget_id(
        &self,
        salt: impl std::hash::Hash + std::fmt::Debug,
    ) -> egui::Id {
        egui::Id::new(("field_edit", self.view_scope, self.tag_key, salt))
    }

    /// Decide the forced open-state for a collapsible node at `node_path`,
    /// whose normal default is `default_open`. `None` means "leave the node's
    /// stored state alone" (no filter applied this frame); `Some(open)` forces
    /// it this frame.
    /// The starting open state for a container whose schema-derived default is
    /// `schema_default`, after the preference is applied.
    ///
    /// This adjusts the *default* rather than forcing the state each frame, so
    /// a container the user has since opened or closed by hand keeps whatever
    /// they chose — egui only consults a default when it has nothing stored.
    pub(in crate::app) fn default_open(&self, schema_default: bool) -> bool {
        self.nested_default.applies_to(schema_default)
    }

    pub(in crate::app) fn resolve_open(&self, node_path: &str, default_open: bool) -> Option<bool> {
        // An explicit expand/collapse-all wins over everything: it is a direct
        // instruction about this whole tag, issued this frame. Every container
        // type -- groups, structs, blocks, arrays -- resolves its open state
        // here, so this one line reaches all of them.
        if self.expand_all.is_some() {
            return self.expand_all;
        }
        // A reference-jump forces every ancestor of its target field open so the
        // field can be scrolled into view. Takes precedence over the search filter.
        if let Some(nav) = self.field_nav {
            if nav.tag_key == self.tag_key
                && path_is_ancestor(
                    &strip_node_indices(node_path),
                    &strip_node_indices(&nav.field_path),
                )
            {
                return Some(true);
            }
        }
        match self.field_filter? {
            // Query cleared: snap every node back to its normal default.
            FieldFilterAction::RestoreDefaults => Some(default_open),
            FieldFilterAction::Apply(filter) => {
                let canon = strip_node_indices(node_path);
                // Every rendered container is on a match path (others are hidden
                // by `field_visible`), so expand it to reveal the match in
                // context. The implicit root group has no path — always open.
                if canon.is_empty() || filter.visible_paths.contains(&canon) {
                    Some(true)
                } else {
                    Some(false)
                }
            }
        }
    }

    /// Whether a Find filter is applied this frame — i.e. the editor
    /// is hiding non-matches. Used to also suppress injected section/explanation
    /// rows so no orphan headers remain.
    pub(in crate::app) fn is_active_filter(&self) -> bool {
        matches!(self.field_filter, Some(FieldFilterAction::Apply(_)))
    }

    /// Whether `path`'s field should render at all. While a query is active only
    /// matches, their ancestor containers, and name-matched containers' contents
    /// are shown; everything else is hidden. Always visible with no query.
    pub(in crate::app) fn field_visible(&self, path: &str) -> bool {
        match self.field_filter {
            Some(FieldFilterAction::Apply(filter)) => {
                filter.visible_paths.contains(&strip_node_indices(path))
            }
            _ => true,
        }
    }

    /// Whether the exact `indexed_path` field is the live reference-jump target
    /// and still within its glow window — used to pulse the landed-on field.
    pub(in crate::app) fn field_nav_glow(&self, indexed_path: &str, now: f64) -> bool {
        self.field_nav.is_some_and(|nav| {
            nav.tag_key == self.tag_key && nav.field_path == indexed_path && now < nav.glow_until
        })
    }
}

/// Whether `ancestor` is `target` itself or an ancestor of it, compared
/// segment-wise so `"custom references"` is an ancestor of
/// `"custom references/sounds"` but not of `"custom references extra"`. Both
/// paths must already be index-stripped (see [`strip_node_indices`]).
fn path_is_ancestor(ancestor: &str, target: &str) -> bool {
    if ancestor.is_empty() {
        return true;
    }
    target == ancestor
        || (target.len() > ancestor.len()
            && target.as_bytes()[ancestor.len()] == b'/'
            && target.starts_with(ancestor))
}

/// What Find filtering should do to the editor's collapse state this frame.
pub(in crate::app) enum FieldFilterAction {
    /// Hide everything except matches and their ancestor containers; expand the
    /// containers that remain.
    Apply(std::sync::Arc<FieldFilter>),
    /// Re-expand every node to its normal default (query was cleared).
    RestoreDefaults,
}

/// The Find filter a pane last applied, with the inputs it was built from.
/// Rebuilding it walks every element of the tag, so it is reused until the
/// query, its options or the document change.
pub(in crate::app) struct AppliedFindFilter {
    pub(in crate::app) signature: String,
    pub(in crate::app) filter: std::sync::Arc<FieldFilter>,
}

/// Which collapsible nodes a Find query wants open. Paths are the
/// canonical field paths with element indices (`[3]`) stripped, so they're
/// independent of which block element happens to be selected.
#[derive(Clone, Default)]
pub(in crate::app) struct FieldFilter {
    /// Canonical paths of every field that should render while searching:
    /// matches, their ancestor containers, and the contents of name-matched
    /// containers. Fields absent from this set are hidden.
    pub(in crate::app) visible_paths: std::collections::HashSet<String>,
}

#[derive(Clone)]
pub(in crate::app) struct FieldDisplayMeta {
    pub(in crate::app) label: String,
    pub(in crate::app) unit: Option<String>,
    /// A `[min,max]` range/bounds hint (shown after the unit/type), e.g.
    /// `[0,+inf]`. Parsed out of the unit slot or the bare name.
    pub(in crate::app) range: Option<String>,
    pub(in crate::app) help: Option<String>,
    /// Tag groups declared by the JSON definition for tag_reference fields.
    /// The runtime blam-tags layout keeps only reference flags, so Baboon
    /// carries this through the docs overlay for display-only affordances.
    pub(in crate::app) tag_reference_allowed: Vec<u32>,
    pub(in crate::app) read_only: bool,
    pub(in crate::app) advanced: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_draft_is_not_replaced_by_stale_model_value() {
        let mut draft = EditDraft::new("10");
        draft.text = "25".to_owned();
        draft.changed = true;
        draft.synchronize("10");
        assert_eq!(draft.text, "25");
        assert!(draft.changed);
    }

    #[test]
    fn successful_commit_becomes_the_new_clean_baseline() {
        let mut draft = EditDraft::new("10");
        draft.text = "25".to_owned();
        draft.changed = true;
        draft.synchronize("25");
        assert_eq!(draft.text, "25");
        assert!(!draft.changed);
        draft.synchronize("30");
        assert_eq!(draft.text, "30");
    }

    // Foundation unit tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    /// Expand/collapse-all is a direct instruction about the whole tag, so it
    /// has to win over the rules that otherwise decide a container's open
    /// state — the search filter's, and a reference jump forcing its target's
    /// ancestors open. Every container type resolves through here, so this is
    /// the one place that ordering is decided.
    #[test]
    fn expand_all_overrides_the_other_open_rules() {
        with_test_edit_context(|edit| {
            // Nothing asked for: the caller's own default stands.
            assert_eq!(edit.resolve_open("some/block", true), None);

            edit.expand_all = Some(true);
            assert_eq!(edit.resolve_open("some/block", false), Some(true));

            edit.expand_all = Some(false);
            assert_eq!(edit.resolve_open("some/block", true), Some(false));
        });
    }

    /// The preference adjusts each container's *default* rather than forcing
    /// its state, so a group the user has since opened or closed keeps their
    /// choice — egui only consults a default when it has nothing stored.
    #[test]
    fn nested_default_overrides_only_the_schema_default() {
        with_test_edit_context(|edit| {
            edit.nested_default = NestedDefault::Schema;
            assert!(edit.default_open(true));
            assert!(!edit.default_open(false));

            edit.nested_default = NestedDefault::Collapsed;
            assert!(
                !edit.default_open(true),
                "collapsed must close a section the schema opens"
            );

            edit.nested_default = NestedDefault::Expanded;
            assert!(
                edit.default_open(false),
                "expanded must open a section the schema closes"
            );

            // And it stays a default: nothing here forces an open state.
            assert_eq!(edit.resolve_open("some/block", true), None);
        });
    }
}
