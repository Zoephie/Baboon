//! Tag paths: the names the games give tags, such as
//! `objects\characters\elite\elite`, separated by `\` on every host.
//!
//! A tag path is shown, compared, stored and handed to the kit's tools in this
//! form. It becomes a filesystem path only where a file is read or written,
//! through [`resolve_tag_path`] or [`to_relative`], and a filesystem path
//! becomes a tag path through [`relative_tag_path`]. Nothing else converts
//! between the two.

use std::path::PathBuf;

pub(crate) use blam_tags::paths::{relative_tag_path, resolve_tag_path};

/// The path relative to the tags folder that a tag path (or a folder's) names.
pub(crate) fn to_relative(path: &str) -> PathBuf {
    segments(path).collect()
}

/// The separator between a tag path's folders and its name.
pub(crate) const SEPARATOR: char = '\\';

/// The folder a tag path is in, or `None` for a tag at the top of the tags
/// folder: `objects\props\crate` is in `objects\props`.
pub(crate) fn parent(path: &str) -> Option<&str> {
    path.rsplit_once(SEPARATOR).map(|(parent, _)| parent)
}

/// A tag path's last segment: `objects\props\crate` is `crate`.
pub(crate) fn name(path: &str) -> &str {
    path.rsplit_once(SEPARATOR).map_or(path, |(_, name)| name)
}

/// A tag path split before the extension its name ends in, or `None` when it
/// has none: `levels\a10.old\foo.scenario` is `levels\a10.old\foo` and
/// `scenario`. A dot in a folder, or one that starts the name, is not an
/// extension.
pub(crate) fn split_extension(path: &str) -> Option<(&str, &str)> {
    let start = path.rfind(SEPARATOR).map_or(0, |separator| separator + 1);
    match path[start..].rfind('.') {
        Some(dot) if dot > 0 => Some((&path[..start + dot], &path[start + dot + 1..])),
        _ => None,
    }
}

/// A tag path without the extension its name ends in, if it has one. A dot in
/// a folder, or one that starts the name, is not an extension.
pub(crate) fn without_extension(path: &str) -> &str {
    split_extension(path).map_or(path, |(stem, _)| stem)
}

/// A tag path's folders and name, with empty segments skipped.
pub(crate) fn segments(path: &str) -> impl DoubleEndedIterator<Item = &str> {
    path.split(SEPARATOR).filter(|segment| !segment.is_empty())
}

/// `name` inside the folder `parent`; an empty `parent` is the top of the
/// tags folder.
pub(crate) fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}{SEPARATOR}{name}")
    }
}

/// The Unreal path a Campaign Evolved pak files a tag path under, beneath its
/// `Tags` root: `objects\crate` is `objects/crate`. Unreal separates with `/`.
pub(crate) fn to_unreal(path: &str) -> String {
    segments(path).collect::<Vec<_>>().join("/")
}

/// The tag path for an Unreal path beneath a Campaign Evolved pak's `Tags`
/// root: the inverse of [`to_unreal`].
pub(crate) fn from_unreal(path: &str) -> String {
    path.split('/').filter(|segment| !segment.is_empty()).collect::<Vec<_>>().join("\\")
}

/// A tag path a person typed. Typing `/` is accepted and means `\`: this is
/// the one place input in either form is taken, and it never leaves here.
pub(crate) fn from_input(text: &str) -> String {
    text.replace('/', "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_path_splits_on_backslash_only() {
        assert_eq!(parent("objects\\props\\crate"), Some("objects\\props"));
        assert_eq!(parent("crate"), None);
        assert_eq!(name("objects\\props\\crate"), "crate");
        assert_eq!(name("crate"), "crate");
        assert_eq!(segments("objects\\props\\crate").collect::<Vec<_>>(), ["objects", "props", "crate"]);
        assert_eq!(join("objects\\props", "crate"), "objects\\props\\crate");
        assert_eq!(join("", "crate"), "crate");
        assert_eq!(to_unreal("objects\\props\\crate"), "objects/props/crate");
        assert_eq!(from_unreal("objects/props/crate"), "objects\\props\\crate");
        assert_eq!(without_extension("objects\\props\\crate.scenery"), "objects\\props\\crate");
        assert_eq!(without_extension("levels\\v1.2\\rock"), "levels\\v1.2\\rock");
        assert_eq!(without_extension("objects\\.hidden"), "objects\\.hidden");
        assert_eq!(split_extension("levels\\a10.old\\foo.scenario"), Some(("levels\\a10.old\\foo", "scenario")));
        assert_eq!(split_extension("levels\\a10.old\\foo"), None);
        assert_eq!(split_extension("objects\\.hidden"), None);
        assert_eq!(split_extension("crate.scenery"), Some(("crate", "scenery")));
        assert_eq!(to_relative("objects\\props"), PathBuf::from("objects").join("props"));
        assert_eq!(relative_tag_path(&to_relative("objects\\props")), "objects\\props");
        // `/` is not a separator in a tag path, only in what a person types.
        assert_eq!(name("objects/props/crate"), "objects/props/crate");
        assert_eq!(from_input("objects/props/crate"), "objects\\props\\crate");
    }
}
