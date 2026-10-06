//! Staged block editing: entry identity, transactions, and document commit.

use super::*;

fn entry_name_field(element: TagStruct<'_>, prefix: &str) -> Option<(String, String)> {
    for field in element.fields() {
        if !is_name_like_field(field.name()) || field_display_meta(field.name()).read_only {
            continue;
        }
        let text = match field.value() {
            Some(TagFieldData::String(text) | TagFieldData::LongString(text)) => text,
            Some(TagFieldData::StringId(id) | TagFieldData::OldStringId(id)) => id.string,
            _ => continue,
        };
        return Some((append_field_path_for(prefix, &field), text));
    }
    for field in element.fields() {
        if let Some(nested) = field.as_struct()
            && let Some(found) = entry_name_field(nested, &append_field_path_for(prefix, &field))
        {
            return Some(found);
        }
    }
    None
}

fn table_row(
    tag: &TagFile,
    path: &str,
    index: usize,
    id: u64,
    original: Option<usize>,
    names: &TagNameIndex,
) -> Result<BlockTableRow, String> {
    let field = tag
        .root()
        .field_path(path)
        .ok_or("Block no longer resolves")?;
    let block = field.as_block().ok_or("Field is not a block")?;
    let element = block.element(index).ok_or("Entry no longer resolves")?;
    let (name_field, name) = match entry_name_field(element, "") {
        Some((field, name)) => (Some(field), name),
        None => (
            None,
            block_element_content_label(element, names)
                .unwrap_or_else(|| element.name().to_owned()),
        ),
    };
    Ok(BlockTableRow {
        id,
        original_index: original,
        name_field,
        stored_name: name.clone(),
        name,
    })
}

impl BlockTableState {
    pub(in crate::app) fn has_changes(&self) -> bool {
        self.changed || self.rows.iter().any(|row| row.name != row.stored_name)
    }

    /// Validate on a fresh copy; a failed rename or index remap never partly
    /// mutates even the staged tag, much less the live document.
    pub(in crate::app) fn candidate(&self) -> Result<TagFile, String> {
        let bytes = self
            .tag
            .write_to_bytes()
            .map_err(|error| error.to_string())?;
        let mut candidate = crate::core::source::read_tag_from_bytes(
            &bytes,
            self.game,
            self.definitions_root.as_deref(),
            self.tag.group().tag,
        )
        .map_err(|error| error.to_string())?;
        for (index, row) in self.rows.iter().enumerate() {
            if row.name == row.stored_name {
                continue;
            }
            let field = row
                .name_field
                .as_ref()
                .ok_or("This entry has no editable name field")?;
            apply_field_edit(
                &mut candidate,
                &format!("{}[{index}]/{field}", self.request.path),
                &row.name,
            )
            .map_err(|error| format!("Entry {index}: {error}"))?;
        }
        Ok(candidate)
    }

    pub(in crate::app) fn stage(
        &mut self,
        kind: BlockOpKind,
        names: &TagNameIndex,
    ) -> Result<(), String> {
        let mut candidate = self.candidate()?;
        let field = candidate
            .root()
            .field_path(&self.request.path)
            .ok_or("Block no longer resolves")?;
        let block = field.as_block().ok_or("Field is not a block")?;
        let cap = block.definition().max_count() as usize;
        if cap != 0
            && block.len() >= cap
            && matches!(
                kind,
                BlockOpKind::Add | BlockOpKind::Insert(_) | BlockOpKind::Duplicate(_)
            )
        {
            return Err(format!("Block is at its schema maximum of {cap} entries"));
        }
        apply_one_block_op(
            &mut candidate,
            &BlockOp {
                path: self.request.path.clone(),
                kind: kind.clone(),
            },
        )?;
        let mut rows = self.rows.clone();
        for row in &mut rows {
            row.stored_name = row.name.clone();
        }
        match kind {
            BlockOpKind::Reorder { order } => {
                rows = order.into_iter().map(|index| rows[index].clone()).collect();
            }
            BlockOpKind::Insert(index) => {
                rows.insert(
                    index,
                    table_row(
                        &candidate,
                        &self.request.path,
                        index,
                        self.next_id,
                        None,
                        names,
                    )?,
                );
                self.next_id += 1;
            }
            BlockOpKind::Add => {
                rows.push(table_row(
                    &candidate,
                    &self.request.path,
                    rows.len(),
                    self.next_id,
                    None,
                    names,
                )?);
                self.next_id += 1;
            }
            BlockOpKind::Duplicate(index) => {
                let mut row = rows[index].clone();
                row.id = self.next_id;
                row.original_index = None;
                rows.insert(index + 1, row);
                self.next_id += 1;
            }
            BlockOpKind::Delete(index) => {
                rows.remove(index);
            }
            _ => return Err("Unsupported block table action".to_owned()),
        }
        self.changed = candidate
            .write_to_bytes()
            .map_err(|error| error.to_string())?
            != self.baseline_bytes;
        self.tag = candidate;
        self.rows = rows;
        self.status = None;
        Ok(())
    }
}

