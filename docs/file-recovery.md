# Deleted tags and folders

Loose-file deletion moves tags or whole folders into a recovery location. It
does not participate in the tag editor's Undo history. Unsaved edits are still
discarded when the deletion confirmation says so.

In **Add/Edit Editing Kit → Advanced**, **Deleted Tags Folder** shows the recovery location.
Use **Browse** to choose another location, **Open in File Explorer** to access
it, or **Reset to Default** to return to Baboon-managed storage. The **File →
Open Deleted Tags Folder** shortcut opens the current workspace's location.
An empty field uses the default shown as hint text. The hint may abbreviate
AppData as `%APPDATA%` on Windows or the home directory as `~` on macOS/Linux;
these are display abbreviations, and the full path is available on hover.
The opening button reads **Open in Finder** on macOS and **Open in File Manager**
on Linux.

Defaults follow the active installed or portable Baboon data folder:

```text
deleted-tags/
  halo2_mcc/
    halo_2_rebalance/
    halo_2_combat_evolved/
    misc/
  halo3_mcc/
    halo_3_vanilla/
    misc/
```

A kit receives a stable, readable folder name when its recovery settings are
first assigned. Renaming the kit does not rename that folder. Duplicate names
receive a suffix. A loaded tags folder without a saved kit uses its engine's
`misc` folder.
Individually opened loose tag files use `unknown-game/misc` when their engine
is unknown; their origin record identifies the original parent directory.

Custom locations can be absolute or relative to the kit root. Relative paths
follow root changes; absolute paths stay fixed. Defaults follow engine and
installed/portable storage changes. The form displays the resolved location.
Locations inside any configured or open live tags folder are rejected, including
paths that reach those folders through links. Shared custom recovery folders
are permitted with a warning.

Saving an editing kit creates its custom Deleted Tags Folder, including missing
parent directories. If the folder cannot be created, Save reports the error and
keeps the dialog open. Baboon also recreates missing directories when deleting
an item or opening the location. Default locations are created on first use.
Changing a location affects future deletions only; existing recovery files are
not moved or purged.

Each new deletion reserves a unique timestamped batch. Its `files` directory
preserves the original paths relative to the tags root; `recovery-info.json`
records the original root, kit identity, engine, operation and creation time.
To recover manually, use that record to identify the original tags directory
and copy the recovered contents of `files` back to the corresponding paths.
Folder recovery preserves non-tag files and empty subdirectories too.

Existing engine-level timestamp folders are migrated into `misc`. Their
contents and existing names are preserved, with suffixes for collisions. An
interrupted migration can resume. Legacy batches have no kit-origin information
and retain their original internal layout rather than gaining a `files` wrapper.

Campaign Evolved's container backups and deletion restrictions are unchanged.
Monolithic cache sources remain read-only. Folder merges preserve overwritten
destination files in the same recovery location; see [Folder merging](folder-merging.md).
