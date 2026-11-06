//! What every kind of thing an object carries shares: the region it shows
//! under, and how it is renamed when the object it hangs on is brought into
//! another's preview.

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
pub(in crate::app::model_preview) fn rename_region(only_under: &mut Option<(String, String)>, name: &str) {
    if let Some((region, _)) = only_under {
        *region = format!("{name}: {region}");
    }
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
                $crate::app::model_preview::gen3::effects::adopt_event_function(
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