/// The block table for the block at `request.path` in `doc`: a private copy of
/// the tag that every staged change edits, and the rows it lists.
pub(in crate::app) fn block_table_for(
    kit: KitId,
    key: &str,
    doc: &TagDocument,
    source: Option<&LoadedSourceData>,
    names: &TagNameIndex,
    request: BlockTableRequest,
) -> Result<BlockTableState, String> {
    let baseline_bytes = doc.tag.write_to_bytes().map_err(|error| error.to_string())?;
    let game = source.and_then(|source| source.game);
    let definitions_root = source.and_then(|source| match &source.source {
        TagSource::LooseFolder {
            definitions_root, ..
        } => Some(definitions_root.clone()),
        _ => None,
    });
    let tag = crate::core::source::read_tag_from_bytes(
        &baseline_bytes,
        game,
        definitions_root.as_deref(),
        doc.tag.group().tag,
    )
    .map_err(|error| error.to_string())?;
    let count = tag
        .root()
        .field_path(&request.path)
        .and_then(|field| field.as_block())
        .ok_or("Field is not a block")?
        .len();
    let rows = (0..count)
        .map(|index| table_row(&tag, &request.path, index, index as u64, Some(index), names))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BlockTableState {
        kit,
        tag_key: key.to_owned(),
        request,
        stamp: doc.content_stamp(),
        game,
        definitions_root,
        tag,
        baseline_bytes,
        rows,
        next_id: count as u64,
        status: None,
        changed: false,
    })
}

impl Baboon {
    /// Commit the open block table's staged changes to its document, as one
    /// undo step. A refusal leaves the table open with the reason.
    pub(in crate::app) fn save_block_table(&mut self, ctx: &egui::Context) {
        let Some(mut table) = self.dialogs.close::<BlockTableState>() else {
            return;
        };
        let result = (|| {
            let kit_index = self
                .model
                .kit_index(table.kit)
                .ok_or("The editing kit is closed")?;
            if self.model.editing_kit_is_read_only(kit_index) {
                return Err("This editing kit is read-only".to_owned());
            }
            let doc = self.model.kits[kit_index]
                .parsed_tags
                .get_mut(&table.tag_key)
                .ok_or("Tag is no longer open")?;
            if !commit_table_document(&table, doc)? {
                return Ok(());
            }
            // Indexed drafts cannot survive moving entries to different paths.
            self.views[table.kit].edit_buffers.forget_tag(&table.tag_key);
            let selected = table
                .rows
                .iter()
                .position(|row| row.original_index == Some(table.request.selected))
                .unwrap_or(0);
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new((
                        "field_edit",
                        &table.request.view_scope,
                        &table.tag_key,
                        ("block_sel", &table.request.path),
                    )),
                    selected,
                )
            });
            self.invalidate_tag_caches_in(kit_index, &table.tag_key);
            self.model.status = format!("Reorganized {} (unsaved)", table.request.label);
            ctx.request_repaint();
            Ok(())
        })();
        if let Err(error) = result {
            table.status = Some(error);
            self.dialogs.open(table);
        }
    }
}

