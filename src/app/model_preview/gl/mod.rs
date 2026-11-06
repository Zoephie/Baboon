//! The GL toolkit every pipeline draws with: objects that clean up after
//! themselves when a later step fails.

pub(in crate::app::model_preview) mod chain;
pub(in crate::app::model_preview) mod link;
pub(in crate::app::model_preview) mod scope;
pub(in crate::app::model_preview) mod target;
