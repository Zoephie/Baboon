# Baboon

**Baboon** is a native desktop viewer and editor for Halo tag files, built in
Rust with [`eframe`/`egui`](https://github.com/emilk/egui). It links the [`blam-tags`](https://github.com/camden-smallwood/blam-tags) engine directly for
byte-exact tag reading, editing, and asset extraction, and presents a
Guerilla-style editing surface for working with the loose tag files shipped in
the **Halo: The Master Chief Collection** editing kits — no round-trip through
the official tools required.

Open a single tag, an entire editing-kit `tags/` folder, or a monolithic tag
cache; browse and search the tag tree (by name *or* field value); edit fields,
blocks, shaders, and functions inline with full undo/redo; preview bitmaps and
3D models; trace references and diff tags; and extract geometry, textures, and
animations — all from one application.

> Baboon is the GUI front end for the `blam-tags` project. The library does the
> binary tag parsing; Baboon is the interactive editor built on top of it.

---

## Supported games

Baboon recognises and auto-configures itself for the following MCC editing
kits, detected from the kit's root folder name:

| Editing kit              | Folder              | Game identifier |
| ------------------------ | ------------------- | --------------- |
| Halo CE                  | `HCEEK` / `H1EK`    | `haloce_mcc`    |
| Halo 2                   | `H2EK`              | `halo2_mcc`     |
| Halo 3                   | `H3EK`              | `halo3_mcc`     |
| Halo 3: ODST             | `H3ODSTEK`          | `halo3odst_mcc` |
| Halo: Reach              | `HREK`              | `haloreach_mcc` |
| Halo 4                   | `H4EK`              | `halo4_mcc`     |
| Halo 2: Anniversary (MP) | `H2AMPEK` / `H2AEK` | `halo2amp_mcc`  |

The game is also detected from a folder literally named after the game id (e.g.
`halo3_mcc`), and **custom editing-kit folder names** can be mapped to a game in
*File → Settings* for non-standard layouts.

Per-game group-name tables and schemas are loaded from
`definitions/<game>/*.json`. Release builds place the `definitions/` folder next
to `Baboon.exe`, which keeps the schemas inspectable and editable without
rebuilding the app.

---

## Features

### Loading tag sources

Baboon can open three kinds of source, each on a background thread so the UI
never blocks:

- **Single tag** — open any individual loose tag file.
- **Loose tags folder** — point at an MCC editing-kit `tags/` directory (or the
  kit root, e.g. `H3EK`; Baboon locates the `tags` folder and identifies the
  game automatically). The folder tree is loaded **lazily**, expanding
  directories only as you open them, so even a full kit opens instantly.
- **Monolithic cache** — open a Halo 4 `blob_index.dat` monolithic tag cache and
  browse its contents as if they were loose files (read-only).

Tag files are identified by probing their 64-byte header for the `BLAM`
(big-endian) / `MALB` (little-endian) magic, so non-tag files in the tree are
silently skipped.

### Tag browser

- **Folder view** — the on-disk directory hierarchy, with a **per-group icon**
  beside each tag (and on its editor tab) for quick visual scanning.
- **Groups view** — tags regrouped by tag group (e.g. *biped*, *weapon*,
  *render_model*), with friendly names resolved from the definition tables.
- **Recent folders** — a quick-open list of recently opened tag folders.
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
  automatic reference fix-up across every referencing tag), and *Open in File
  Explorer*.

### Search, navigation & cross-referencing

- **Field-value search** — search across tags' *field values* (not just names),
  run on a background worker against an in-memory field index and optionally
  scoped to a tag group; results open in a clickable window.
- **Find references** — list every tag that references the current tag.
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

### Tabbed, dockable editor

- Open multiple tags as **tabs** in a rack, each with its group icon and an
  amber tint + ● marker when it has unsaved edits.
- **Tear off** any tab into a floating, resizable window — and drag it back onto
  the rack (or click *dock*) to re-dock it.
- An LRU cache bounds how many parsed tags and tabs are kept in memory at once,
  trimming least-recently-used documents automatically.

### Field editing

The editor renders the full tag structure — nested blocks, arrays, structs, and
pageable resources — with inline editing for loose little-endian tags:

- Scalars, integers, reals, strings, and `string_id`s.
- **Enums** and **bit flags** with named options.
- **Colors** via an interactive color-picker popup with channel parsing.
- **Tag references** with an *Open* button (Alt-click opens in a floating window)
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
- **Guerilla-style "Search fields"** — type a block or field name to collapse the
  editor down to just the matching node(s) and their ancestors.
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
  - `scenario` (`scnr`) — geometry extraction via the shell.
- **Import info** and **animation extraction** via the companion
  `blam-tag-shell` binary.

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

A **Run Tool Command** window lists each game's `tool` commands (from per-game
JSON), with a form for their parameters — enum dropdowns, file/path pickers, and
**inline validation** that flags empty required parameters before running. The
assembled command runs in the integrated terminal.

### Preferences

Browser mode, prefix display, expert mode, dark/light theme, the Blender path,
custom editing-kit folder names, recent folders, keyword and palette sidecars,
and per-kit terminal state are persisted to `%APPDATA%\Baboon` and restored on
launch.

---

## Technical overview

- **Language / edition** — Rust 2024.
- **UI** — [`eframe`/`egui`](https://github.com/emilk/egui) (immediate-mode GUI) with the `glow` (OpenGL)
  backend and bundled default fonts. Native file dialogs via [`rfd`](https://github.com/PolyMeilex/rfd).
- **Engine** — the [`blam-tags`](https://github.com/camden-smallwood/blam-tags) crate, pulled as a pinned Cargo git dependency,
  provides all binary tag parsing/serialisation, bitmap decoding, geometry
  export (JMS/ASS), render-method handling, sound-tag audio decoding (all games,
  via its `audio` feature), and the monolithic cache reader.
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

Geometry/animation/import-info extraction additionally relies on the companion
`blam-tag-shell` binary. The root workspace builds Baboon and `blam-tag-shell`
together, placing them side by side in `target/debug/` or `target/release/`.
Ship `Baboon.exe`, `blam-tag-shell.exe`, and the `definitions/` folder in
releases.

---

## Usage

Use the **File** menu to open a single tag, a loose tags folder (e.g. an MCC
editing-kit `tags/` directory), or a Halo 4 monolithic cache (`blob_index.dat`).
Browse or search in the left panel, click a tag to open it in a tab, and edit
inline. Save loose little-endian tags back to disk from the editor. The toolbar
buttons launch the kit's Sapien / tag_test and Blender.
