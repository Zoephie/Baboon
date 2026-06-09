# Baboon

**Baboon** is a native desktop viewer and editor for Halo tag files, built in
Rust with [`eframe`/`egui`]. It links the [`blam-tags`] engine directly for
byte-exact tag reading, editing, and asset extraction, and presents a
Guerilla-style editing surface for working with the loose tag files shipped in
the **Halo: The Master Chief Collection** editing kits — no round-trip through
the official tools required.

Open a single tag, an entire editing-kit `tags/` folder, or a monolithic tag
cache; browse and search the tag tree; edit fields, blocks, shaders, and
functions inline; preview bitmaps; and extract geometry, textures, and
animations — all from one application.

> Baboon is the GUI front end for the `blam-tags` project. The library does the
> binary tag parsing; Baboon is the interactive editor built on top of it.

---

## Supported games

Baboon recognises and auto-configures itself for the following MCC editing
kits, detected from the kit's root folder name:

| Editing kit | Folder | Game identifier |
|-------------|--------|-----------------|
| Halo 3 | `H3EK` | `halo3_mcc` |
| Halo 3: ODST | `H3ODSTEK` | `halo3odst_mcc` |
| Halo: Reach | `HREK` | `haloreach_mcc` |
| Halo 4 | `H4EK` | `halo4_mcc` |
| Halo 2: Anniversary (MP) | `H2AMPEK` / `H2AEK` | `halo2amp_mcc` |

Per-game group-name tables (`blam-tags/definitions/<game>/_meta.json`) are
provided by the submodule and embedded into the binary at compile time, so
friendly tag-group names and reference resolution work regardless of where the
executable is launched from.

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

- **Folder view** — the on-disk directory hierarchy.
- **Groups view** — tags regrouped by tag group (e.g. *biped*, *weapon*,
  *render_model*), with friendly names resolved from the definition tables.
- **Search/filter** — a fast, memoised substring filter that prunes the tree to
  matching tags and their ancestors. Results are cached and recomputed only when
  the query, source, or mode actually changes — not per frame — so it stays
  responsive across 100k+ entry kits.
- **Background indexing** — a full recursive scan runs in the background to power
  Groups view and global search without expanding every node first. The
  completed index is **persisted** (per game, to `%APPDATA%\Baboon`) so
  subsequent launches skip the scan entirely.
- **Context actions** — per-tag and per-folder right-click actions for JSON dump,
  raw extraction, bitmap/geometry/animation extraction, and *Open in File
  Explorer*.

### Tabbed, dockable editor

- Open multiple tags as **tabs** in a rack.
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
- **Tag references** with an *Open* button that resolves and opens the referenced
  tag in a new tab — even if it isn't in the current index — plus an *Import*
  button on geometry references that re-imports the source asset via `tool`.
- **Guerilla-style "Search fields"** — type a block or field name to collapse the
  editor down to just the matching node(s) and their ancestors.
- **Expert mode** toggle to reveal advanced/normally-hidden fields.
- Monolithic-cache and big-endian tags are opened **read-only**; only
  little-endian loose tags can be saved back to disk.

### Block editing

Full structural editing of tag blocks, applied safely after each frame's render
pass:

- **Add**, **insert**, **duplicate**, and **delete** elements, plus **delete
  all** (with a confirmation modal for destructive ops).
- **Copy / paste** elements — including the entire block — between two open tags
  of the same group, with compatibility re-validated by the library before
  insertion.
- **Replace** a selected element or an entire block from the clipboard.

### Shader & material editor

For `shader`, `material`, and `material_shader` tags, Baboon builds a
**Guerilla-style shader grid** instead of a raw field dump:

- Resolves the tag's render-method definition (`rmdf`) and options (`rmop`),
  caching them across tags.
- Shows bitmap, scalar, integer, color, and category parameters with their
  defaults, all editable inline.
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
- Per-channel **R / G / B / A** toggles, including alpha-only inspection.
- **Zoom-to-cursor** and **drag-to-pan**, with fit-on-open.
- Reports format, type, dimensions, and image count for multi-image bitmaps.

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

### Tool launchers

Toolbar buttons launch the loaded kit's tools, with the executable auto-detected
per game:

- **Sapien** (`sapien.exe`).
- **tag_test** — the game-specific build (`halo3_tag_test.exe`,
  `atlas_tag_test.exe`, `reach_tag_test.exe`, `halo4_tag_test.exe`, or the
  generic `tag_test.exe`).
- **Blender** — at a user-configured path (set in *File → Settings*).

Launchers are disabled until the relevant executable is found in the kit.

### Preferences

Browser mode, prefix display, expert mode, dark/light theme, the Blender path,
and per-kit terminal state are persisted to `%APPDATA%\Baboon` and restored on
launch.

---

## Technical overview

- **Language / edition** — Rust 2024.
- **UI** — [`eframe`/`egui`] (immediate-mode GUI) with the `glow` (OpenGL)
  backend and bundled default fonts. Native file dialogs via [`rfd`].
- **Engine** — the [`blam-tags`] crate, linked by path, provides all binary tag
  parsing/serialisation, bitmap decoding, geometry export (JMS/ASS), render-
  method handling, and the monolithic cache reader.
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
- **Dependencies** — `eframe`, `image` (icon/bitmap handling), `rfd` (dialogs),
  `walkdir` (folder scanning), `serde_json` (JSON dump & index/prefs), `anyhow`.

---

## Building

This crate depends on the `blam-tags` engine crate through a Git submodule
pinned to the `back` branch of `https://github.com/Zoephie/blam-tags.git`.

```
Baboon/
├── blam-tags/         <- Git submodule
│   └── blam-tags/     <- the library crate
└── src/
```

Clone with submodules:

```sh
git clone --recurse-submodules <Baboon-repo-url>
cd Baboon
```

Or, after a normal clone:

```sh
git submodule update --init --recursive
```

Then build:

```sh
cargo build --release
```

The per-game definition tables under `blam-tags/definitions/` are embedded at
compile time, so the binary resolves group names and references no matter where
it is launched from. Geometry/animation/import-info extraction additionally
relies on the companion `blam-tag-shell` binary. The root workspace builds
Baboon and `blam-tag-shell` together, placing them side by side in
`target/debug/` or `target/release/`. Ship both `Baboon.exe` and
`blam-tag-shell.exe` in releases.

## Usage

Use the **File** menu to open a single tag, a loose tags folder (e.g. an MCC
editing-kit `tags/` directory), or a Halo 4 monolithic cache (`blob_index.dat`).
Browse or search in the left panel, click a tag to open it in a tab, and edit
inline. Save loose little-endian tags back to disk from the editor. The toolbar
buttons launch the kit's Sapien / tag_test and Blender.

[`eframe`/`egui`]: https://github.com/emilk/egui
[`rfd`]: https://github.com/PolyMeilex/rfd
[`blam-tags`]: ../blam-tags
