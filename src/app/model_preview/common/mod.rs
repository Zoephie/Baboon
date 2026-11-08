//! What every game's preview code shares: vector arithmetic and the random
//! number generator, each rule kept exactly as the code it came from wrote
//! it since games differ in the last bit, what attached things have in
//! common, reading a tag's fields, the functions tags drive by name, and the
//! view and pose things are placed by.

pub(in crate::app::model_preview) mod attached;
pub(in crate::app::model_preview) mod camera;
pub(in crate::app::model_preview) mod clock;
pub(in crate::app::model_preview) mod features;
pub(in crate::app::model_preview) mod fields;
pub(in crate::app::model_preview) mod flares;
pub(in crate::app::model_preview) mod geometry;
pub(in crate::app::model_preview) mod image;
pub(in crate::app::model_preview) mod marker;
pub(in crate::app::model_preview) mod math;
pub(in crate::app::model_preview) mod named_function;
pub(in crate::app::model_preview) mod plain;
pub(in crate::app::model_preview) mod point_physics;
pub(in crate::app::model_preview) mod random;
pub(in crate::app::model_preview) mod slots;
pub(in crate::app::model_preview) mod sprites;
pub(in crate::app::model_preview) mod view;
