//! Foundation-style recursive field editor and its shared row widgets.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

mod traversal;
pub(super) use traversal::*;
mod containers;
pub(super) use containers::*;
mod value_rows;
pub(super) use value_rows::*;
mod references;
pub(super) use references::*;
mod functions;
pub(super) use functions::*;
mod widgets;
pub(super) use widgets::*;

#[cfg(test)]
#[path = "foundation/tests.rs"]
mod extracted_tests;
