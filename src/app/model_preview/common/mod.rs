//! What every game's preview code shares: vector arithmetic and the random
//! number generator, each rule kept exactly as the code it came from wrote
//! it since games differ in the last bit, and what attached things have in
//! common.

pub(in crate::app::model_preview) mod attached;
pub(in crate::app::model_preview) mod math;
pub(in crate::app::model_preview) mod random;
