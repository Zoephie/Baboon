//! Tag-browser tree, list, filtering, and context-menu presentation.
//! It owns tag-browser filtering and presentation; source discovery, document loading, and edit application belong elsewhere.

use super::*;

mod filter;
mod tree;

pub(super) use filter::*;
pub(super) use tree::*;
