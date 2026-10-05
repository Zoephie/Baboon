//! The generic field editor: every tag's fields drawn recursively from its
//! schema, laid out after Foundation's, with the row widgets the panels
//! share.

use super::*;

mod traversal;
pub(in crate::app) use traversal::*;
mod containers;
pub(in crate::app) use containers::*;
mod value_rows;
pub(in crate::app) use value_rows::*;
mod references;
pub(in crate::app) use references::*;
mod functions;
pub(in crate::app) use functions::*;
mod widgets;
pub(in crate::app) use widgets::*;


/// Runs `assertion` against an editable field-edit context for a Halo 3
/// `jpt!` tag, with the bundled definitions.
#[cfg(test)]
pub(in crate::app) fn with_test_edit_context(
    assertion: impl FnOnce(&mut FieldEditContext<'_>),
) {
    let definitions_root = locate_definitions_root();
    let mut sinks = EditSinks::default();
    let mut edit = FieldEditContext::read_only(&mut sinks, "test", "test");
    edit.group_tag = parse_group_tag("jpt!").unwrap();
    edit.game = Some(GameId::Halo3);
    edit.definitions_root = Some(definitions_root.as_path());
    edit.editable = true;
    assertion(&mut edit);
}