fn commit_table_document(table: &BlockTableState, doc: &mut TagDocument) -> Result<bool, String> {
    if doc.content_stamp() != table.stamp {
        return Err("This tag changed while the table was open. Cancel and reopen the table to use its latest contents.".to_owned());
    }
    let candidate = table.candidate()?;
    // Returning to the original order/name makes Save a harmless no-op.
    if candidate
        .write_to_bytes()
        .map_err(|error| error.to_string())?
        == table.baseline_bytes
    {
        return Ok(false);
    }
    doc.journal.end_edit_window();
    doc.journal.begin_edit(&doc.tag, "Reorganize Block Entries");
    doc.tag = candidate;
    doc.dirty.touch();
    doc.note_layout_change();
    doc.journal.end_edit_window();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(tag: &mut TagFile, path: &str, count: usize) {
        for _ in 0..count {
            apply_one_block_op(
                tag,
                &BlockOp {
                    path: path.to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
        }
    }

    fn fixture() -> TagFile {
        let mut tag =
            TagFile::new(crate::app::test_definition_path("halo2_mcc/model.json")).unwrap();
        add(&mut tag, "variants", 3);
        for index in 0..3 {
            let base = format!("variants[{index}]");
            apply_field_edit(
                &mut tag,
                &format!("{base}/name"),
                &format!("variant_{index}"),
            )
            .unwrap();
            add(&mut tag, &format!("{base}/regions"), 1);
            let region = format!("{base}/regions[0]");
            apply_field_edit(
                &mut tag,
                &format!("{region}/region name"),
                &format!("region_{index}"),
            )
            .unwrap();
            apply_field_edit(
                &mut tag,
                &format!("{region}/parent variant"),
                &((index + 1) % 3).to_string(),
            )
            .unwrap();
            add(&mut tag, &format!("{region}/permutations"), index + 1);
            for permutation in 0..=index {
                apply_field_edit(
                    &mut tag,
                    &format!("{region}/permutations[{permutation}]/permutation name"),
                    &format!("permutation_{index}_{permutation}"),
                )
                .unwrap();
            }
        }
        tag
    }

    fn staged(tag: &TagFile) -> BlockTableState {
        let baseline_bytes = tag.write_to_bytes().unwrap();
        let game = matches!(tag.container, blam_tags::file::TagContainer::Classic { .. })
            .then_some(GameId::Halo2);
        let definitions_root = game.as_ref().map(|_| {
            crate::app::test_definition_path("halo2_mcc/model.json")
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf()
        });
        let tag = crate::core::source::read_tag_from_bytes(
            &baseline_bytes,
            game,
            definitions_root.as_deref(),
            tag.group().tag,
        )
        .unwrap();
        let names = TagNameIndex::default();
        let rows = (0..3)
            .map(|index| {
                table_row(&tag, "variants", index, index as u64, Some(index), &names).unwrap()
            })
            .collect();
        BlockTableState {
            kit: KitId(1),
            tag_key: "test".to_owned(),
            request: BlockTableRequest {
                path: "variants".to_owned(),
                label: "Variants".to_owned(),
                view_scope: "test".to_owned(),
                selected: 0,
            },
            stamp: (1, 0),
            game,
            definitions_root,
            tag,
            baseline_bytes,
            rows,
            next_id: 3,
            status: None,
            changed: false,
        }
    }

    #[test]
    fn staged_reorder_and_rename_keep_nested_data_and_references_attached() {
        let live = fixture();
        let bytes = live.write_to_bytes().unwrap();
        let mut table = staged(&live);
        table.rows[2].name = "renamed_variant".to_owned();
        table
            .stage(
                BlockOpKind::Reorder {
                    order: vec![2, 0, 1],
                },
                &TagNameIndex::default(),
            )
            .unwrap();
        assert_eq!(
            table
                .rows
                .iter()
                .map(|row| row.original_index)
                .collect::<Vec<_>>(),
            vec![Some(2), Some(0), Some(1)]
        );
        assert_eq!(table.rows[0].name, "renamed_variant");
        assert!(table.has_changes());
        let saved = TagFile::read_from_bytes(&table.candidate().unwrap().write_to_bytes().unwrap())
            .unwrap();
        for (new, old) in [2, 0, 1].into_iter().enumerate() {
            let region_path = format!("variants[{new}]/regions[0]");
            let region = saved.root().descend(&region_path).unwrap();
            assert_eq!(
                region
                    .field_path("region name")
                    .unwrap()
                    .value()
                    .as_ref()
                    .and_then(stringish_label),
                Some(format!("region_{old}"))
            );
            let parent_old = (old + 1) % 3;
            let parent_new = [2, 0, 1]
                .iter()
                .position(|index| *index == parent_old)
                .unwrap();
            assert_eq!(
                region.read_int_any("parent variant"),
                Some(parent_new as i128)
            );
            let permutations = region
                .field_path("permutations")
                .unwrap()
                .as_block()
                .unwrap();
            assert_eq!(permutations.len(), old + 1);
            for permutation in 0..=old {
                assert_eq!(
                    first_named_string_label(permutations.element(permutation).unwrap()),
                    Some(format!("permutation_{old}_{permutation}"))
                );
            }
        }
        drop(table); // Cancel discards the private copy.
        assert_eq!(live.write_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn staged_insert_duplicate_delete_preserve_identity_and_reference_targets() {
        let live = fixture();
        let mut table = staged(&live);
        let names = TagNameIndex::default();
        table.rows[0].name = "renamed".to_owned();
        table.stage(BlockOpKind::Duplicate(0), &names).unwrap();
        assert_eq!(table.rows[1].original_index, None);
        assert_eq!(table.rows[1].name, "renamed");
        assert_eq!(
            table
                .tag
                .root()
                .descend("variants[1]/regions[0]")
                .unwrap()
                .read_int_any("parent variant"),
            Some(2)
        );
        table.stage(BlockOpKind::Insert(0), &names).unwrap();
        assert_eq!(table.rows[0].original_index, None);
        // Remove original variant 1. Both copies of variant 0 referenced it.
        table.stage(BlockOpKind::Delete(3), &names).unwrap();
        for index in [1, 2] {
            let region = table
                .tag
                .root()
                .descend(&format!("variants[{index}]/regions[0]"))
                .unwrap();
            assert_eq!(region.read_int_any("parent variant"), Some(-1));
            assert_eq!(
                region
                    .field_path("permutations")
                    .unwrap()
                    .as_block()
                    .unwrap()
                    .len(),
                1
            );
        }
        assert_eq!(table.rows[3].original_index, Some(2));
        assert_eq!(
            table
                .tag
                .root()
                .descend("variants[3]/regions[0]")
                .unwrap()
                .read_int_any("parent variant"),
            Some(1)
        );
    }

    #[test]
    fn returning_to_original_order_is_a_noop_and_invalid_order_is_atomic() {
        let live = fixture();
        let mut table = staged(&live);
        let names = TagNameIndex::default();
        let before = table.tag.write_to_bytes().unwrap();
        assert!(
            table
                .stage(
                    BlockOpKind::Reorder {
                        order: vec![1, 1, 2]
                    },
                    &names
                )
                .is_err()
        );
        assert_eq!(table.tag.write_to_bytes().unwrap(), before);
        table
            .stage(
                BlockOpKind::Reorder {
                    order: vec![2, 0, 1],
                },
                &names,
            )
            .unwrap();
        table
            .stage(
                BlockOpKind::Reorder {
                    order: vec![1, 2, 0],
                },
                &names,
            )
            .unwrap();
        assert!(!table.has_changes());
        assert_eq!(table.tag.write_to_bytes().unwrap(), before);
    }

    #[test]
    fn save_is_one_undo_step_and_conflicts_or_failed_names_leave_document_untouched() {
        let mut doc = TagDocument::clean(fixture());
        let before = doc.tag.write_to_bytes().unwrap();
        let mut table = staged(&doc.tag);
        table.stamp = doc.content_stamp();
        assert!(!commit_table_document(&table, &mut doc).unwrap());
        assert!(!doc.dirty.is_set());
        assert!(!doc.journal.can_undo());
        table.rows[1].name = "renamed".to_owned();
        table
            .stage(
                BlockOpKind::Reorder {
                    order: vec![2, 0, 1],
                },
                &TagNameIndex::default(),
            )
            .unwrap();
        assert!(commit_table_document(&table, &mut doc).unwrap());
        let after = doc.tag.write_to_bytes().unwrap();
        assert!(doc.dirty.is_set());
        let (undo, label) = doc.journal.undo(&doc.tag).unwrap();
        assert!(
            undo.as_ref() == &before,
            "undo restores the exact original bytes"
        );
        assert_eq!(label, "Reorganize Block Entries");
        assert!(
            !doc.journal.can_undo(),
            "the entire staged session is one step"
        );
        doc.tag = TagFile::read_from_bytes(&undo).unwrap();
        let (redo, _) = doc.journal.redo(&doc.tag).unwrap();
        assert!(redo.as_ref() == &after, "redo restores the staged commit");

        let mut doc = TagDocument::clean(fixture());
        let before_failed_name = doc.tag.write_to_bytes().unwrap();
        let mut table = staged(&doc.tag);
        table.stamp = doc.content_stamp();
        table.rows[0].name_field = Some("no such field".to_owned());
        table.rows[0].name = "bad".to_owned();
        assert!(commit_table_document(&table, &mut doc).is_err());
        assert!(
            doc.tag.write_to_bytes().unwrap() == before_failed_name,
            "a failed rename leaves the live document untouched"
        );
        assert!(!doc.dirty.is_set());
        assert!(!doc.journal.can_undo());

        let mut table = staged(&doc.tag);
        table.stamp = doc.content_stamp();
        table.rows[0].name = "staged_name".to_owned();
        apply_field_edit(&mut doc.tag, "variants[2]/name", "concurrent_name").unwrap();
        doc.dirty.touch();
        let concurrent = doc.tag.write_to_bytes().unwrap();
        assert!(
            commit_table_document(&table, &mut doc)
                .unwrap_err()
                .contains("changed while")
        );
        assert_eq!(doc.tag.write_to_bytes().unwrap(), concurrent);
        assert!(!doc.journal.can_undo());
    }

    #[test]
    fn classic_halo2_table_preserves_nested_entries_and_parent_references() {
        let layout_tag =
            TagFile::new(crate::app::test_definition_path("halo2_mcc/model.json")).unwrap();
        let mut header = vec![0; 64];
        header[36..40].copy_from_slice(&layout_tag.group().tag.to_le_bytes());
        header[60..64].copy_from_slice(b"!MLB");
        // Real classic root header: MCC-created tags have no classic root header.
        header.extend_from_slice(b"dfbt");
        header.extend_from_slice(&1u32.to_le_bytes());
        header.extend_from_slice(&1u32.to_le_bytes());
        header.extend_from_slice(&(layout_tag.root().raw().len() as u32).to_le_bytes());
        header.resize(header.len() + layout_tag.root().raw().len(), 0);
        let definitions_root = crate::app::test_definition_path("halo2_mcc/model.json")
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let mut live = crate::core::source::read_tag_from_bytes(
            &header,
            Some(GameId::Halo2),
            Some(&definitions_root),
            layout_tag.group().tag,
        )
        .unwrap();
        add(&mut live, "variants", 3);
        for index in 0..3 {
            let base = format!("variants[{index}]");
            apply_field_edit(
                &mut live,
                &format!("{base}/name"),
                &format!("variant_{index}"),
            )
            .unwrap();
            add(&mut live, &format!("{base}/regions"), 1);
            let region = format!("{base}/regions[0]");
            apply_field_edit(
                &mut live,
                &format!("{region}/parent variant"),
                &((index + 1) % 3).to_string(),
            )
            .unwrap();
            add(&mut live, &format!("{region}/permutations"), index + 1);
        }
        let mut table = staged(&live);
        table
            .stage(
                BlockOpKind::Reorder {
                    order: vec![2, 0, 1],
                },
                &TagNameIndex::default(),
            )
            .unwrap();
        table.rows[0].name = "renamed_classic".to_owned();
        let candidate = table.candidate().unwrap();
        let saved = crate::core::source::read_tag_from_bytes(
            &candidate.write_to_bytes().unwrap(),
            table.game,
            table.definitions_root.as_deref(),
            candidate.group().tag,
        )
        .unwrap();
        assert_eq!(
            first_named_string_label(saved.root().descend("variants[0]").unwrap()),
            Some("renamed_classic".to_owned())
        );
        let region = saved.root().descend("variants[0]/regions[0]").unwrap();
        assert_eq!(region.read_int_any("parent variant"), Some(1));
        assert_eq!(
            region
                .field_path("permutations")
                .unwrap()
                .as_block()
                .unwrap()
                .len(),
            3
        );
    }
}
