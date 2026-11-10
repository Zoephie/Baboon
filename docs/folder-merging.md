# Folder merging

This implements Stage 2 of the File Management design agreed in this chat after
the grilling session. The original Notion Quick Note is background, not the final
specification.

- Move and Copy stay inside the current kit's tags directory. Matching folders
  merge recursively, including non-tag files and empty folders.
- Review source, destination, file counts and conflicts before merging into an
  existing destination, even if there are no file conflicts.
- Resolve conflicts individually with Replace, Skip or Keep Both, and an
  operation-only Apply this choice to all remaining conflicts checkbox.
- Show sizes, modification dates and distinct referring-tag counts for each
  file. Non-tag files show Not applicable. Complete reference checks first;
  unknown required reference information blocks the operation.
- Keep Both gives the incoming item the smallest available numbered suffix,
  before the extension for files. Names reserved by the operation count too.
  File/directory mismatches permit Skip or Keep Both, never Replace.
- Preserve overwritten destinations in the configured Deleted Tags Folder.
- Move rewrites references only to items actually moved. Skipped items remain
  at the source. Copy rewrites references within copied tags only. Keep Both
  references follow the incoming item's assigned name; references to the
  existing destination retain their identity.
- Offer Save All and Continue for unsaved edits. Stop if saving fails.
- Gather every decision before modifying files. If reviewed files change,
  rebuild the review. Cancellation is available during scanning and review.
- Lock editing while applying. On filesystem or reference-rewrite failure,
  stop and attempt rollback, retain recovery copies, and report anything that
  could not be restored.

Use the browser folder menu's **Move Folder To** or **Copy Folder To** action,
and choose the destination parent. The incoming folder keeps its name; when
that name is already present, the review opens. Rename Folder retains its
existing-name validation.

Conflicts are reviewed before any files change. A cancelled review creates no
recovery batches. On success, overwritten destinations remain in Deleted Tags
in one batch per operation, preserving their tag-root-relative folder paths
and origin information, and temporary rollback copies are removed. If a
rollback cannot restore something, the error identifies the paths and where its
rollback copies were retained. Recovery and rollback copies preserve timestamps.

Background catalog refreshes wait while a move, rename, copy or merge owns the
source snapshot. An already-running refresh is retried afterward: installing it
in the middle would advance the workspace generation and discard the operation's
completion, preventing the browser and open tabs from following the moved files.
Open documents reload automatically after completion, including background tabs
and rewritten referrers. Their last contents remain visible until the new read
finishes, and field-page scrolling belongs to the pane rather than the file path.
Reads started before the operation cannot overwrite the refreshed documents.

Preflight reports file counts and checks cancellation during metadata scanning.
It hashes the reviewed folder trees and possible referrers, while checking the
rest of the kit by size, modification time and permissions. This avoids reading
every tag payload twice for a small merge. Ordinary changes anywhere in the kit
during review trigger another review; content changes to reviewed files or
referrers are detected even if their size and modification time are preserved.
