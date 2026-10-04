//! Changing which tags exist and where: New Tag, renaming and moving tags and
//! folders with their references rewritten, duplicating, deleting, and folders
//! inside a Campaign Evolved container.

use super::*;

pub(in crate::app) mod delete;
pub(in crate::app) mod duplicate;
pub(in crate::app) mod rename_in_place;
pub(in crate::app) mod folder_rename;
pub(in crate::app) use folder_rename::sibling_differing_in_case;
pub(in crate::app) mod container_folders;
pub(in crate::app) mod new_tag_window;
pub(in crate::app) mod rename_tag_window;
pub(in crate::app) mod delete_confirm;
pub(in crate::app) mod container_duplicate_confirm;
pub(in crate::app) mod container_folder_window;
pub(in crate::app) mod loose_folder_rename_window;
pub(in crate::app) mod new_tag;
pub(in crate::app) use new_tag::*;
pub(in crate::app) mod refactor;
pub(in crate::app) use refactor::*;
