# Browser selection and bulk operations

Dedicated browser tabs and windows open in **Flat** folder view by default.
Flat view lists the current directory's immediate folders and tags; searching
filters that list rather than finding descendants. Double-click a folder to
enter it. **View > Tree** and **View > Groups** retain hierarchical browsing
with single selection. Each browser saves its own folder-view mode in sessions.
Folder tab menus offer Reveal in Browser, which reopens the left navigation
and reveals the browser's current folder, plus the platform file-manager action
for loose folders. Tab menu commands use title case and shared icons.
The left navigation stays single-select, and only folder chevrons expand or
collapse its tree by mouse. Double-clicking a folder label opens its browser.

## Selection

- Ctrl/Cmd-click toggles an item; Shift-click selects a range in displayed order.
- Flat view accepts mixed folders and tags. Asset Browser accepts multiple tags.
- Ctrl/Cmd+A selects the visible listing in these dedicated views, never the
  entire left navigation tree. Selected folders include all their contents for
  file operations, including non-tag files and empty directories.
- Right-click keeps an existing selected group or selects the clicked item if
  it was outside the group. The menu applies to that selection.
- Each pane keeps an independent selection. Moving keyboard focus elsewhere
  preserves it with a muted appearance. Blank-space clicks and Escape clear
  selection; navigating or changing the search/view/filter clears it too.
  Sorting preserves the selected items. Each browser container's footer reports
  its own item total in faded text and selected roots. Empty selection categories
  are omitted; its About button opens the Folder Browser help section.
- Typing selects a matching leaf name without filtering or opening it. A short
  pause resets the prefix; repeating a letter cycles matching names with wrap.
  Matches scroll into view. Text editors retain their normal keyboard input.

## Actions

Open uses the selected tags themselves. A single folder enters the current
browser; opening multiple folders creates or reuses their browser tabs, without
opening every descendant tag. Opening ten or more new tabs needs confirmation;
already-open tabs do not count toward the warning.

Writable loose-file kits support Rename, Move To, Copy To and Delete. Asset Browser
omits Move To and Copy To because its selection may span folders. Unsupported
file operations are disabled. Compatible exports show their selected tag counts
and reuse the existing bitmap, sound, shader, include, JSON and raw container-tag exporters.
Geometry, animation and other single-tag exporters keep their existing menus.

Move and Copy choose one destination parent inside the loaded tags root. All
selected items share one reference-index preflight and one conflict review.
Matching folders merge, and Replace, Skip and Keep Both can apply to remaining
conflicts. Incoming roots with duplicate destination names are rejected before
writing. A same-folder Move is a no-op; Copy assigns an available numbered name.

Unsaved workspace tags require Save All and Continue or cancellation. Decisions
are collected before applying; ordinary file changes during review trigger a
fresh review. Cancellation is available during preflight/review. Applying locks
editing and attempts to roll back the entire selection if a write or reference
rewrite fails. Replaced or deleted files share one recovery batch with their
original tags-root-relative paths. Empty deleted folders are preserved too.
Delete review shows tag/non-tag counts and references from outside the selection.

Move completion remaps open tag paths, folder browsers, favorites and references,
and refreshes documents and the browser. Completed roots leave the selection;
skipped roots remain selected. Copy leaves the source selection intact.

## Implementation boundaries

`browser/selection.rs` owns ordered selection and menu eligibility.
`browser/flat.rs` owns cached immediate listings and virtualized rows.
`browser/navigation.rs` owns keyboard ordering and type-to-select.
`browser/bulk.rs` coordinates prompts, whole-selection jobs and completion.
`core/folder_merge.rs` and `tag_ops/folder_merge_ops.rs` share the filesystem
plan, recovery and rollback journal between existing single-folder and new
multi-root operations. Recovery batches also record `original_relative_paths`.

Tests use isolated temporary editing kits and headless egui frames; they never
move files in a user's editing kit.

Bulk Rename uses a preview dialog with current-name/number tokens and literal
find/replace, preserving extensions and parents. It runs a collision-strict
variant of the shared multi-root refactor: no merge, Replace or Keep Both path
can change the reviewed names. Collisions with existing selected names must be
resolved in separate batches. Completion remaps the owning pane's selection.
Type-to-select only changes the viewport when the target is outside it, avoiding
a list jump for already-visible matches. Flat folder/tag selections use the
same border-only style.

Single-folder and multi-selection **Extract** menus share the same options and
counts, including unopened subfolders. If the complete tag list is not ready,
choose **Load Folder**; its hover hint explains what is loaded. While loading,
the action is disabled. Once ready, folders with no supported exports show
**No Extractable Tags**. Loading does not open editor tabs.
