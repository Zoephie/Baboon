# Folder Browser

Open a folder in its own browser tab to browse, select and manage its contents
beside your open tags. Browser tabs can be split or moved to separate windows.
Each browser keeps its own location, view mode, search and selection.

## Folder and Asset Views

**Folder View** opens in **Flat** mode: it lists the current folder's immediate
subfolders and tags. Double-click a folder to enter it, use the breadcrumbs to
return to a parent, or press Alt+Up. Alt+Up selects and reveals the folder you
just left, retaining keyboard focus; if needed it returns to Folder View so
that folder can be selected. At the tags root it leaves the selection intact.
New, Sort and View are in the header.

The View menu also offers **Tree**, an expandable folder hierarchy, and
**Groups**, which groups descendant tags by type. These modes use single
selection. Only a folder's chevron expands or collapses it; its label selects
it, and double-clicking opens it. **Collapse All** in the View menu closes
every folder; a folder's right-click menu has it for just that folder and
everything inside it, as does Alt-clicking an open folder's arrow. There is
no Expand All: opening every folder of a big kit at once stops the app
answering.

## Searching

A search in the Tree or Groups view (and in the left browser) shows the
folders or groups that hold matches **closed**, each with how many matches it
holds, counting every folder inside it. Open one to look inside: while a
folder holds a single item, the next one opens too, so a lone path down to
the matches takes one click. A folder or group holding only one match opens
by itself, as does a lone folder at the top. With **Folders** ticked, a folder
matched by its name counts as a match too, along with everything inside it.

Folders you open or close stay that way while you refine the search. Clearing
the search shows the browser's folders as they were before it.

A search in **Flat** view lists every match beneath the folder, not just its
own: this folder's results first, then each folder's in path order, each row
led by its folder path in muted text. Multiple selection and every bulk
action work on the results. Moving, copying or deleting a folder together with
items inside it handles those items once, with their folder, and the review
says so. The footer counts the results.

Right-click a result in Flat view or the Asset Browser for **Open Tag Folder**,
which opens the folder it is in, in a new folder tab, with it selected.

**Asset Browser** shows bitmap and model previews beneath the current folder.
Use its type filters and search to narrow the listing. It supports multiple tag
selection, with fewer file-management actions because assets may span folders.

## Keywords

A folder carries keywords as a tag does: add them on the **Keywords** bar in
its header, and they show in its row's Keywords column. Typing in the bar
lists the keywords that match, so similar tags and folders share one
spelling: click one to add it, or use Up and Down and press Enter; Tab fills
the highlighted one in. Keywords used in other games are offered too. A
folder's keywords are not given to the tags inside it.

**Tools → Manage Keywords** lists every keyword, in every game, in your
order, with how many tags and folders in this kit carry each:

- Drag a row by its handle to reorder. Keyword pills everywhere (headers,
  the Keywords column, suggestions, the search box) follow this order.
- Click a keyword's pill to give it a colour, or none. Each colour has a
  light and a dark version.
- **Search** (magnifier) opens the kit's root in a folder window with the
  keyword in its search box.
- **Rename** (pencil) renames it on every tag and folder in every game.
  Renaming it to another keyword merges the two, after asking.
- **Delete** removes it from every tag and folder in every game, after
  asking. It can't be undone.

The row's count is this kit's only. The Merge and Delete warnings show both
this kit's count and the total across all your editing kits, since a
keyword unused here may still be used in another kit.
- **Add New Keyword** adds one before anything carries it.

Keywords keep the spelling they were first typed with ("To Be Deleted") and
match without letter case.

### Keyword pills in the search box

With **Keywords** ticked, typing in a search box suggests matching keywords.
Picking one (click, Tab, or Up/Down then Enter) makes it a pill in the box. A
pill matches its keyword exactly, so "wip" doesn't find "wip-old". Text typed
beside a pill searches as usual, and both must match: a **WIP** pill with
`model` finds tags named "model" that carry WIP. `^`, `$` and `|` still work,
and with `|` the pill applies to each alternative. Each pill has its own ×,
and Backspace in an empty box removes the last one.

Which tags and folders carry which keyword lives in Baboon's own data, not in
the tags, one file per game shared by that game's kits. The list of keywords,
their spelling, colours and order is one file for every game. Renaming or
moving a folder in Baboon carries its keywords, and those of the folders
inside it, along; deleting it drops them.

## Selection and keyboard navigation

- Click an item to select it. Ctrl-click (Command-click on macOS) toggles an
  item; Shift-click selects a range in the displayed order.
- Flat view permits mixed folders and tags. Selecting a folder includes its
  entire contents for file operations, including non-tag files and empty folders.
- Ctrl/Command+A selects the listed items in Flat or Asset Browser. The left
  navigation pane stays single-select and does not support Select All.
- Type a name prefix to jump to a matching item without searching or opening
  it. Repeating a letter cycles matching names; pausing resets the prefix.
- Escape or a blank-space click clears the selection. Navigating, searching or
  changing view filters clears it; sorting preserves it. Clicking another pane
  preserves the selection with a muted appearance.

The footer shows the item total in faded text, followed by the current
selection. Filtered Flat and Asset listings show how many items remain visible.
Folder-view totals count immediate subfolders and tags; non-tag files are not
listed. Asset totals count supported assets under the folder.

## Context menus and file operations

Right-click a selected item to act on the group, or an unselected item to target
it alone. Single folders retain the standard folder menu. Multi-selection menus
offer compact Move, Copy To and Delete buttons, Open, JSON export and compatible
Extract actions. Asset Browser omits Move and Copy To.

Move and Copy To choose one destination parent inside the current tags folder.
Matching folders merge. Conflicts offer Replace, Skip and Keep Both, with a
choice to apply to remaining conflicts. The whole selection shares one review
and rollback journal. Unsaved tags must be saved before file changes continue.
Delete shows a review and preserves deleted contents in one Deleted Tags batch;
replaced files are preserved there too. Cancellation is available before applying.

Opening a single selected folder navigates the current browser. Opening several
folders opens or reuses browser tabs without opening every tag inside them.
Opening ten or more new tabs requires confirmation.

Right-click a browser tab for **Reveal in Browser**, which reopens the left
navigation pane at its current folder, or the platform file-manager action for
a loose folder. Container folders do not have a directory on disk to open.

## Renaming

Rename and Bulk Rename have their own page, **Renaming**. The About button
in the Bulk Rename window opens it.

## Tabs and extraction

Tab-menu close actions apply only to the clicked tab group in that workspace.
**Close Tab** closes the clicked tab, **Close All** closes that group, and
**Close All Other Tabs** keeps the clicked tab and closes its group neighbors.
Other split groups and workspaces stay open. Unsaved tags in the closing group
still use the normal save/discard prompt. The File menu's Close All Tags remains
a workspace-wide action.

Single-folder and multi-selection **Extract** menus share the same options and
counts, including unopened subfolders. If the complete tag list is not ready,
choose **Load Folder**; its hover hint explains what is loaded. While loading,
the action is disabled. Once ready, folders with no supported exports show
**No Extractable Tags**. Loading does not open editor tabs.
