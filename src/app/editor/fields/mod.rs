//! The generic field editor: every tag's fields drawn recursively from its
//! schema, laid out after Foundation's, with the row widgets the panels
//! share.

use super::*;

mod traversal;
pub(in crate::app) use traversal::*;
mod containers;
pub(in crate::app) use containers::*;
mod value_rows;
pub(in crate::app) use value_rows::*;
mod references;
pub(in crate::app) use references::*;
mod functions;
pub(in crate::app) use functions::*;
mod widgets;
pub(in crate::app) use widgets::*;

#[cfg(test)]
pub(in crate::app) mod extracted_tests;
