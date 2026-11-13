# Renaming

## Renaming one tag or folder

Rename changes a tag or folder's name within its current parent. The window
opens with the current name selected, so typing replaces it; press **Enter**
or choose **Rename** to apply. `{name}` stands for the current name here too,
so `{name}_v2` adds a suffix. The Original and Rename row previews the result,
and **References to Fix** lists the tags whose references will be updated
(for a folder, tags outside it that reference tags inside it). Hover the
original name to see its full location.

Names cannot contain `/` or `\`; use **Move** to choose a different folder.

## Bulk Rename

Select tags and/or folders in Flat view, or tags in Asset Browser, then choose
**Rename** in the selection menu or press **F2**. Bulk Rename is available for
writable loose editing kits. It keeps each parent folder and tag extension.
Tags with the same name before different extensions can share a new name too.

The window shows the folder being renamed in at the top. On the right, the
preview table lists each item's Original name beside its Rename result, with
changed names in blue and problems in red.
Renaming is one **Find** and **Replace With** pair, in which `{name}` stands
for the current name without its extension:

- Find `{name}` (the default) to rename whole names: Replace With `test_{name}`
  adds a prefix, `{name}_rebalance` adds a suffix, and plain text replaces the name.
- Find other text to replace just that text. All matches are replaced, letter
  case is ignored (tag paths are case-insensitive), and items without a match
  stay unchanged.
- Double braces produce literal braces: `{{name}}` produces `{name}`.

Find and Replace match names only, not extensions or paths. An empty Find
changes nothing. Each field's **Current Name** button inserts `{name}` at that
field's text cursor, replacing highlighted text.

Invalid names, duplicate results, case-only changes and existing destination
names block Apply. Bulk Rename never overwrites or merges existing items, and
does not automatically suffix a conflicting result. This includes another
selected item's current name: choose vacant names or rename in two batches.

Review the preview, then choose **Rename**, or **Save All and Rename** if there
are unsaved workspace tags. Reference checking runs before files change. The
whole batch shares rollback handling; tag references, open tabs and folder
browsers follow successful renames. Renamed items stay selected.

## References

When a renamed folder holds tags, they get new paths too: a note under
**References to Fix** says how many, and references to them (including ones
between tags inside the folder) are fixed with the rest. A band under the Bulk
Rename preview counts the items selected and which folders and tags the rename
changes.

The list and note cover every selected item from the moment the window opens,
before any name changes. Renaming changes only the items whose names change,
and checks their references again before any file moves.

**References to Fix** lists the known referring tags for the entire selection,
with their tag icons and folders, including references within the selection and
renamed folders; each referring tag appears once. An unavailable or incomplete reference index is
clearly marked. References are checked again and updated automatically when the
rename runs.

Background index refreshes preserve the Bulk Rename form and its known-reference
list. A reference rebuild keeps the last known list until the new index is
ready; Rename checks the current index again before changing files.

## Scripts

Mission scripts name tags by path, such as `"objects\characters\grunt\grunt"`.
The editing kit compiles a scenario's scripts each time it loads the scenario,
and one path to a tag that no longer exists stops the whole mission's scripts
from compiling, so the mission loads with none. Rename and Move therefore fix
script paths along with tag references.

**References to Fix** lists each affected script after the referring tags, as
one row with a script icon, its folder muted, and how many of its paths change.
Hover a row to see each line: the path, what it becomes, and the type the script
uses it as. A note explains that scripts are updated in their `.hsc` file in
`data\` and in the copy saved in their scenario (and in any script source tags).
Fixing the `.hsc` file means re-importing or recompiling it cannot bring the old
path back.

The Rename windows check scripts in the background as they open: the box shows a
small spinner beside "Checking scripts…", and **Rename** stays unavailable until
the check finishes (its hover hint says so), so the window always says what will
change before it can run. Scripts are checked again when the rename runs.

Script paths have no extension: `objects\characters\grunt\grunt` can mean the
biped, the render model or the animation graph, depending on what the script
passes it to. Baboon reads that from the script documentation, so moving just the
biped fixes only the lines that want an object. When a line could mean a moved
tag or one that stays, and the documentation can't tell which, the line is left
unchanged and counted in red as **can't fix**. Comments that mention a path are
updated too, but not counted.

Scripts are fixed in the same batch as the tags: if a script can't be written
(it is read-only, say), nothing is renamed or moved. The compiled script data
saved in a scenario is left as it is; the tools rebuild it from the text.

Halo CE, Halo 2, Halo 3, ODST and Reach are checked. Halo 2 Anniversary
Multiplayer and Halo 4 keep their scripts in script tags that Baboon doesn't
check yet, so their Rename, Move and Delete windows say so.
