//! Appending geometry to a preview: where an append's indices count from, and
//! what each append does once the vertices no longer fit a 32-bit index.

/// What an append does when the preview's vertices, with its own, outgrow a
/// `u32` index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum Overflow {
    /// Nothing is appended once the vertex count, or the count with the
    /// append's own, no longer fits a `u32` index. Indices are plain sums from
    /// the base, which cannot pass it.
    RefusesAppend,
    /// As `RefusesAppend`, and each index is also summed checked, so a triangle
    /// with one that would pass `u32::MAX` is left out.
    DropsTriangle,
}

/// The first vertex an append's indices count from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) struct VertexBase {
    first: u32,
}

impl VertexBase {
    /// The base for appending `adding` vertices after `existing`, or `None`
    /// when `rule` refuses the append.
    pub(in crate::app::model_preview) fn of(existing: usize, adding: usize, rule: Overflow) -> Option<Self> {
        match rule {
            Overflow::RefusesAppend | Overflow::DropsTriangle => {
                let first = u32::try_from(existing).ok()?;
                if adding.checked_add(existing).is_none_or(|count| count > u32::MAX as usize) {
                    return None;
                }
                Some(Self { first })
            }
        }
    }

    /// The append's `index`th vertex: a plain sum, for `RefusesAppend`, whose
    /// base and count fit.
    pub(in crate::app::model_preview) fn offset(self, index: u32) -> u32 {
        self.first + index
    }

    /// The append's `index`th vertex, or `None` past `u32::MAX`, for
    /// `DropsTriangle`.
    pub(in crate::app::model_preview) fn checked_offset(self, index: u32) -> Option<u32> {
        self.first.checked_add(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOST: usize = u32::MAX as usize;

    #[test]
    fn an_append_that_fits_counts_from_the_vertices_before_it() {
        assert_eq!(VertexBase::of(5, 1, Overflow::RefusesAppend).map(|base| base.offset(4)), Some(9));
    }

    #[test]
    fn a_refused_append_is_one_whose_vertices_no_longer_fit() {
        for rule in [Overflow::RefusesAppend, Overflow::DropsTriangle] {
            assert_eq!(VertexBase::of(MOST - 4, 4, rule).map(|base| base.offset(0)), Some(u32::MAX - 4), "{rule:?}: exactly fits");
            assert_eq!(VertexBase::of(MOST - 4, 5, rule), None, "{rule:?}: one too many");
            assert_eq!(VertexBase::of(MOST + 1, 0, rule), None, "{rule:?}: the base itself too far");
            assert_eq!(VertexBase::of(1, usize::MAX, rule), None, "{rule:?}: the count overflows");
        }
    }

    #[test]
    fn a_checked_index_past_32_bits_is_none() {
        let base = VertexBase { first: u32::MAX - 1 };
        assert_eq!(base.checked_offset(1), Some(u32::MAX));
        assert_eq!(base.checked_offset(2), None);
        assert_eq!(base.offset(1), u32::MAX);
    }
}
