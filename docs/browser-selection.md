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

After you pick its destination, a bulk Move opens the Move Selected Items? review
at once, styled like the delete windows. The destination folder is shown on a band
under the title. The selected items ("N Items to Move") sit beside the tags whose
references the move fixes ("N References to Fix"), which fill in when the
reference check finishes. Mission scripts that name the moved tags are listed
after the tags, one row per script, and are fixed with them (see Renaming,
Scripts). A line says whether any item has the same name as
something already in the folder; those are still decided one by one afterwards.
Move stays unavailable until the check finishes. Moving a single item narrows
the window to the single Delete windows' layout: the item is an icon row above
the References to Fix box rather than a box of its own. Copy keeps its summary
review, since copying fixes no references.

Unsaved workspace tags require Save All and Continue or cancellation. Decisions
are collected before applying; ordinary file changes during review trigger a
fresh review. Cancellation is available during preflight/review. Applying locks
editing and attempts to roll back the entire selection if a write or reference
rewrite fails. Replaced or deleted files share one recovery batch with their
original tags-root-relative paths. Empty deleted folders are preserved too.
Every delete window shares Rename's style. Deleting one tag or folder shows it as
an icon row with its folder muted. The bulk Delete Selected Items? review says how
many tags and other files the selection holds, then shows the selected items
("N Items to Delete") beside the tags that will have missing references. All of them end with a boxed list of the tags that will have
missing references, styled like Rename's References to Fix. Tags that are being
deleted too (inside the folder or the selection) are left out of that list.
Mission scripts that name a deleted tag follow the tags, one row per script with
how many of its paths will be missing, and a note that those missions load
without scripts until the paths are fixed. Delete never edits scripts, and a
path that another tag of the right type still answers to is not counted. A line
whose meaning can't be told is counted as "may break".
The bulk review opens as soon as you choose Delete, listing the selected items
right away. While Baboon checks references, the references box shows a spinner
and the check's progress, and Delete stays unavailable until the check finishes.
Both of the review's boxes show eight rows and scroll past that, so the window
keeps its size when the check fills in.
The single Delete Tag? and Delete Folder? windows check scripts in the
background as they open, with a small spinner beside "Checking scripts…", and
hold Delete until that check finishes.
Every delete window's Delete button has a red accent fill (the Delete icon's
red at 25%, 50% with a solid red border on hover) as a cue to take care.

Move completion remaps open tag paths, folder browsers, favorites and references,
and refreshes documents and the browser. The left navigation keeps its scroll
position while its folders reload. Completed roots leave the selection;
skipped roots remain selected. Copy leaves the source selection intact.

## Drag and drop

In Flat view, drag tags and folders onto a folder to move them there; hold
**Ctrl** when you let go to copy them instead. A drag carries the whole selection,
or just the item you grabbed if it wasn't selected (it becomes the selection).
You can drop onto:

- a folder row in the same view,
- another folder tab of the same kit (its tab header),
- another folder pane open beside it, anywhere in its listing,
- a folder row in a folder tab showing Tree view, or
- a folder in the left navigation pane.

Dragging out of Tree view still only sets references; moving starts in Flat view.

Above the pointer, plain text names what is dragged: the tag or folder, or
"3 items". A blue outline marks where the items will land, and a boxed label
below the pointer, led by a Move or Copy icon, says what will happen ("Move to
covenant"); away from any target there is no box. The two never overlap.
Where a drop isn't allowed, the cursor shows "not allowed" and the label says
why, in red: a folder can't go inside itself, and drops into another kit or a
read-only kit are refused. Asset Browsers say "Drop onto a folder in Flat
view"; the navigation pane and Tree view tabs, away from their folder rows, say
"Drop onto a folder"; a Groups view tab says "Switch this tab to Flat or Tree
view to drop here", and the navigation pane in Groups view "Switch to Folders
view to drop here" (any folder tab's header still takes the drop). The folder the items are already in is simply not a target: nothing is
outlined there, and letting go does nothing. Ctrl-dropping
into it duplicates them with numbered names, as Copy does.

A drop changes nothing by itself: it opens the same **Move Selected Items?**
review as Move (or Copy's review), with the folder already chosen, and nothing
moves until you confirm.

Dragging a single tag still fills tag reference fields and shader bitmap slots,
and hands the tag to Sapien or Guerilla. A drag of several items, or of a
folder, is refused there.

## Implementation boundaries

`browser/selection.rs` owns ordered selection and menu eligibility.
`browser/flat.rs` owns cached immediate listings and virtualized rows.
`browser/navigation.rs` owns keyboard ordering and type-to-select.
`browser/bulk.rs` coordinates prompts, whole-selection jobs and completion.
`browser/drag_move.rs` owns the Flat view drag payload, drop targets (folder
rows, folder panes, folder tab headers in `shell/tag_tiles.rs`) and the label
beside the pointer; a drop becomes a bulk Move or Copy with its destination set.
`core/folder_merge.rs` and `tag_ops/folder_merge_ops.rs` share the filesystem
plan, recovery and rollback journal between single-item and multi-root
operations: single tag and folder Rename and Move go through
`folder_merge_ops::run_reviewed_move`. `core/script_paths` finds and rewrites tag paths in
mission scripts; `tag_ops/script_fixes.rs` runs it for the windows (a background
check each window owns) and the jobs (inside their rollback transaction). Recovery batches also record `original_relative_paths`.

Tests use isolated temporary editing kits and headless egui frames; they never
move files in a user's editing kit.

Bulk Rename uses a preview dialog with one literal find/replace, where a
current-name token in Find renames whole names, preserving extensions and
parents. It runs a collision-strict variant of the shared multi-root
refactor: no merge, Replace or Keep Both path can change the reviewed names.
Collisions with existing selected names must be
resolved in separate batches. Completion remaps the owning pane's selection.
Type-to-select only changes the viewport when the target is outside it, avoiding
a list jump for already-visible matches. Flat folder/tag selections use the
same border-only style.

Single-folder and multi-selection **Extract** menus share the same options and
counts, including unopened subfolders. If the complete tag list is not ready,
choose **Load Folder**; its hover hint explains what is loaded. While loading,
the action is disabled. Once ready, folders with no supported exports show
**No Extractable Tags**. Loading does not open editor tabs.
