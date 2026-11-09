//! What every feature reader is handed: the tag reader, the model's markers and the preview being built.

use super::super::{RenderModelPreviewMarker, TagFile};
use super::TagReader;

/// What a feature reader reads an object with: the kit, the object's tag, the
/// owner its features hang on, its markers and where `frame` places each, and
/// the name an attached object goes by (none for the previewed object).
pub(in crate::app::model_preview) struct LoadCx<'a, F> {
    pub(in crate::app::model_preview) reader: &'a TagReader,
    pub(in crate::app::model_preview) object: &'a TagFile,
    pub(in crate::app::model_preview) owner: u8,
    pub(in crate::app::model_preview) markers: &'a [RenderModelPreviewMarker],
    pub(in crate::app::model_preview) frame: &'a F,
    pub(in crate::app::model_preview) name: Option<&'a str>,
}

impl<F> Clone for LoadCx<'_, F> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<F> Copy for LoadCx<'_, F> {}
