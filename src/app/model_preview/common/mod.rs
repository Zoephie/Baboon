//! What every game's preview code shares: vector arithmetic, and the random
//! number generator with its output maps. Each rule is kept exactly as the
//! code it came from wrote it, since games differ in the last bit.

pub(in crate::app::model_preview) mod math;
