# Material Editor in the Model Preview: plan

Status: plan only, nothing built. Written 2026-11-26 against `main` at 011500c0.

Status: **built** on branch `material-editor` (local, not pushed), 2026-11-26, one commit per step. Where the build departed from this plan:

- **Material saving is not held back** (§3.9). The research answered it: all 6977 Halo 4 kit materials ship an empty `postprocess definition` (counted with `blam-tag-shell block … count`), and the game builds the postprocess from the parameters at load (blam-tags `material` module, after `c_material::postprocess`). What the grid writes is what the game reads.
- **The material view was designed without Foundation's.** `~/Source/HREK_Foundation` is no longer on disk; the view follows the tag's own semantics (the engine's bake: an override's value, then its functions over it) and the render-method grid's rows.
- **No input-bytes key** (§3.6, point 10). After OK on a picker, the preview resolves the material once more and gets the same textures; its maps come from the kept caches, so nothing is uploaded and nothing flickers. Not worth the hashing.
- **A picker shows a draft only once its colour differs from the one it opened with** (§3.7): sending the tag's own colour at open made a resolve for nothing.
- **A new read kind, `ReadAs::Live`**, notes the files of a preview's held shaders: neither a save nor an edit of one makes the preview load again.
- **Not run:** the pixel/GL-call gates (their baselines, `~/Source/baboon-gates`, are gone), and the unsaveable Save button in the window (covered by `unsaveable_reason`'s own tests). H2A and Halo 5 materials were not tried on real tags (no kits).

## 1. What we're building

A **Material Editor** button sits in the Model Preview header, beside Refresh Model. It opens a window that has:

1. **A material dropdown.** It lists the shaders the previewed model draws with. While the pointer is over a row, the viewport draws a coloured overlay on every part that uses that material.
2. **The shader's own editor.** Below the dropdown, the window shows the same UI the shader gets in its own tab: the render-method grid, the Halo 2 grid, or the field tree.
3. **Live edits, saved only on request.** Each edit shows in the preview at once. Nothing is written to disk until the user saves.
4. **A modified list.** It names every material with unsaved edits. Each row has **Save** and **Reset**, and the list has **Save All** and **Reset All**.

It covers every game whose preview resolves textures: Halo CE (MCC), Halo 2, Halo 3, ODST, Reach and Halo 4 (`load::rules(game).textures`). Halo 2 Anniversary and Campaign Evolved resolve no textures, so they get no button. Halo 4, Halo 2 Anniversary and Halo 5 `material` (`mat `) tags become **editable everywhere**, in their own tabs as well as in the window (owner's decision, 2026-11-26; §3.9). Each game's limits are in §5.

## 2. What the code already has, and what it lacks

These facts shape the plan. Each was read in the code, and the load-bearing ones were checked a second time.

**Header.** `panel/layout.rs` `draw_preview_header` lays the header out right to left: the View and Shading menus (`draw_model_view_settings_menu`), then **Refresh Model**, then the layer toggles from the left edge. "To the right of Refresh Model" therefore means drawing the new button **between the Shading menu and Refresh Model**, before Refresh in code order.

**Materials.**
- `RenderModelPreview::materials` has one entry per element of the render model's material block, index-aligned with `batch.material_index`. It is **never deduplicated**.
- Entries are appended after the model's own:
  - the LOD model's whole list;
  - held weapons and attached objects (`merge_attached`, with `object = owner`);
  - H2 hologram, shield and overshield shaders, and CE's `modifier shader`, which have **no batches of their own**;
  - gen3 screen-effect `rmss`;
  - CE flags and cloth;
  - each BSP of a scenario;
  - empty-path flat materials for collision and physics.
- The same shader can appear several times: CE permutations, the LOD model, a weapon sharing a unit's shader, BSPs.
- The render model carries **no material names**, only the shader reference. The footer already labels materials by `shader_path`.

**The shader UI.**
- `editor/material/mod.rs` `draw_material_tag` → `editor/shader/widgets.rs` `draw_shader_editor_model` draws the grid. It needs:
  - a `FieldEditContext`;
  - the kit's `rmdf_cache`, `rmop_cache` and `h2_templates`;
  - `source: Some(&TagSource)`;
  - colour and function popup sinks.
- Which UI each game's shaders get today:
  - **Halo CE `shdr` and its children get no special UI.** A test pins this (`only_grid_shaders_are_routed_to_the_shader_grid`). They draw as the generic field tree.
  - **H2 `shad`** gets the H2EK grid, but only for classic Halo2V4 tags.
  - **gen3 `rm*`** gets the render-method grid.
  - **H4/H2A `mat ` and `mats`** get the material view, which is **read-only**: `draw_material_struct_fields` is never given the edit context.
- Edits never touch the `TagFile`. They are collected as `DeferredOps` and sent each frame as `EditorCommand::PaneDrawn { kit, key, ops }`, which `apply_pane_drawn` hands to `apply_doc_ops(…, UndoStep::Coalesce)`. The empty send on frames without edits is what closes the undo window.
- `editor/pane.rs` `draw_tag_pane` is documented as "the only place a tag document is rendered". It builds the whole `FieldEditContext` (`pane.rs` ~350–460) and afterwards stamps the colour, function and reference popups with the `KitId` and the document's `layout_stamp()`.

**Documents with no tab.**
- `kit.parsed_tags` can hold a document with no tab. Edits, `save_tag_by_key`, `discard_tag_changes` and the preview's unsaved-edits overlay all work from that map.
- Four things assume a tab:
  - `handle_tag_loaded` (`documents/selection.rs`) **throws away a load whose key isn't in `open_tabs`**.
  - `close_action_tag_keys` (`documents/close.rs`) lists **`open_tabs` only** for CloseApp, CloseAllTabs and CloseKit. A dirty document with no tab is never offered to the save prompt. CloseApp then refuses ("Could not close while unsaved workspace data remains"), because `any_kit_dirty` counts it, so **the app cannot quit**. CloseAllTabs and CloseKit drop such documents silently.
  - `check_open_tags_in` (`documents/disk_changes.rs`) checks `open_tabs` only for changes on disk.
  - Undo and redo (`documents/undo.rs`) act on the active kit's `selected_key`, and `restore_snapshot` is wired to the active kit.

**Unsaved edits reaching the preview.**
- `core/source/reads.rs` `ReadFiles::with_edits` swaps serialized document bytes in for **loose-file** reads. Container and monolithic reads never consult it.
- With View > Show Unsaved Edits on, `check_previews_for_unsaved_edits_at` waits `UNSAVED_EDITS_QUIET` (0.3 s) and then **reloads the whole preview**.
- After a save, `reload_previews_reading` skips files that were read `ReadAs::Edits`. A preview that already drew the edits therefore doesn't reload when they're saved.

**Resolving and uploading.**
- `resolve_model_textures` resolves every material on a worker.
- `resolve_one_material` exists but is private.
- `finish_dx9_layers` → `share_object_layers` / `share_with_object` **`mem::take`s** these passes into the matching layer of every material with the same `object`:
  - CE's overlay material's Transparent passes, into the Hologram layer;
  - H2's hologram, shield-damaged and overshield passes.
- So materials are interdependent after the resolve.
- `ResolveCaches` and the `TagReader` memo live for one resolve only.
- `ModelPreviewData::set_textures` gives a new `textures_id`. `ModelGpuSlot::sync_textures` and `Gen3State::upload` then **delete and re-upload everything**; there is no partial upload. GL textures are shared between materials by `Arc` pointer.
- These are read live from `frame.textures` on every draw:
  - DX9 uniforms, constants, animations, tints and pass state;
  - gen3 `values`, animations, blend and sort layer;
  - H4 per-draw values.
- These are baked at upload: main-program `uv_scales` and `detail`, and every GL texture.

**The colour picker and function editor.**
- They are app-level `Dialog`s: `ColorPopupWindow` and `FunctionPopupWindow` in `editor/dialogs.rs`. Each is stamped with its `KitId` and the tag's `layout_stamp`.
- Each keeps a **draft**: `MaterialColorPopup`'s channels, and `FunctionPopup::view` with its own undo history. They write to the tag **only on OK**:
  - `draw_color_popup` builds a `ColorPopupResult`: `FieldEdit`, `ShaderOp`, `ShaderParamOp`, `H2ShaderParamOp`, `StringListOp`, or `FunctionDraftColor`.
  - `draw_function_popup` builds a `FunctionEditBatch` with `push_function_edit(&paths, &last_applied, &view)`.
- That result goes out as `EditorCommand::ApplyPopupOps`, which `apply_doc_ops(…, UndoStep::Own)` applies. The command is refused if the layout stamp moved.
- The function editor says why it never writes before OK: *"Live-writing while a modal is open can invalidate classic H2 wrapper fields underneath combo boxes."*
- The function editor's own colour swatches open a second colour picker. Its OK goes back into the function's draft through `EditorCommand::FunctionDraftColor`, not to the tag.
- `apply_deferred_ops(&mut TagDocument, DeferredOps, label)` (`core/document/apply.rs`) is plain core code. It can run on a throwaway document on a worker.

**Overlays.**
- `RenderCore::draw_overlays` (collision and physics) is the model to copy:
  - main program, culling off;
  - a depth-only prepass, then a constant-alpha blended colour pass with `LEQUAL` and no depth write, then optional edges.
- The main program skins in its vertex shader from `u_bones`, so it lines up with every pipeline.
- Gen3's albedo pass clears depth and rounds differently, so the highlight needs its own prepass and a polygon offset, never `EQUAL`.

## 3. Design decisions

### 3.1 A material is a shader, not a slot

The dropdown lists each **(shader path, group) once**. A `MaterialKey` stands for one shader and the preview material indices that use it, across:
- the model's own materials;
- the LOD model's;
- held and attached objects (weapons);
- BSPs;
- CE permutations.

Editing one shader changes every place it draws, as saving it would, so hovering it lights them all.

Empty paths (collision, physics) are left out. So are materials whose tag can't be read; they are listed greyed out with the reason.

Each row shows:
- the group icon (`tag_icon(group, game)`);
- the basename;
- a muted suffix naming where it draws when that isn't the opened object alone (`assault_rifle (held)`, `LOD`, `bsp 0`);
- a dot when the material is modified.

Hovering a row shows the full path. The list is searchable, as the Permutations dropdowns are (`picker_button` / `picker_results`).

A material with **no batches** (CE `modifier shader`, H2 hologram and shields) highlights **every batch of its owner object**, because it draws over all of them. The row says "(overlay)".

### 3.2 "Modified" is derived, never stored

A material is modified when its document is dirty: `parsed_tags[key].dirty.is_set()`. The editor keeps **no list of its own** to drift from the document. If the shader is also open in a tab, there is one document, so an edit in either place shows in both, and Save or Reset in either covers both. This follows the [[unconsulted-state-defects]] rule.

### 3.3 Documents the editor opens

- When a material is picked whose shader isn't open, the editor loads it as a **document without a tab**. The kit records it in a new `Kit::held_documents: HashMap<String, HoldReason>`, here `HoldReason::MaterialEditor`.
- Being held makes the document count everywhere a tab would count it today:
  - `handle_tag_loaded` keeps a load for a held key;
  - `check_open_tags_in` checks held keys for changes on disk;
  - `close_action_tag_keys` includes dirty held keys for CloseApp and CloseKit, and for CloseAllTabs or CloseTab when the tab being closed is the preview that holds them.
- This also fixes the existing "can't quit" hole for any other document left dirty without a tab, such as `open_string_transfer`'s.
- **A clean held document is released** (`drop_document`) when its preview tab closes, or when the editor moves to another material while it's clean.
- **A dirty one stays held until it is saved or reset.** Closing the preview's tab puts dirty held documents in the same save prompt as the tab, under its name.

### 3.4 The window belongs to the pane, not the DialogHost

`Dialog`s draw read-only and only send commands (ARCHITECTURE.md, "Dialogs"). The shader UI needs `&mut KitView` (edit buffers, row heights, render-method caches), so it can't run inside a `Dialog`.

The precedent is **Layout Differences** (`editor/layout_diff.rs` `draw_layout_diff_window`): an `egui::Window` that the tag pane draws, registered in the smoke-test table by source file. The Material Editor works the same way:
- `draw_tag_pane` draws it after `draw_tag` returns, once the `model_preview` borrow of `view.caches` has ended.
- It shows only while its preview's pane is drawn. A background tab hides it, and it comes back with the tab.
- Its open/closed state and selection live on `ModelPreviewState` (`material_editor: Option<MaterialEditorState>`).
- Its egui id is salted with the pane's `scope` and the preview key, so two splits of the same model don't share a window.

### 3.5 Hosting the editor body: one renderer, not two

`draw_tag_pane` is split in two:
- **`draw_tag_pane`** keeps the header, the disk banner and the read-only label.
- **`draw_tag_body(cx, ui, kit_index, entry, scope, inputs, view, editor)`** does everything from "`let Some(doc) = …`" down: builds the `FieldEditContext`, calls `draw_tag`, stamps the popups, and sends `PaneDrawn`.

The Material Editor calls `draw_tag_body` for the shader's entry, with scope `"{pane scope}\u{1f}material_editor"`. That gives it the same edit context, popups, undo coalescing (its own `PaneDrawn` for the shader key every frame) and draft flushing, with no second copy of the scaffolding. The [[fixes-must-meet-the-audit-bar]] rule rules out a second copy.

The header stays out, because the window has its own (§4). The shader's documents are never models, so there's no nested preview.

What two scopes on one document still share (§6 of the research), which is fine:
- text drafts are keyed `"{tag}|{path}"`;
- the grid's model memo is keyed by document id;
- grid column widths are global.

### 3.6 Live edits: re-resolve the edited material only

The existing overlay reloads the whole model after 0.3 s, which is seconds on a Spartan. The editor gets its own path, `maybe_request_material_resolve`:

1. **Detect.** Each frame, in `apply_pane_drawn`'s post-draw hook beside `maybe_request_model_textures`, compare each held material's `content_stamp()` with the stamp its textures were last resolved from (`MaterialEditorState::resolved_stamps`). A difference starts a job.
2. **Job.**
   - Serialize the edited document on the UI thread. A shader is small, and this is what `snapshot_edits` already does.
   - Spawn a worker under `KitStamp`. It parses the bytes, seeds the `TagReader` memo with them at the shader's (path, extension, group), and resolves **only that material's indices** with `resolve_one_material`. `resolve_one_material` is made `pub(in crate::app::model_preview)`.
   - Reads are recorded as `ReadAs::Edits(stamp)` and merged into `state.reads`, so file monitoring covers any bitmap the edit newly refers to.
   - Only the newest job counts (a generation counter), with **no debounce**. A drag sends a stamp each frame; results for older stamps are dropped.
3. **Caches.** A `ResolveCaches` persists on the preview for the editor's lifetime. It keeps kit names, `Gen3Sources`, the HLSL includes, the H2 template, the CE `model_common` and camo, and decoded bitmaps. **Every entry keyed by a held document's path is evicted** before a job runs. That covers Reach reference shaders and CE extra-layer shaders that are being edited themselves. Without this cache, a cold single-material resolve pays the HLSL include parse again each time.
4. **Object sharing.** `finish_dx9_layers` is made re-runnable:
   - Each material's resolve output is kept **unshared** (`MaterialTextures::own_layers`) next to the shared view, and sharing is recomputed from the own layers for the edited material's whole object.
   - Without this, a re-resolved plain material loses its hologram/shield layers. A re-resolved overlay shader wouldn't hand its passes out again.
5. **Apply.**
   - The result replaces entries `[k…]` in a clone of the `Vec<MaterialTextures>`. The other entries are `Arc`-shared, so this is cheap.
   - Each material gets its own **upload id** (`ModelPreviewData::material_ids: Vec<u64>`), next to `textures_id`.
   - `sync_textures` and `Gen3State::upload` re-upload only the materials whose id changed. GL textures are reference-counted by image `Arc` pointer before they're deleted, because they are shared.
   - `textures_id` keeps meaning "the whole list was replaced".
6. **Derived things follow.** Some views are computed from the resolved textures: shield-only and never-shield parts, function sliders and the shield-damage gate, CE's A–D outputs panel, and footer notes. They have to be **computed from `textures` on each read, not cached at load**. The implementation step audits each one (`frame/functions.rs`, `ce/panel.rs`, `frame/mod.rs` `without_shield_parts`, `panel/viewport_size.rs`).
7. **Full loads agree.**
   - While the editor holds dirty materials, **every** load of this preview reads their edits, whatever the Show Unsaved Edits toggle says: `open_tag_edits` always includes this preview's held documents. Refresh Model, a weapon change or a Game Shaders toggle therefore never drops them.
   - The 0.3 s full-reload check treats this preview's held documents as up to date, so they don't also trigger a reload.
   - Other previews of the same shader keep today's behaviour: they follow their toggle, or reload on save.
8. **Saving doesn't reload our own preview.** Our reads are recorded `Edits(stamp)`, `mark_saved` keeps the stamp, and `any_changed` ignores Edits reads.
9. **Reset re-resolves from disk.** The document is dropped or reloaded, the stamp check sees it gone, and the job resolves from the file.

10. **Key resolves by their input bytes.** Each job is keyed by a hash of the bytes it resolves from, as well as the stamp. A job whose bytes equal those already drawn isn't run. That is what makes OK on a draft (§3.7) cost nothing.

A later speed-up, measured first and not part of the first build: a value-only edit (a constant, a colour, an animation) can skip decoding by reusing the material's existing images and swapping only the `Arc`, because those values are read live. The resolver already shares images by path inside the persistent caches, so most of that saving may come for free.

### 3.7 The colour picker and function editor preview live

While the colour picker or the function editor is open on a material in the editor, its **draft** draws in the preview as it changes: each drag of a hue, each moved graph point. The document stays untouched until OK, as it does now.

- **OK** applies the edit exactly as today. It is one undo step, and the material becomes modified.
- **Cancel**, or closing the popup, puts the preview back to the document.

So the popups keep their contract, and the H2 hazard the function editor guards against never comes up.

**One builder for the draft and the commit.** The code that builds OK's result is pulled out into non-consuming methods:
- `MaterialColorPopup::draft_result(&self) -> Option<ColorPopupResult>`
- `FunctionPopup::draft_batch(&self) -> Option<FunctionEditBatch>`, which is `push_function_edit` against `last_applied`

OK calls the same methods. The preview therefore can't show anything OK wouldn't write. That is the test (§6, step 8).

**Only pickers opened in the Material Editor preview live** (owner's decision, 2026-11-26). A picker opened from the shader's own tab, or from anywhere else, behaves exactly as today, even when a Material Editor is showing the same shader.

**Which pickers those are.**
- When `draw_tag_body` is drawn by the Material Editor, it stamps the colour and function popups it opens with `live_preview: Some(LivePreviewTarget { preview_key, draft_id })`:
  - `preview_key` is the model tag whose preview opened it;
  - `draft_id` is a fresh `u64` for this popup.
- The stamping happens where the pane already stamps `kit` and `opened_at`. It goes on the new fields of `ColorPopupWindow` and `FunctionPopupWindow`.
- Every other caller passes `None`, and those popups never publish.
- The colour picker that a live function editor opens for its own colours inherits the same target.

**Publishing the draft.**
- Each frame, a popup with a target compares its draft with the last one it sent. When it changed, it sends `EditorCommand::DraftPreview { kit, preview_key, draft_id, tag_key, opened_at, ops: Option<DeferredOps> }`. `ops` is `Some` while the draft differs from the tag, and `None` when it matches or the popup closes (OK, Cancel, or Esc).
- The handler stores the draft on **that preview's** editor: `model_previews[preview_key].material_editor.draft: Option<DraftPreview { draft_id, tag_key, ops, layout_stamp, seq }>`. The draft can't reach any other preview.
- It is dropped:
  - when the document's `layout_stamp()` moves (the same refusal OK makes);
  - when the document or the preview closes;
  - when `close_tag_popups` closes the popup;
  - when **no open popup carries its `draft_id` any more**. `DialogHost` replaces a popup of the same type when another opens: a picker opened from a tab replaces the editor's picker, and the replaced one never gets to send `None`. The check runs each frame, with `dialogs.any`.

**Colours inside the function editor.**
- The function editor's own colour picker has the `FunctionDraftColor` target. Its draft goes into the function editor's draft as a **tentative** colour: `FunctionPopup::tentative_color`, set by a new `EditorCommand::FunctionDraftColorPreview`.
- The function's `draft_batch` applies it, so dragging that picker previews the function's colours live too.
- Its OK makes the colour permanent in the function draft, as today. Its Cancel clears the tentative colour.

**Resolving a draft.** The live path (§3.6) takes the draft into account:
- The change check compares `(content_stamp, draft seq)`, not the stamp alone.
- A job for a material with a draft sends the document's bytes **and** the draft ops to the worker. `DeferredOps` isn't `Clone`, but the popup builds new ops every time it sends, so none is needed.
- The worker parses the bytes into a throwaway `TagDocument::clean`, runs `apply_deferred_ops` on it, resolves from the result, and records the read as `ReadAs::Edits`.
- **After OK**, the document holds what the draft held. The input-bytes key (§3.6, point 10) then finds nothing new to resolve, so the preview doesn't flicker.
- **After Cancel**, the draft is gone. The job resolves from the document and shows the material as it was.
- Function edits change animated values (CE and H2 constants and animations, gen3 `animations`, H4 per-draw functions). These are read from `frame.textures` on every draw, so a re-resolved material picks them up at once.

**Indicator.** While a draft is showing, the material's row in the window says "previewing unsaved picker changes". This keeps a material that looks changed but isn't modified from confusing anyone.

### 3.8 The hover highlight

- `ModelPreviewState::highlighted_material: Option<MaterialKey>` is set while a dropdown row, or a modified-list row, is hovered, and cleared when it isn't.
- `build_frame` turns it into `ModelGpuFrame::highlight: Option<Highlight { batches: Vec<usize>, color }>`:
  - the batch indices are taken from `visible_batches` and `shield_batches` whose `material_index` is in the key's set;
  - for an overlay-only material, every visible batch of its owner object;
  - `BatchOutline::Lines` batches are skipped.
- `RenderCore::draw_material_highlight` runs in `ModelGlRenderer::paint` **after `game.finish`, before `end_frame`**, so bloom, composite and sprites don't cover it. It uses:
  - the main program, the slot VAO and `bind_material(None)`;
  - a depth-only prepass with `polygon_offset(-1, -1)`;
  - a colour pass with `LEQUAL`, no depth write, a constant-alpha blend (about 0.45) and `u_unlit = 1`;
  - a line pass for the edges.
- All GL state is restored afterwards, as `draw_overlays` does.
- It works in every shading mode, including Wireframe, because the prepass lays its own depth. It works with skinning, cloth and CE flags (they share the slot VBO), instances, the LOD model, and held objects.
- The colour is a fixed scene colour. It is exempt from the palette gate like the rest of `renderer/`, and is chosen to read against all four backgrounds (Theme, White, Black, Green Screen). **Decided (owner, 2026-11-26):** a translucent amber fill with white edges, shown **only while a row is hovered**. The picked material isn't kept lit.
- The popup draws after the viewport in the same pass, so it calls `ctx.request_repaint()` whenever the hovered key changes, set or cleared. Otherwise the highlight lags a frame.

### 3.9 Editable `material` tags, everywhere

Halo 4, Halo 2 Anniversary and Halo 5 `mat ` tags get an editable view, in their own tab as well as in the Material Editor. Today `editor/material/mod.rs` `draw_material_struct_fields` / `draw_material_field` / `draw_material_value_row` paint every value as text. They are never given the `FieldEditContext`. Swatches open a colour picker with no write path, and `draw_material_function_value_row` opens the function editor with an empty tag key and `editable: false`.

**The tag** (`definitions/halo4_mcc/material.json`; H2A and H5 to be compared field by field before building):
- `material shader` (`mats`);
- `material parameters`. Each element holds:
  - `parameter name`, `parameter type`, `parameter index`;
  - the values: `bitmap` / `bitmap path`, `color`, `real`, `vector`, `int/bool`;
  - sampler words and bytes;
  - `function parameters`, each a typed `mapping_function`;
  - display metadata;
- `postprocess definition`: textures, constants, functions, externs, blend modes, physics materials;
- physics material names, `sort offset`, `alpha blend mode`, `sort layer`, flags, `Transparent Shadow Policy`.

**The view.** It works the way the render-method grid does: one row per parameter the material's `mats` declares, with a default column and an override column. An authored parameter is an override; one the material leaves out shows the shader's default and can be overridden.
- Each row's edit kind writes through the context (`edit.pending`), like `ShaderRowEdit`:
  - a value edit at `material parameters[i]/<value field>`;
  - the colour picker with a real write path (`with_write` / `ColorFieldWrite` on `color`);
  - the function editor opened **editable**, with the tag key and the `function parameters[j]/function` path;
  - the bitmap reference cell shared with the grid (Open, Browse, completion, drag and drop).
- Overriding a default adds a `material parameters` element with the right name and type. Clearing an override removes it. This is a new `MaterialParamOp`, handled in `apply_deferred_ops` beside `ShaderParamOp`.
- The other root fields (`sort offset`, blend mode, sort layer, flags, shadow policy, physics material names) go in a section under the parameters.
- `postprocess definition` stays read-only, behind expert mode.
- The model for the view is built once per document revision, as `ShaderEditorModel` is.
- The parameters and their defaults come from the same reader the preview uses: blam-tags `material::{Material, MaterialShader}`. Display names, groups and help text come from the parameter's `display *` data.
- The layout follows Foundation's own material view (`BlamPlugin` `TagTemplateView` in `~/Source/HREK_Foundation`): read it before designing the rows, as for the shader grid ([[foundation-comparison]]).

**Research needed before saving, the possible blocker.** `postprocess definition` is what the tool builds from the parameters (gen3's equivalent: [[h3-render-method-bake]]). Before Save is enabled for an edited `mat `, establish from the Halo 4 tool and the game, never by guessing ([[read-the-engine-dont-curve-fit]]):
1. Do kit `.material` tags ship a filled postprocess? Run a kit census.
2. Does the game read the postprocess or the parameters?
3. Does the tool or Foundation rebuild it on save or import?

The plan then forks:
- **The game rebuilds it, or reads the parameters:** nothing more is needed.
- **The game reads a stale postprocess:** Save must rebuild it. blam-tags `material::bake` already bakes parameters over the shader for the preview; whether it can write a tool-identical postprocess is part of the research.
- **Until this is settled,** an edited `mat ` can be edited and previewed, but Save says why it is held back.

**Gates that stay as they are.** Monolithic H4 builds stay editable and never saveable ([[monolithic-tags-editable-not-saveable]]). Halo 5's `structure_locked` keeps blocks fixed: overriding and clearing parameters adds and removes elements, so on Halo 5 only existing overrides can be edited.

## 4. The window

```
┌ Material Editor — masterchief.model ─────────────────────── ✕ ┐
│ [▣ masterchief_body           ▾]  ‹ ›          [Open in Tab]  │
│  objects\characters\masterchief\shaders\masterchief_body       │
│  Cannot be saved: read-only build            (only when so)   │
├────────────────────────────────────────────────────────────────┤
│  (the shader's own editor: grid / H2 grid / field tree,        │
│   scrolling inside the window)                                 │
├────────────────────────────────────────────────────────────────┤
│ Modified (2)                          [Save All] [Reset All]  │
│  ● masterchief_body                   [Save] [Reset]          │
│  ● visor             (held: …)        [Save] [Reset]          │
└────────────────────────────────────────────────────────────────┘
```

- It is resizable, with a default size of about 560×640. It can't be narrower than `SHADER_GRID_MIN_WIDTH` (420) plus padding, since the grid scrolls sideways under that.
- The body runs inside `draw_shader_scroll_area`, as in a tab, and calls `scroll_during_drag`.
- The arrows beside the dropdown step through the materials, as the Permutations arrows do.
- **Open in Tab** opens the shader in its own tab (`ReferencesCommand::Open`). It's the same document, so the edits carry over.
- The modified list shows only when something is modified. Its rows highlight on hover too.
- **Save** follows the close prompt's routing for each key (`close_prompt_save_route`: File / ContainerInPlace / NewContainer, after `focus_kit`). A changed-on-disk conflict opens the existing `DiskSaveConfirm`.
- **Save All** loops over the keys, stops at the first refusal and reports it.
- **Reset** asks for confirmation, since it can't be undone: "Discard the unsaved changes to *name*? This also discards changes made in its tab." The note appears only if a tab is open. Then it calls `discard_tag_changes(kit, key)`, which drops a held document and reloads a tab's.
- **Reset All** asks once, listing the materials.
- An unsaveable document (`unsaveable_reason`: monolithic, big-endian) shows the reason in the header and disables Save with that reason on hover. Edits still work, as in a tab.
- In a read-only kit, the body is drawn read-only, as the pane does.
- **Undo and redo.** With focus inside the window, Ctrl+Z and Ctrl+Shift+Z act on the shown shader. `undo_current_tag` / `redo_current_tag` take an explicit `(kit, key)` target: the editor's shader when the window holds focus, else `selected_key` as now. `restore_snapshot` takes the kit index instead of assuming the active kit.
- When the Shading mode is Solid or Wireframe, a muted line under the dropdown says edits show under Textured or Game Shaders. Plain Textured only draws base maps.
- When a reload is in progress (`state.reloading`/`settling`), the dropdown keeps the previous list and the editor body stays usable. Its edits land in the new load (§3.6, point 7).

## 5. Per-game matrix

| Game | Shader groups | Editor body in the window | Live resolve route | Notes |
| --- | --- | --- | --- | --- |
| Halo CE (MCC) | `shdr` children: senv, soso, schi, scex, sotr, sgla, smet, spla, swat | **Generic field tree**, the only UI CE shaders have | `resolve_ce_shader(…, permutation)` | The same shader under two permutations resolves per entry. The `modifier shader` overlay shares into its object (§3.6, point 4). `sotr`/`schi`/`scex` extra layers are other shader tags, which can be picked in the dropdown only if the model lists them. Game Shaders need `<tags>/../shaders/fx` ([[ce-game-shaders-need-kit-shader-source]]). The A–D function panel follows the textures. |
| Halo 2 | `shad` | H2EK grid (classic Halo2V4 only; others get the field tree) | `resolve_h2_shader` | The template (`stem`) and its `spas` come from disk, as do `.psh` files. A template switch leaves the old parameters under "unused", as today, with no pruning. Hologram/shield/overshield shaders have no batches and highlight the whole object. |
| Halo 3 / ODST | rm family (ODST adds `rmss`, `rmbk`) | Render-method grid | shared rm path + `prepare_gen3_material` | The HLSL translate is the costly part, so it must hit the persistent `Gen3Sources`. Editing `definition` (expert mode) re-resolves against the new rmdf, with no parameter reconciliation, as today. |
| Reach | rm family + `rmgl` `rmfu` `rmfs` `rmmx` `rmmm` `rmss` | Render-method grid | as above + `shader_version_update` + reference shader | The reference shader is cached by path, so it's evicted when it is itself held. Shield-only / never-shield flags move parts between passes live. |
| Halo 4 | mostly `mat ` (plus rm groups such as `rmwf`) | rm groups: grid. `mat `: the **editable material view** (§3.9) | `resolve_h4_material` | It bakes the material over its `mats` from `material parameters` (blam-tags `material::bake`), so parameter edits show in the preview. Saving is gated on the `postprocess definition` question in §3.9. |
| H2A, Campaign Evolved | — | — | — | `textures: false`, so no button. |

Halo 4 `mats` (material_shader), gen3 `rmdf`/`rmop`/`rmt2`, `hlsl_include` and globals are shared infrastructure. They aren't listed. The existing save-triggered full reload still covers them when edited in their own tab.

## 6. Steps

Each step builds on its own and gets its own commit. The feature ships as one PR ([[one-pr-for-a-multi-phase-feature]]). Gates run at the end ([[sprint-code-work-gate-at-the-end]]).

1. **Held documents** (`kits`, `documents`).
   - Add `Kit::held_documents` and keep held loads in `handle_tag_loaded`.
   - Include held keys in the disk checks and in `close_action_tag_keys`. This fixes the "can't quit with a tab-less dirty document" hole.
   - Make undo and redo target `(kit, key)`.
   - Tests:
     - a dirty held document appears in the CloseApp prompt and the app can then quit;
     - the disagreeing case: an unheld, clean document doesn't appear;
     - a held load lands;
     - an unheld load is still dropped;
     - undo hits the held document, not the selected tab.
2. **Split `draw_tag_pane`** into header and `draw_tag_body`. No behaviour change. The existing pane and frame tests are the proof.
3. **Editable `material` view** (§3.9), in the tag's own tab first.
   - The parameter model, row edit kinds, `MaterialParamOp`, the editable colour picker and function editor, the other root fields, and expert-mode postprocess.
   - The postprocess research comes first. Save stays held back for edited `mat ` tags until it is answered.
   - Tests:
     - a commit-path test for each value kind on real H4 kit materials: typing changes the document, and a picked colour and an edited function land at their paths;
     - overriding a default adds an element and clearing removes it, with the disagreeing case on Halo 5, where `structure_locked` refuses both;
     - a kit sweep (H4 and H2A, plus H5 where its tags are readable): every material builds its view;
     - the preview bakes an edited parameter: its uniforms change, and a parameter the shader doesn't declare doesn't.
4. **Material list.**
   - `MaterialKey` grouping over `preview.materials`: owners, LOD, BSP, overlay-only and empty-path rules, and labels.
   - Unit tests on a synthetic preview:
     - duplicates merge;
     - empty paths drop;
     - an overlay-only material maps to its object's batches.
   - Kit-gated tests (`BLAM_TEST_*`) on a real model per game: the list has the shaders the footer names.
5. **Header button, window shell, dropdown and highlight.**
   - Add the button, the pane-owned window and the smoke case (register `model_preview/panel/material_editor.rs` in the `cases()` table).
   - Add `highlighted_material` → `ModelGpuFrame::highlight` → `draw_material_highlight`.
   - Headless GL shots of the highlight ([[headless-gl-preview-shots]], [[preview-shots-must-cover-themes-and-close-ups]]):
     - under Solid, Textured and Game Shaders;
     - on a gen3 model, a CE model and an H2 model;
     - on a skinned pose;
     - with a held weapon;
     - on light and dark backgrounds, plus a close-up.
   - A pixel test that the highlighted parts change colour and the others don't. That is the disagreeing case.
6. **Editor body in the window.**
   - The editor loads a held document on pick and draws `draw_tag_body` in the window.
   - Edits apply through its `PaneDrawn`, and the popups are stamped with the shader's layout stamp.
   - Until step 6 lands, the preview shows the edits through a full reload. The interim forces `open_tag_edits` to include held documents.
   - A test drives the commit path ([[shader-structural-references-editable]]: "a widget that renders is not a widget that commits"). It types into a grid cell in the window, checks the document changed and is dirty, and checks the tab's document is the same one.
7. **Live per-material resolve.**
   - Persistent `ResolveCaches` with path eviction.
   - A re-runnable `finish_dx9_layers` that keeps own layers.
   - Per-material upload ids, plus partial `sync_textures` and `Gen3State::upload`.
   - The newest job wins.
   - Audit the derived views (§3.6, point 6).
   - The key test is an **equivalence test per game kit**: for every material of a real model, resolving it alone after an edit-and-revert must equal `resolve_model_textures()[k]`, shared layers included. It must also fail when sharing isn't re-run (the disagreeing case).
   - A glcalls gate checks that one material's edit uploads only that material's textures.
8. **Live colour and function drafts** (§3.7).
   - Add `draft_result` / `draft_batch`, shared with OK.
   - Add `EditorCommand::DraftPreview`, `KitView::draft_previews`, the tentative colour in the function editor, and draft-aware jobs.
   - Tests:
     - **equivalence**, for each popup result kind (field edit, shader op, shader param op, H2 shader param op, function edit and function data op): the throwaway document after the draft is byte-equal to the real document after OK;
     - the document stays clean, at the same revision, while a draft changes;
     - Cancel resolves back to the pre-open textures;
     - the disagreeing case: a moved layout stamp drops the draft, and the preview shows the document;
     - the other disagreeing case: a picker opened from the shader's own tab, with the Material Editor open on that shader, publishes nothing and the preview doesn't change while it is dragged;
     - a tab's picker replacing the editor's picker drops the editor's draft;
     - OK after a draft starts no second resolve.
   - A kit-gated pixel test on a gen3 colour parameter: the parts change colour while the picker is dragged, before OK.
9. **Modified list, Save and Reset.**
   - Per-key routing, confirmations, saveability, and read-only kits.
   - Tests: save writes only that key; our preview doesn't reload after the save; Reset drops a held document and the preview goes back to the file; Reset of a document with a tab reloads the tab; the unsaveable path refuses with its reason.
10. **Docs.**
   - A "Material Editor" section in `docs/model-preview.md`, including a note in "Keeping up with saved tags" that the editor's edits always show in its own preview.
   - The header list.
   - The help section anchor.

## 7. Risks and traps

- **Borrowing.** `model_preview` is borrowed from `view.caches` while `draw_tag` runs. The window must be drawn after that borrow ends, which is why it's drawn from `draw_tag_pane` and not from `draw_model_preview_panel`.
- **`TagEntryLocation` arms.** The save routing must handle LooseFile, Container, NewContainer and Monolithic ([[ce-tag-authoring]]: grep every new `match` for the missing arm).
- **The reads overlay covers loose files only.** Monolithic H4 builds can't save anyway, but their full reloads would drop edits. The live path seeds the `TagReader` directly, so it works there. A later full reload in a monolithic kit loses them until the next edit; this has to be handled or stated.
- **CE projects.** Autosave already stashes every dirty `parsed_tags` document, held ones included. After a restore, a held document comes back dirty with no editor holding it. It must be held again, or listed in the prompt on its own. Step 1 must cover this.
- **Undo coalescing.** The window must send its `PaneDrawn` every frame it is shown, as a pane does, or a drag never closes its undo step.
- **Re-entrancy of popups.** The colour and function popups from the window are app-level `Dialog`s stamped with the **shader** document's `KitId` and `layout_stamp`. `close_tag_popups` closes them when that document goes.
- **Panic safety.** `resolve_one_material` is already wrapped in `catch_unwind` by its caller. The single-material job must keep that wrapper, so a custom kit's shader can't leave the material resolving forever.

## 8. What this plan leaves out

- Editing infrastructure tags: rmdf, rmop, rmt2, `mats`, `stem`, `hlsl_include`, globals.
- Picking a material by clicking the model. This is a natural follow-up: the highlight's batch-to-material map gives the reverse lookup.
- Reconciling parameters when a template or definition changes.
- The value-only fast path (§3.6, after point 9).

## 9. Owner decisions (2026-11-26)

1. **Halo CE:** the window shows the field tree, the same UI a CE shader has in its tab. No new CE layout.
2. **`mat ` materials:** made **editable everywhere**, in their own tabs and in the window, as part of this feature (§3.9, step 3).
3. **Reset with the shader also open in a tab:** it resets the one shared document, and so the tab's edits too. The confirmation says so (§4).
4. **Highlight:** translucent amber with white edges, only while hovering (§3.8).
5. **Closing the model's tab** with modified materials: they're listed in the same save prompt as the tab (§3.3).
6. **Colour picker and function editor live preview:** only for pickers opened inside the Material Editor (§3.7).
