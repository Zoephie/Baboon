//! What every kind of thing an object carries shares: the region it shows
//! under, how it is renamed when the object it hangs on is brought into
//! another's preview, and how Reach and Halo 4 read an object's attachments.

/// A light, flare, volume, strip, particle system or the like that hangs on
/// an object.
pub(in crate::app::model_preview) trait Attached: Sized {
    /// The region and permutation it shows only under, if any.
    fn only_under(&self) -> Option<&(String, String)>;

    /// Renamed for the object `name` brings: its region as `merge_attached`
    /// renames it, and, unless the kind keeps its function, an effect event's
    /// function handed to the owner that plays it.
    fn on_attached(self, name: &str) -> Self;
}

/// `only_under`'s region as `merge_attached` renames the regions of what
/// `name` brings.
pub(in crate::app::model_preview) fn rename_region(
    only_under: &mut Option<(String, String)>,
    name: &str,
) {
    if let Some((region, _)) = only_under {
        *region = format!("{name}: {region}");
    }
}

/// One entry of an object's `attachments` block, as Reach and Halo 4 read it.
pub(in crate::app::model_preview) struct ObjectAttachment {
    pub(in crate::app::model_preview) group: u32,
    pub(in crate::app::model_preview) path: String,
    /// The marker it hangs on; none is the object's root.
    pub(in crate::app::model_preview) marker: String,
    pub(in crate::app::model_preview) scale_by: String,
    pub(in crate::app::model_preview) secondary_by: String,
    /// The tag's first option is none; the change colour channels follow.
    pub(in crate::app::model_preview) change_color: Option<usize>,
    /// `force always on`.
    pub(in crate::app::model_preview) force_on: bool,
}

/// `object`'s attachments whose type is one of `groups`, with a path, as
/// Reach and Halo 4 read them. Halo 3 and earlier read theirs differently
/// (no root default, the change colour as stored) and keep their own scans.
pub(in crate::app::model_preview) fn object_attachments(
    object: &blam_tags::TagFile,
    groups: &[u32],
) -> Vec<ObjectAttachment> {
    use crate::app::model_preview::common::fields::{
        elements, function_slot_of, has_flag, object_block,
    };
    use crate::app::model_preview::effects::marker_name;
    let Some(block) = object_block(object, "attachments") else {
        return Vec::new();
    };
    elements(block, "attachments")
        .into_iter()
        .filter_map(|attachment| {
            let (group, path) = attachment.read_tag_ref_with_group("type")?;
            if path.is_empty() || !groups.contains(&group) {
                return None;
            }
            let marker = marker_name(attachment, "marker");
            Some(ObjectAttachment {
                group,
                path,
                marker: if marker.is_empty() {
                    "root".to_owned()
                } else {
                    marker
                },
                scale_by: attachment
                    .read_string_id("primary scale")
                    .unwrap_or_default(),
                secondary_by: attachment
                    .read_string_id("secondary scale")
                    .unwrap_or_default(),
                change_color: function_slot_of(attachment, "change color"),
                force_on: has_flag(attachment, "flags", "force always on"),
            })
        })
        .collect()
}

/// What an attached object brings, renamed after it when it has a name, so
/// its owner's panel can tell its effects apart.
pub(in crate::app::model_preview) fn named<T: Attached>(
    found: impl IntoIterator<Item = T>,
    name: Option<&str>,
) -> impl Iterator<Item = T> {
    found.into_iter().map(move |at| match name {
        Some(name) => at.on_attached(name),
        None => at,
    })
}

/// [`Attached`] for kinds with `only_under`, `scale_by` and `object` fields,
/// written where the fields are visible. `keeps function:` is for kinds whose
/// function stays with the object they are on.
macro_rules! attached {
    (keeps function: $($kind:ty),+ $(,)?) => {
        $(impl $crate::app::model_preview::common::attached::Attached for $kind {
            fn only_under(&self) -> Option<&(String, String)> {
                self.only_under.as_ref()
            }

            fn on_attached(mut self, name: &str) -> Self {
                $crate::app::model_preview::common::attached::rename_region(&mut self.only_under, name);
                self
            }
        })+
    };
    ($($kind:ty),+ $(,)?) => {
        $(impl $crate::app::model_preview::common::attached::Attached for $kind {
            fn only_under(&self) -> Option<&(String, String)> {
                self.only_under.as_ref()
            }

            fn on_attached(mut self, name: &str) -> Self {
                $crate::app::model_preview::effects::adopt_event_function(
                    name,
                    &mut self.scale_by,
                    &mut self.object,
                );
                $crate::app::model_preview::common::attached::rename_region(&mut self.only_under, name);
                self
            }
        })+
    };
}
pub(in crate::app::model_preview) use attached;
