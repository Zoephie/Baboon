//! The panic guards around each feature reader: a reader that crashes leaves a note, and the preview loads without it.

use super::*;

/// Read one feature of a preview: a broken tag that panics its reader costs
/// that feature, noted as `what`, never the geometry.
pub(in crate::app::model_preview) fn feature<T: Default>(
    notes: &mut Vec<String>,
    what: &str,
    read: impl FnOnce() -> T,
) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|panic| {
        note_crash(notes, what, panic.as_ref());
        T::default()
    })
}

/// [`feature`] for a reader that adds to the preview itself, such as a held
/// weapon's geometry: a panic puts the preview back as it was, so nothing is
/// left half merged.
pub(in crate::app::model_preview) fn feature_into<T: Default>(
    notes: &mut Vec<String>,
    what: &str,
    preview: &mut RenderModelPreview,
    read: impl FnOnce(&mut RenderModelPreview) -> T,
) -> T {
    let before = preview.clone();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(preview))).unwrap_or_else(
        |panic| {
            *preview = before;
            note_crash(notes, what, panic.as_ref());
            T::default()
        },
    )
}

pub(super) fn note_crash(notes: &mut Vec<String>, what: &str, panic: &(dyn std::any::Any + Send)) {
    let detail = crate::app::shell::worker::panic_detail(panic);
    notes.push(format!("{what}: the reader crashed ({detail})"));
}
