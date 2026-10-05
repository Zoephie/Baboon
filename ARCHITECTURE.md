# Baboon's architecture

This is a map of the code: what lives where, how a frame runs, and the few
patterns everything follows. It describes structure, not features. For what
Baboon does, see the [README](README.md).

## The crates

Baboon is the egui front end of the [`blam-tags`](https://github.com/camden-smallwood/blam-tags)
engine. The split is strict:

- **`blam-tags`** reads, writes, converts and extracts tags, and knows the
  games' formats: tag layouts, caches, Campaign Evolved's IoStore containers and
  Unreal packages. It has no UI.
- **Baboon** is the editor built on it. It is one package with three parts:
  - `src/core` — the application's model without any UI: games, tag sources
    and their catalogs, documents and their edit journal, value formatting,
    storage locations, bundled files, helper processes. `core` may not use
    egui, eframe or anything in `src/app`; the tests in `core/mod.rs` fail the
    build if it does.
  - `src/app` — the egui application.
  - `src/bin`: the build tools that generate `docs/script_docs.sqlite3` and
    `docs/tag_compat.sqlite3`, each a folder holding all of its own code.

During development the engine is used from a sibling checkout through a
`[patch]` in `Cargo.toml`; a release pins it by revision.

## The application

[`Baboon`](src/app/mod.rs) is the eframe app. It holds:

- **`model: Model`** — the state every feature reads: the open kits (each a
  workspace: its tag source, open documents, project), which one is active,
  the live preferences, and the status line.
- **`views: KitViews`** — each kit's view state, keyed by its `KitId`: tile
  trees, browser state, edit buffers, caches, Chimp's panes. Kept apart from the
  model so a draw can change its kit's view while it reads the model.
- **features** — one `…Feature` struct per feature (`EditorFeature`,
  `SearchFeature`, `ChimpFeature`, …), holding that feature's own app-wide
  state: jobs in flight, caches, engines such as Find's.
- **`dialogs: DialogHost`** — every window open over the workspaces.
- **`commands: CommandQueue`** — what this frame's draws have asked for.
- the worker channel that background jobs answer on.

## A frame

eframe calls `App::logic` and then `App::ui`
([`shell/frame`](src/app/shell/frame.rs)):

1. **logic** (`run_logic`) needs no UI and also runs while the window is
   hidden: it applies finished background work (`WorkerMessage`s), runs timers
   such as autosave and recovery checkpoints, and handles a request to close.
2. **ui** (`draw_root_ui`, [`shell/workspace`](src/app/shell/workspace.rs)):
   keyboard shortcuts, the menu and status bars, the kit and tag tiles, then
   the dialogs. Last, it applies every command the draws queued, in the order
   they were sent.

While the first-run wizard is open it draws alone.

## Drawing: `Ctx` and commands

A draw is given a [`Ctx`](src/app/context.rs) — the model read-only, the
egui context, the command queue and the job channel — and holds its own
feature's state mutably:

```rust
draw_poke_window(&cx!(self, ctx), &mut self.poke);
```

What a draw wants done to anything it does not own, it **sends as a command**:
`cx.send(SearchCommand::FindStep(1))`. Each feature has its own command enum
and an `apply_…_command` on `Baboon`; `Command` collects them. Because commands
run after drawing, a draw never changes the model under a sibling drawn after
it, and what a click does is a value a test can read without a window.

Some rules that follow from this:

- A draw that needs another feature's state reads it from what it is passed,
  never by reaching into `Baboon`.
- Text being typed is a draft owned by the window that shows it; the window
  sends the finished value. Settings and the first-run wizard edit a copy of
  the preferences and send it once drawn, before anything else they asked for.
- A draw that must fill a cache first gets it from a step that runs before
  drawing. The kit tiles are the main case: `prepare_tiles` fills every cache
  their panes read, then the tiles draw from `TileParts`, the features they
  draw into borrowed field by field.

## Dialogs

Every window over the workspaces is a [`Dialog`](src/app/dialogs.rs) in the
`DialogHost`:

```rust
trait Dialog: Any {
    fn show(&mut self, cx: &Ctx, app: &AppReads) -> bool; // stays open?
    fn instance(&self) -> u64 { 0 }
}
```

- A dialog **owns its state**: its draft, its choices, what it was opened on
  (including the kit it acts on).
- It is drawn with the `Ctx` and **`AppReads`**: other features' state, lent
  read-only for the frame. It changes nothing but itself; everything else is a
  command.
- Handlers find an open dialog by type: `dialogs.get::<T>()`,
  `get_mut::<T>()`, `close::<T>()`.
- An action that can be refused **leaves its dialog open**, and the handler
  takes it back with `close`, acts, and opens it again with the reason if it
  failed. New Tag, the save-changes prompt and the editing-kit editor work this
  way.
- A draw opens a dialog with `cx.open_dialog(…)`; a handler with
  `dialogs.open(…)`. Opening one of the same type and instance replaces it.

The smoke test in `shell/frame.rs` draws every window over a populated app
and fails for any `impl Dialog` without a case.

## Documents and undo

An open tag is a `TagDocument` (in `core/document`): the parsed `TagFile`, its
dirty state, and an `EditJournal`. The journal stores whole snapshots, the
bytes that restore a document (`JournalDocument`), taken once before each run
of edits; a frame that draws a pane without editing closes the run, so a drag
is one undo step. Chimp's packages use the same journal with rebuilt package
bytes as their snapshots.

## Background work

Long work runs off the UI thread through `spawn_worker` / `Ctx::spawn`
([`shell/worker`](src/app/shell/worker.rs)): the job returns a
`WorkerMessage`, and if it panics a message is built from the panic instead,
so nothing the UI marked as in flight is left that way. `run_logic` applies the
messages each frame.

## Kits and their views

A kit is a workspace: one tag source (a loose editing kit, a single tag, a
monolithic cache, or a Campaign Evolved install), its documents and its
project. `Kit` (in `Model`) is its content; `KitView` (in `KitViews`) is how it
is being looked at. Both are keyed by `KitId`, which outlives positions in the
kit list. Anything that acts on a kit names its `KitId`, so a command answered
after the user switched workspace still lands in the right one.

## Features

`src/app` is a folder per feature, each with a `mod.rs` whose top comment says
what it owns and what it leaves to others. A module that is one file with its
tests is `foo.rs`, not a folder:

| Folder | What it is |
| --- | --- |
| `browser` | The tag browser: folder and group trees, docked folder panes (column table, asset grid), search, menus, thumbnail libraries |
| `editor` | The tag pane, the generic field editor, the panels for particular groups, and their popups |
| `documents` | Opening, saving, undo and redo, and closing with the save-changes prompt |
| `search` | Find, the field-value index and search, and source listings |
| `references` | The reverse-dependency index, the Content Explorer, reference jumps |
| `compare` | Git Review, Compare Tags, and the tag diff they share |
| `tag_ops` | New Tag, renaming and moving tags and folders, duplicating, deleting |
| `import` | Import Tags across games, single-tag import, cache import, Blam! assets |
| `export` | Extracting raw tags, bitmaps, sounds, geometry, animations, sources |
| `mods` | Campaign Evolved projects, container writes, Export Mod |
| `kits` | Loading and indexing sources, kit profiles, the kit's tools and terminal |
| `chimp` | Campaign Evolved's Unreal package workspace |
| `runtime_poke` | Campaign Evolved runtime poking |
| `help` | The documentation window, tutorials, script docs, tag compatibility |
| `shell` | The frame, menus and bars, kit tiles, settings, first run, sessions, updates |

Shared services sit beside them: `ui_kit` (theme, icons, widgets — nothing
about tags), `prefs`, `model_preview`, `audio`, plus `model`, `context` and
`dialogs`.

**Imports.** A file in a feature imports its own feature with `use super::*`.
What a feature uses from another, it names in its root with explicit paths —
`use crate::app::browser::{BrowserMode, TagQueryResults};` — so a feature's
`mod.rs` lists what it depends on. `app/mod.rs` does not glob-import features;
it globs only the shared services (`ui_kit`, `prefs`, `model_preview`) and
core's document and keyword types, which every feature sees.

## Tests

Tests sit beside the code they test, in the module whose code they exercise:
one `#[cfg(test)] mod tests { … }` at the bottom of the file, or a `tests.rs`
beside `mod.rs` when the module is a folder with other children. A test that
needs no UI belongs in `core`. Fixtures several modules share are
`#[cfg(test)]` items beside what they set up, such as
`editor::fields::with_test_edit_context`, or live in `src/core/test_kits.rs`.

Most tests build what they need synthetically from the bundled definitions.
Tests that need a real editing kit read its location from the environment
through `src/core/test_kits.rs` (`BLAM_TEST_HCEEK`, `BLAM_TEST_H2EK`,
`BLAM_TEST_H3EK`, `BLAM_TEST_HREK`) and skip, by name, when it is not set;
Campaign Evolved ones read `BLAM_TEST_CE` through `test_kits`, or `CE_PAKS`
directly, and are `#[ignore]`d otherwise. No tag files are checked in.

Whole-frame tests drive `Baboon::run_frame` headlessly through the `Harness`
in `shell/frame.rs`'s tests.

## Adding things

- **A window**: a struct holding its state, `impl Dialog` for it, open it from
  a handler or `cx.open_dialog`, and add a smoke case.
- **Something a draw asks for**: a variant on the feature's command enum and an
  arm in its `apply_…_command`.
- **Background work**: `Ctx::spawn` with a `WorkerMessage` variant and its
  handler.
- **A feature**: a folder under `src/app` with its `…Feature` state on
  `Baboon`, a command enum, and its imports from other features named in its
  root.
