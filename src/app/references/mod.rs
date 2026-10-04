//! References: the reverse-dependency index, the content explorer, reference jumps, and fixing a tag's dependencies.

use super::*;

pub(in crate::app) mod index;
pub(in crate::app) use index::*;
pub(in crate::app) mod dependencies;
pub(in crate::app) use dependencies::*;
pub(in crate::app) mod ref_jump;
pub(in crate::app) mod explorer;
