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
it, and double-clicking opens it.

**Asset Browser** shows bitmap and model previews beneath the current folder.
Use its type filters and search to narrow the listing. It supports multiple tag
selection, with fewer file-management actions because assets may span folders.

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
