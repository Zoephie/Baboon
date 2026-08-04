# Baboon

**Baboon** is a native desktop viewer and editor for Halo tag files, built in
Rust with [`eframe`/`egui`](https://github.com/emilk/egui). It links the [`blam-tags`](https://github.com/camden-smallwood/blam-tags) engine directly for
byte-exact tag reading, editing, and asset extraction, and presents a
Guerilla-style editing surface for working with the loose tag files shipped in
the **Halo: The Master Chief Collection** editing kits — no round-trip through
the official tools required.

Open a single tag, an entire editing-kit `tags/` folder, a monolithic tag cache,
or a Halo: Campaign Evolved install — **several at once**, each in its own
workspace, side by side. Browse and search the tag tree (by name *or* field
value); edit fields, blocks, shaders, and functions inline with full undo/redo;
preview bitmaps and 3D models; trace references and diff tags; duplicate and
delete tags; and extract geometry, textures, and animations — all from one
application. For Halo: Campaign Evolved there is a second workspace, **Chimp**,
for editing the game's Unreal packages directly.

> Baboon is the GUI front end for the `blam-tags` project. The library does the
> binary tag parsing; Baboon is the interactive editor built on top of it.

---

## Supported games

Baboon recognises and auto-configures itself for the following games. The MCC
editing kits are detected from the kit's root folder name; Halo: Campaign
Evolved is mounted from its game install (see the footnote and *Campaign Evolved
mods* below):

| Game                     | Folder                        | Game identifier  |
| ------------------------ | ----------------------------- | ---------------- |
| Halo CE                  | `HCEEK` / `H1EK`              | `haloce_mcc`     |
| Halo 2                   | `H2EK`                        | `halo2_mcc`      |
| Halo 3                   | `H3EK`                        | `halo3_mcc`      |
| Halo 3: ODST             | `H3ODSTEK`                    | `halo3odst_mcc`  |
| Halo: Reach              | `HREK`                        | `haloreach_mcc`  |
| Halo 4                   | `H4EK`                        | `halo4_mcc`      |
| Halo 2: Anniversary (MP) | `H2AMPEK` / `H2AEK`          | `halo2amp_mcc`   |
| Halo: Campaign Evolved † | game install (IoStore paks)   | `haloce_evolved` |

The MCC game is also detected from a folder literally named after the game id
(e.g. `halo3_mcc`), and **custom editing-kit folder names** can be mapped to a
game in *File → Settings* for non-standard layouts.

† **Halo: Campaign Evolved** is not an MCC editing kit — it's the UE5 remake of
Halo 1 on a modified Reach engine, whose Reach-format tags are cooked into UE5
IoStore paks. It's mounted from its game folder via **Load Folder** rather than a
`tags/` directory; see *Campaign Evolved mods* below.

Per-game group-name tables and schemas are loaded from
`definitions/<game>/*.json`. Release builds place the `definitions/` folder next
to `Baboon.exe`, which keeps the schemas inspectable and editable without
rebuilding the app.

---

## Features

### Loading tag sources

Baboon can open four kinds of source, each on a background thread so the UI
never blocks. Opening a source that is already open switches to it rather than
loading a second copy; opening a new one adds a workspace beside the existing
ones rather than replacing them:

- **Single tag** — open any individual loose tag file.
- **Loose tags folder** — point at an MCC editing-kit `tags/` directory (or the
  kit root, e.g. `H3EK`; Baboon locates the `tags` folder and identifies the
  game automatically). The folder tree is loaded **lazily**, expanding
  directories only as you open them, so even a full kit opens instantly.
- **Monolithic cache** — open a Halo 4 `blob_index.dat` monolithic tag cache and
  browse its contents as if they were loose files (read-only).
- **Campaign Evolved container** — point **Load Folder** at the Halo: Campaign
  Evolved game directory (or its `Meteorite/Content/Paks`). Baboon finds the
  container directory inside the install itself, and remembers the folder you
  picked. It auto-detects the UE5 IoStore paks, mounts them as one read-only virtual filesystem, and
  presents the Reach tags exactly like loose files. Every pack (the shared base
  chunk plus the per-level chunks that carry each mission's scenario and BSPs) is
  merged into a single lowercase tag tree. See *Campaign Evolved mods* below.

Tag files are identified by probing their 64-byte header for the `BLAM`
(big-endian) / `MALB` (little-endian) magic, so non-tag files in the tree are
silently skipped.

### Tag browser

- **Folder view** — the on-disk directory hierarchy, with a **per-group icon**
  beside each tag (and on its editor tab) for quick visual scanning.
- **Groups view** — tags regrouped by tag group (e.g. *biped*, *weapon*,
  *render_model*), with friendly names resolved from the definition tables.
- **Recent folders** — a quick-open list in the File menu, on the workspace
  tab bar, and on the welcome screen; entries can be removed individually or
  cleared.
- **Boolean search/filter** — a fast, memoised filter supporting space-separated
  **AND**, `|` **OR**, and `^prefix` / `suffix$` / `^exact$` anchors matched over
  the filename, group four-CC, and group name. A label flags degenerate filters
  (an empty `|` operand, an anchor-only term). Results are cached and recomputed
  only when the query, source, or mode changes — not per frame — so the tree
  stays responsive across 100k+ entry kits.
- **Sort** — order each folder/group by natural, name, or type.
- **Reveal in tree** — jump the browser to any tag (e.g. from a search result),
  force-opening its ancestors and scrolling it into view.
- **Background indexing** — a full recursive scan runs in the background to power
  Groups view and global search without expanding every node first. The
  completed index is **persisted** (per game, to `%APPDATA%\Baboon`) so
  subsequent launches skip the scan entirely.
- **Context actions** — per-tag and per-folder right-click actions for JSON dump,
  raw extraction, bitmap/geometry/animation extraction, *Rename / Move* (with
  automatic reference fix-up across every referencing tag), *Duplicate*,
  *Delete*, and *Open in File Explorer*.

### Duplicating & deleting tags

Right-click any tag in the browser to **Duplicate** it. The dialog edits the leaf
name only — the parent folder, tag group and extension are fixed — and validates
the name against Windows' rules and the tags already in the source before
anything is written. The copy opens in a tab and is revealed in the tree beside
the tag it came from.

- **Loose kits** — the copy is written next to the original as a new file. If the
  source tag has unsaved edits, the copy takes the edited bytes and the original
  keeps its own; a name that already exists is refused rather than overwritten.
- **Campaign Evolved** — the copy is written **into the game's own pak**, in
  place: the new package is appended to the container's `.ucas` and the `.utoc`
  is rewritten to address it, with a new package identity and export hash. This
  changes shipped game files, so it always confirms first, naming the exact pack
  (mod or shipped) and its `.utoc` path.

**Delete** removes a tag again. A loose tag is *moved*, not erased — it goes to
`%APPDATA%\Baboon\deleted-tags\<game>\<timestamp>\…` so it can be recovered by
hand. A Campaign Evolved tag is retired from its container: its chunk slots are
emptied and its package-store entry removed, without moving any other chunk.

Deleting a Campaign Evolved tag is deliberately limited to **copies Baboon
itself created**. Once a copy is in a pak it is indistinguishable from a tag the
game shipped, so eligibility comes from Baboon's own records — a duplicate
ledger in `%APPDATA%\Baboon`, and the immutable backup written beside a container
before it is first modified — never from the container. Anything the game
shipped has Delete greyed out, and the library re-checks the same evidence before
touching a byte. The confirmation lists every tag that references the one being
deleted, or says plainly when no reference index is available.

Every in-place container write takes an immutable `.utoc` backup plus a manifest
beside the pak first, and reports the result — including the backup path — in a
dialog that stays until dismissed. A failed write restores the container and
changes nothing.

### Chimp — the Campaign Evolved package workspace

Campaign Evolved's tags are cooked into UE5 packages, and some of what the game
loads is not a tag at all. **Chimp** is a second workspace inside a loaded
Campaign Evolved kit for working on those packages directly, rather than forcing
Unreal concepts through the tag model. Switch to it from the workspace surface
toggle; it shares the kit's Paks root but keeps its own package index, documents
and editor state.

- Browse every cooked package in the mounted containers, by folder tree, by
  filter, or by type.
- Edit reflected properties against the game's schema (`.usmap`), with typed
  values, containers, and structs resolved to their real names.
- Preview textures, static meshes and skeletal meshes. Textures use the same
  viewer as `bitmap` tags, stepping through every mip level and — on a virtual
  texture with more than one layer — every layer. Campaign Evolved ships most of
  its art as UE5 virtual textures; those are reassembled from their tiles,
  borders removed, with the UDIM blocks laid out side by side. A base mip larger
  than the display can upload is noted and the largest mip that fits is shown
  instead; export is unaffected.
- **Extract Texture2D** — one menu entry that asks which image format first,
  then where to save. DDS, TIFF and PNG all split a UDIM virtual
  texture into `name.1001.…`, `name.1002.…`, … following the UDIM convention,
  ready to import back into Unreal as a UDIM set, and all three give each block
  the resolution it was authored at — a set that mixes 2048 and 1024 blocks
  exports each at its own size rather than magnifying the smaller ones. DDS
  additionally keeps the cooked pixel format (BC1–BC7 and the uncompressed
  formats) and the whole mip chain, so it is the bytes the game ships rather
  than a re-encode; TIFF and PNG are one flat RGBA8 image per block. Splitting
  can be turned off, writing the whole set as the single stitched image its
  tiles were reassembled into, for an engine with no UDIM support — blocks
  authored smaller are magnified onto the same grid, so it costs some of the
  detail splitting keeps.
- Extracting a static or skeletal mesh offers its textures alongside it — either
  every texture the materials reference, or just the ones named after the model —
  and asks the same image-format question, so they land beside the mesh in
  whichever format you picked rather than always TIFF.
- Save edited packages back into the container in place, or bundle them into a
  mod container.
- Edits are checkpointed to a recovery folder, so a crash or a restart does not
  lose work in progress.

### Search, navigation & cross-referencing

- **Find in fields (`Ctrl+F`)** — search field values, labels, or both in the
  current tag, all open tags, or the complete loaded source. Every matching
  substring is highlighted, and previous/next navigation reveals the matching
  field and selects the required nested block element.
- **Field-value search** — search across tags' *field values* (not just names),
  run on a background worker against an in-memory field index and optionally
  scoped to a tag group; results open in a clickable window.
- **Find references** — list every tag that references the current tag; click a
  result to open it and **jump straight to the exact field** where the reference
  lives (ancestor blocks expanded, the field scrolled into view and briefly
  glowed). When a tag references the target in more than one place, an expander
  under its row lists every occurrence, each individually clickable.
- **Content Explorer** — a reference-graph navigator centred on one tag: who
  references it (parents) and what it references (children), with back/forward
  history and a filter box.
- **Unreferenced tags** — scan for tags that nothing else points at.
- **Keyword tagging** — attach freeform keywords to tags (stored in a per-game
  sidecar, outside the tags) and browse or filter by them.
- **Scenario map IDs** — list scenario map IDs across the kit.
- **Tag Diff** — compare the current tag field-by-field against another open tab
  *or* any tag on disk; differences (changed values and block element-count
  mismatches) show in a table and export as TSV.

### Multiple games at once

Each loaded source is a **workspace** with its own browser, its own open tags,
its own indexes, and its own undo history, shown as a tab across the top of the
window.

- The **+** on the workspace tab bar opens another game, including from the
  recent folders list.
- **Drag a workspace tab** against the edge of a pane to split the window and
  work in two games side by side, each with its own browser.
- Closing a workspace prompts for anything unsaved in it, and closing Baboon
  prompts for every workspace that has unsaved work.
- With nothing loaded, the editor shows a **welcome screen** listing the open
  actions, your configured editing kits, and recent folders — each of which can
  be removed individually or cleared.

### Tabbed, splittable editor

- Open multiple tags as **tabs**, each with its group icon and an amber tint +
  ● marker when it has unsaved edits.
- **Split** the editor by dragging a tab against the edge of a pane, and view
  two tags side by side. Drag a tab onto another group to move it.
- Open **several games at once**: each is a workspace tab with its own browser
  and its own tags, and dragging a workspace tab splits the window between two
  games.
- **Right-click a tab** to reveal that tag in the browser tree, or to close all
  tabs / all tabs but this one.
- Tabs remain open until explicitly closed; the practical document limit is
  available memory. Drag a tab against a pane edge to split the editor and view
  two tags at once.

### Field editing

The editor renders the full tag structure — nested blocks, arrays, structs, and
pageable resources — with inline editing for loose little-endian tags:

- Scalars, integers, reals, strings, and `string_id`s.
- **Enums** and **bit flags** with named options.
- **Colors** via an interactive color-picker popup with channel parsing.
- **Tag references** with an *Open* button (Alt-click opens in a split beside
  the current pane)
  that resolves and opens the referenced tag — even if it isn't in the current
  index — an *Import* button on geometry references that re-imports the source
  asset via `tool`, **drag-and-drop** from the browser to set a reference, and a
  red highlight when a referenced tag is missing on disk.
- **Block-index fields** render as a dropdown of the target block's elements
  (with a leading `<none>`) plus a "go to" button to the referenced element.
- **Field documentation** — help text, units, and value ranges (recovered from
  the JSON schemas, since shipped tags strip them) shown on hover, plus Foundation-
  style **explanation blocks** inline.
- **Undo / redo** — every edit (field, block, structural) is journaled;
  `Ctrl+Z` / `Ctrl+Y` and the Edit menu step through the history.
- **Guerilla-style "Search fields"** — type a block or field name to filter the
  editor down to just the matching fields and the blocks/structs/arrays that
  contain them; everything else is hidden. Available on every field-tree tag,
  including sound tags. Blocks and structs keep their expand state as you page
  through a block's elements, and structs are expanded by default.
- **Expert mode** toggle to reveal advanced/normally-hidden fields.
- Monolithic-cache and big-endian tags are opened **read-only**; only
  little-endian loose tags can be saved back to disk.

### Block & array editing

Full structural editing of tag blocks, applied safely after each frame's render
pass:

- **Add**, **insert**, **duplicate**, and **delete** elements, plus **delete
  all** (with a confirmation modal for destructive ops). Fixed-size **arrays**
  omit the count-changing actions but support copy and in-place replace.
- **Copy / paste** elements — including the entire block — between two open tags
  of the same group, with compatibility re-validated by the library before
  insertion.
- **Replace** a selected element or an entire block from the clipboard.
- **Copy block as TSV** / **Paste TSV** — round-trip a block's leaf fields
  through tab-separated rows (e.g. via a spreadsheet).
- **Breadcrumb / jump-to-parent** — a `↑` control on nested blocks scrolls back
  to the parent block, with the path shown on hover.

### Shader & material editor

For `shader`, `material`, and `material_shader` tags, Baboon builds a
**Guerilla-style shader grid** instead of a raw field dump:

- Resolves the tag's render-method definition (`rmdf`) and options (`rmop`),
  caching them across tags.
- Shows bitmap, scalar, integer, color, and category parameters with their
  defaults, all editable inline.
- **Inline bitmap thumbnails** on bitmap-reference rows, with an enlarged
  preview on hover (works for classic Halo 1/2 bitmaps too).
- **Differs-from-default** indicator (an accent bar on changed rows) and a
  right-click **Reset to default**.
- **Resizable** label column (drag the divider) and the full parameter name +
  type shown on hover.
- Add optional **animated parameters** (e.g. bitmap transforms) from a context
  menu, and edit their animation **functions**.

### Function editor

An interactive editor for tag mapping functions (`TagFunction`), supporting the
editable function types — *identity*, *constant*, *linear*, and *linear key* —
with curve points, color graphs, input/range `string_id` selection (seeded with
common inputs like *time*, *frame*, *random*, *shield vitality*), and a
hex-blob round-trip channel that preserves arbitrary function data losslessly.

### Bitmap preview

For `bitmap` tags, a built-in texture viewer:

- Decodes the bitmap to RGBA (via `blam-tags`' bitmap decoder).
- **Image (sequence) and mip-level selectors** — step through every image in a
  multi-image bitmap and every mipmap level (the dimensions update accordingly).
- Per-channel **R / G / B / A** toggles, including alpha-only inspection.
- **Zoom-to-cursor**, **drag-to-pan**, zoom presets (25–400 % / fit), and a
  background-colour toggle behind transparent images.
- Under-cursor **pixel coordinate + RGBA readout**.
- Reports format, type, dimensions, and image count.

### Model preview

For `model` (`hlmt`) and `render_model` (`mode`) tags, a real-time 3D preview:

- Renders the model with orbit/pan/zoom camera controls.
- **Variant selector** — switch between the model's named variants and see the
  per-region permutation set applied; region/permutation choices can be tweaked
  and synced back to the variant.
- **Marker overlay** with a name filter, and a loading indicator while geometry
  resolves.
- Edit `render_model` **marker fields and names** inline.

### Sound playback

For `sound` (`snd!`) tags, an in-editor player auditions the tag's audio without
leaving Baboon — decoded in pure Rust by `blam-tags` and played through
[`rodio`](https://github.com/RustAudio/rodio). Baboon resolves each game's audio
storage automatically:

- **Halo CE** — inline Ogg Vorbis on each permutation.
- **Halo 2** — inline Opus, Xbox-IMA-ADPCM (mono / stereo / quad), or PCM, per
  the tag's compression and encoding.
- **Halo 3 / Reach** — FMOD-Vorbis subsounds paged out to the kit's FMOD banks
  (`<game>/fmod/pc/*.fsb`), resolved by permutation name.
- **Halo 4** — Wwise: the tag's event name is resolved through the game's sound
  packages (`<game>/sound/pc/*.pck`) — event → action → sound / container →
  media — and the referenced Wwise-Vorbis audio is rebuilt to Ogg and decoded.

A **play button per permutation** (or per event for Halo 4), a **Stop** control,
and a status line showing the current clip and its duration. Decoded audio is
cached, and the banks / packages are opened lazily on first play.

### Cross-game tag overviews

Curated summary panels for tags that are otherwise tedious as raw field dumps,
resolving the layout differences across kits:

- **material_effects**, **dialogue**, and **sound_classes** overview tables, with
  clickable references that jump to the related tags.

### Custom color palettes

Save colours picked in the color editor and build reusable Baboon palettes that
can be loaded back in any tag — handy for keeping shader/material colours
consistent.

### Export & extraction

All extraction runs on background threads and reports progress to the status bar:

- **JSON dump** — a single tag or an entire folder subtree to pretty-printed
  JSON, preserving the full field hierarchy (blocks, arrays, structs, enums,
  flags, references, resources).
- **Raw tag extraction** — write a tag (e.g. one pulled from a monolithic cache)
  back out as a standalone loose tag file.
- **Bitmap extraction** — every image in a bitmap tag to **TIFF**, individually
  or in bulk across a folder.
- **Geometry extraction** — to **JMS** / **ASS**:
  - `model` (`hlmt`) — resolves and extracts the referenced render, collision,
    and physics models, sharing the render skeleton across them, into
    `render/`, `collision/`, and `physics/` subfolders.
  - `render_model` (`mode`), `collision_model` (`coll`), `physics_model`
    (`phmo`) — direct JMS extraction.
  - `scenario_structure_bsp` (`sbsp`) — ASS extraction.
  - `scenario` (`scnr`) — per-BSP geometry extraction (ASS for Halo 2/3,
    render + collision JMS for Halo CE).
- **Import info** and **animation extraction**, all run in-process via the
  `blam-tags` library.

### Halo: Campaign Evolved mods

Campaign Evolved's tags live inside UE5 IoStore paks. Baboon edits them **in
memory** and offers two ways to commit changes — overwrite the game directly, or
package your changes as a separate, reversible mod:

- **Save** (Ctrl+S) — **overwrites the tag inside the game's own pak, in place.**
  The edited chunk is appended to the container's `.ucas` and its `.utoc` is
  rewritten to point at it (preserving the container's perfect-hash seeds), with
  the paired `.uasset`'s bulk-data size patched when an edit changes the tag's
  byte length. This modifies the shipped game files, so Baboon **always confirms
  first** — the dialog offers *Export Mod* as the non-destructive alternative, and
  has a *Don't ask again* option (also under **Settings → Startup → Saving**).
  There is no undo without a backup of the paks. *(Loose-folder MCC tags save
  normally and never prompt.)*
- **Export Mod…** (File menu) — bundle **every modified tag in the active
  project**, including checkpointed tags whose tabs were closed, into one
  portable, higher-priority **overlay container** without touching the base game.
  Mods are fully reversible (delete the overlay to uninstall).
- **Save As** — *duplicate* the tag under a new name into an overlay container (a
  new UE package with its own identity, `.uasset`, and container-header entry).
- **Rename** (right-click) — the same as Save As, plus a package **redirect** so
  existing tags that reference the old name resolve to the renamed one.
- **Duplicate / Delete** (right-click) — add a copy to, or retire one from, the
  container itself rather than an overlay. See *Duplicating & deleting tags*.

Any in-place write to a container **drops that pak's perfect-hash lookup table**.
The table maps a chunk id to a *slot* in the chunk-id array, so both the chunk
count and every chunk's position are part of it — adding an entry changes the
modulo base for every chunk in the container, and there is no way to regenerate
the table. Dropping it makes the runtime index chunk ids directly instead, which
is exactly how the overlay containers Baboon exports are already laid out. The
cost is a slightly slower mount for that one pak; the alternative is a container
whose lookups silently stop resolving.

Export Mod produces a `<name>-WinGDK_P` IoStore **triplet** — `.utoc`, `.ucas`,
and a small `.pak` stub — plus a same-stem `.baboon` project file containing
the open-tab layout and recoverable copies of every modified/new tag. Use
**File → Open Baboon Project…** to continue editing that project on this or
another machine. Save As / Rename produce the IoStore triplet without a project
sidecar. Drop **all three runtime files**
files into the game's `Meteorite/Content/Paks/` folder alongside the base paks.
The `.pak` is required: UE's loader discovers containers by scanning that folder
for `.pak` files and derives the matching `.utoc`/`.ucas` from each — an overlay
with no `.pak` is never mounted. The `_P` suffix then gives the overlay patch
priority so UE serves your chunks on top of the base (last-mounted-wins).

While a Campaign Evolved source is open, Baboon checkpoints the active project
to `%APPDATA%\Baboon\campaign_evolved_recovery.baboon` after a short idle
period. Clean tabs reload from the game containers; modified and newly-created
tags are stored in the project database. The normal Ask / Always / Never session
setting controls whether this recovery project is reopened on the next launch.

Tag identity (`FPackageId` / export hashes), UE5 Zen-package `.uasset`
(de)serialization, and override-container writing are all implemented natively in
`blam-tags` with no external UE tooling — references resolve by the same
`CityHash64` package-path hashing the game itself uses. Output containers are
validated structurally against an independent packer; loading them in the
shipping game is the one step that requires a Windows/Xbox run to confirm.

### Geometry import & integrated terminal

- An **Import** button on geometry/animation references runs the matching `tool`
  verb (`render` / `collision` / `physics` / `model-animations-uncompressed`)
  against the source asset.
- An integrated **terminal panel** runs commands in the editing-kit root with
  live streamed output. Its open/closed state is remembered **per editing kit**.

### Tool launchers & command runner

Toolbar buttons launch the loaded kit's tools, with the executable auto-detected
per game:

- **Sapien** (`sapien.exe`).
- **tag_test** — the game-specific build (`halo_tag_test.exe`,
  `halo2_tag_test.exe`, `halo3_tag_test.exe`, `atlas_tag_test.exe`,
  `reach_tag_test.exe`, `halo4_tag_test.exe`, or the generic `tag_test.exe`).
- **Blender** — at a user-configured path (set in *File → Settings*).

Launchers are disabled until the relevant executable is found in the kit.

When a loose `.scenario` tag is open, its editor header also provides Sapien and
tag_test buttons. For supported kits, Baboon saves pending edits and passes the
absolute scenario file directly to Sapien; Halo CE is excluded. The tag_test
launcher updates the kit's `init.txt` while preserving unrelated commands. The
quick-access launchers remain generic, and quick-access tag_test removes stale
active scenario-launch commands from `init.txt`.

A **Run Tool Command** window lists each game's `tool` commands (from per-game
JSON), with a form for their parameters — enum dropdowns, file/path pickers, and
**inline validation** that flags empty required parameters before running. The
assembled command runs in the integrated terminal.

### Preferences

Browser mode, prefix display, expert mode, dark/light theme, the Blender path,
custom editing-kit folder names, recent folders, keyword and palette sidecars,
and per-kit terminal state are persisted to `%APPDATA%\Baboon` and restored on
launch.

On launch Baboon can reopen the windows from your previous session. **Every
workspace** is restored, each with its own tags and its own project, and the
reopen prompt lists them grouped by game. The startup behaviour is a three-way
choice in *File → Settings* — **Ask** which windows to reopen, **Always** reopen
automatically, or **Never** — and the prompt itself carries a *Don't ask again*
option (OK remembers *Always*, Cancel remembers *Never*).

The main native window's last normal bounds and its distinct normal, maximized,
or fullscreen mode are stored separately in a versioned `window-state.json`:

- Windows: `%APPDATA%\Baboon\config\window-state.json`
- Linux: `$XDG_CONFIG_HOME/baboon/window-state.json` (or
  `~/.config/baboon/window-state.json`)
- macOS: `~/Library/Application Support/Baboon/window-state.json`

Baboon restores this state before creating the first visible viewport. Saved
bounds are checked against the connected displays and their work areas, then
clamped or centered on the primary display if necessary. Portable mode does not
redirect this machine-specific file. On Wayland, portable APIs generally do
not expose absolute window positions or desktop work areas; Baboon still
restores size and mode, uses full monitor bounds for validation, and leaves
placement to the compositor when coordinates are unavailable.

---

## Technical overview

- **Language / edition** — Rust 2024.
- **UI** — [`eframe`/`egui`](https://github.com/emilk/egui) (immediate-mode GUI) with the `glow` (OpenGL)
  backend and bundled default fonts. Native file dialogs via [`rfd`](https://github.com/PolyMeilex/rfd).
- **Engine** — the [`blam-tags`](https://github.com/camden-smallwood/blam-tags) crate, pulled as a pinned Cargo git dependency,
  provides all binary tag parsing/serialisation, bitmap decoding, geometry
  export (JMS/ASS), render-method handling, sound-tag audio decoding (all games,
  via its `audio` feature), the monolithic cache reader, and — via its `iostore`
  feature — UE5 container reading, Zen-package (de)serialisation, and the
  in-place container edits behind Campaign Evolved duplicate/delete and Chimp.
  It is currently pinned to the `codex/chimp-backend` branch of the
  [`Zoephie/blam-tags`](https://github.com/Zoephie/blam-tags) fork, which carries
  those container-writing changes ahead of upstream.
- **Concurrency** — all file I/O (loading, scanning, indexing, export) runs on
  worker threads that communicate with the UI via an `mpsc` channel and request
  repaints; the UI thread never blocks on disk.
- **Caching & performance** — lazy folder-tree expansion, a memoised search-match
  tree keyed on a source generation counter, an LRU parsed-tag cache, and a
  persisted per-game entry index on disk.
- **Platform** — primarily Windows (release builds run as a windowed app with no
  console; the app icon is embedded as a Win32 resource via `build.rs`).
  *Open in File Explorer* and the bundled tool launchers are Windows-specific;
  the core editor is platform-neutral.
- **Dependencies** — `eframe`, `egui_extras` (SVG tag icons), `image`
  (icon/bitmap handling), `flate2`, `rfd` (dialogs), `rodio` (audio output),
  `walkdir` (folder scanning), `serde_json` (JSON dump & index/prefs), `anyhow`.

---

## Building

Clone the repo with submodules (required for the tag definitions):

```
git clone --recurse-submodules https://github.com/Zoephie/Baboon.git
cd Baboon
```

Or, after a normal clone:

```
git submodule update --init --recursive
```

Then build:

```
cargo build --release
```

`blam-tags` is fetched automatically by Cargo — you do not need to clone it
separately. The `definitions/` git submodule is required; initialise it with
`git submodule update --init` after cloning. The build script copies that
submodule folder next to the built executable under `target/<profile>/definitions`.

Geometry, animation, and import-info extraction all run in-process via the
`blam-tags` library — Baboon no longer shells out to a companion binary.
Ship `Baboon.exe` and the `definitions/` folder in releases.

---

## Usage

Use the **File** menu to open a single tag, a loose tags folder (e.g. an MCC
editing-kit `tags/` directory), or a Halo 4 monolithic cache (`blob_index.dat`).
Browse or search in the left panel, click a tag to open it in a tab, and edit
inline. Save loose little-endian tags back to disk from the editor. The toolbar
buttons launch the kit's Sapien / tag_test and Blender.

Tags can also be opened from a terminal by passing an editing-kit flag followed
by one or more tag paths:

```text
Baboon.exe -HREK objects/weapons/assault_rifle/assault_rifle.weapon objects/vehicles/warthog/warthog.vehicle
```

Supported flags are `-HCEEK`/`-H1EK`, `-H2EK`, `-H3EK`, `-H3ODSTEK`, `-HREK`,
`-H4EK`, and `-H2AMPEK`/`-H2AEK`; flags are case-insensitive. Relative paths
are resolved beneath the configured editing kit's `tags` folder. Absolute paths
are accepted only when they point inside that same folder. Quote any path that
contains spaces. A command-line launch ignores the previous session and opens
only the requested tags.
