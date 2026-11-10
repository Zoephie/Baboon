//! What one game's data carries inside a shared type: the shared code holds
//! and prints it without knowing what it is, and the game reads it back as
//! its own.

/// One game's value inside a shared type. Every `Debug`, `Send` and `Sync`
/// value is one; read it back with [`of`](dyn Extension::of), on the value
/// itself rather than the box that holds it.
pub(in crate::app::model_preview) trait Extension:
    std::fmt::Debug + Send + Sync + std::any::Any
{
    fn as_any(&self) -> &dyn std::any::Any;
}

impl<T: std::fmt::Debug + Send + Sync + std::any::Any> Extension for T {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl dyn Extension {
    /// The value, if it is a `T`.
    pub(in crate::app::model_preview) fn of<T: 'static>(&self) -> Option<&T> {
        self.as_any().downcast_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A boxed value reads back as itself, and as nothing else.
    #[test]
    fn an_extension_reads_back_as_its_own_type_only() {
        let boxed: Box<dyn Extension> = Box::new(7_u32);
        assert_eq!((*boxed).of::<u32>(), Some(&7));
        assert_eq!((*boxed).of::<i32>(), None);
        assert_eq!(format!("{boxed:?}"), "7", "it prints as the value");
    }
}
