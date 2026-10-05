//! The application's model, free of any UI: the games, tag sources and their
//! catalogs, value formatting, storage locations, bundled files and helper processes.
//! Nothing here may depend on egui or on `crate::app`; `layering_tests`
//! enforces that.

pub(crate) mod bundled;
pub(crate) mod created_tags;
pub(crate) mod document;
pub(crate) mod format;
pub(crate) mod game;
pub(crate) mod keywords;
pub(crate) mod process;
pub(crate) mod source;
pub(crate) mod storage;
pub(crate) mod tag_key;
#[cfg(test)]
pub(crate) mod test_kits;
pub(crate) mod tool_commands;

#[cfg(test)]
mod layering_tests;
