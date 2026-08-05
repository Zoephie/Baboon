//! Carrying a pageable `tag_resource` across a profile pair.
//! It owns deciding whether a resource can cross and reporting when it cannot;
//! field matching and the conversion walk belong to the parent module.

use super::*;

/// Move `source`'s pageable resource onto `target`, returning whether anything
/// was carried.
///
/// A null resource is nothing to carry and is not a loss. A non-null one either
/// crosses whole or is recorded in [`ConversionContext::resources_left_behind`],
/// which is what makes the tag refuse to be written rather than land looking
/// converted and playing nothing.
///
/// The decision is the engine's: `copy_resource_from` proves the two resource
/// definitions describe the same shape before it moves a byte. Past the header
/// struct a resource payload is an opaque codec stream — for an animation graph
/// it is the compressed animation data itself — so there is no partial or
/// best-effort translation to attempt. It fits or it does not.
pub(super) fn transfer_resource(
    source: TagField<'_>,
    target: &mut TagFieldMut<'_>,
    path: &str,
    context: &mut ConversionContext<'_>,
) -> bool {
    let Some(resource) = source.as_resource() else {
        return false;
    };
    if matches!(resource.kind(), TagResourceKind::Null) {
        // Nothing to carry. Clear the destination so a template's own resource
        // cannot survive underneath a source that had none.
        let _ = target.clear_resource();
        return false;
    }

    match target.copy_resource_from(&resource) {
        Ok(()) => {
            context.report.transferred_resources += 1;
            true
        }
        Err(error) => {
            context.resources_left_behind.push(path.to_owned());
            record_unsupported(
                context,
                path.to_owned(),
                format!("The pageable resource could not be carried across: {error}"),
            );
            false
        }
    }
}
