//! What one preview load reads tags through: the source, and every tag it has
//! read so far, so a tag named again (a particle render method's definition by
//! each system, a shared bitmap by each material) is parsed once per load.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use blam_tags::TagFile;
use blam_tags::render_method::{RenderMethodDefinition, RenderMethodOption};

use crate::app::export::load_referenced_tag_from_source;
use crate::core::source::TagSource;

/// A tag as one load asked for it: the reference exactly as given, its
/// extension and its group.
type TagKey = (String, String, [u8; 4]);

/// The source a preview load reads from, with the tags it has read. Made when
/// a load starts and dropped when it ends, so an edit or a reload never sees a
/// tag read before it. A load reads on one thread, hence the `RefCell`.
pub(in crate::app) struct TagReader {
    pub(in crate::app::model_preview) source: TagSource,
    read: RefCell<HashMap<TagKey, Result<Arc<TagFile>, String>>>,
    /// The render methods' definitions and options parsed so far, as the
    /// shader editor's own caches keep them.
    definitions: RefCell<HashMap<String, Option<Arc<RenderMethodDefinition>>>>,
    options: RefCell<HashMap<String, Option<Arc<RenderMethodOption>>>>,
}

impl TagReader {
    pub(in crate::app::model_preview) fn new(source: &TagSource) -> Self {
        Self {
            source: source.clone(),
            read: RefCell::new(HashMap::new()),
            definitions: RefCell::new(HashMap::new()),
            options: RefCell::new(HashMap::new()),
        }
    }

    /// A render method's definition, parsed once per load.
    pub(in crate::app::model_preview) fn render_method_definition(
        &self,
        reference: &str,
    ) -> Option<Arc<RenderMethodDefinition>> {
        crate::app::editor::cached_render_method_definition(
            &self.source,
            reference,
            &mut self.definitions.borrow_mut(),
        )
    }

    /// A render method's option, parsed once per load.
    pub(in crate::app::model_preview) fn render_method_option(
        &self,
        reference: &str,
    ) -> Option<Arc<RenderMethodOption>> {
        crate::app::editor::cached_render_method_option(
            &self.source,
            reference,
            &mut self.options.borrow_mut(),
        )
    }

    /// `reference`'s tag, read the first time it is asked for. A failure is
    /// kept too: asked again, it fails with the same message.
    pub(in crate::app::model_preview) fn load(
        &self,
        reference: &str,
        extension: &str,
        group: &[u8; 4],
    ) -> anyhow::Result<Arc<TagFile>> {
        let key = (reference.to_owned(), extension.to_owned(), *group);
        if let Some(read) = self.read.borrow().get(&key) {
            return read.clone().map_err(|message| anyhow::anyhow!(message));
        }
        // Every error the loader makes is one message with no cause behind
        // it, so its text is all there is to keep.
        let read = load_referenced_tag_from_source(&self.source, reference, extension, group)
            .map(Arc::new)
            .map_err(|error| error.to_string());
        self.read.borrow_mut().insert(key, read.clone());
        read.map_err(|message| anyhow::anyhow!(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::harness::kit::kit_root;
    use crate::core::game::GameId;

    fn reader() -> TagReader {
        TagReader::new(&TagSource::LooseFolder {
            root: std::env::temp_dir().join("baboon-tag-reader-test-missing-root"),
            game: None,
            definitions_root: crate::core::bundled::locate_definitions_root(),
        })
    }

    #[test]
    fn a_failure_is_read_once_and_fails_the_same_again() {
        let reader = reader();
        let first = reader
            .load("objects\\missing\\missing", "biped", b"bipd")
            .unwrap_err()
            .to_string();
        assert_eq!(reader.read.borrow().len(), 1, "the failure is kept");
        let again = reader
            .load("objects\\missing\\missing", "biped", b"bipd")
            .unwrap_err()
            .to_string();
        assert_eq!(first, again);
        assert_eq!(
            format!(
                "{:#}",
                reader
                    .load("objects\\missing\\missing", "biped", b"bipd")
                    .unwrap_err()
            ),
            first
        );
    }

    #[test]
    fn a_tag_read_twice_is_the_same_tag() {
        let Some(root) = kit_root(GameId::Halo2) else {
            return;
        };
        let reader = TagReader::new(&TagSource::LooseFolder {
            root,
            game: Some(crate::core::game::GameId::Halo2),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        });
        let path = "objects\\characters\\masterchief\\masterchief";
        let first = reader.load(path, "biped", b"bipd").unwrap();
        let again = reader.load(path, "biped", b"bipd").unwrap();
        assert!(Arc::ptr_eq(&first, &again), "read once, handed out twice");
        assert_eq!(reader.read.borrow().len(), 1);
    }
}
