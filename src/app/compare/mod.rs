//! Comparing tags: Git Review, Tag Compare, and the tag diff they and mod
//! review share.

use super::*;

pub(in crate::app) mod git_review;
pub(in crate::app) use git_review::*;
pub(in crate::app) mod git_review_window;
pub(in crate::app) mod tag_compare;
pub(in crate::app) mod diff;
pub(in crate::app) use diff::*;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;
