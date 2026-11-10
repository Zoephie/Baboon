//! Which game's shader dialect a render method speaks.

use super::TagSource;

/// Whose frame a material is lit and drawn in: Halo 3's (and ODST's),
/// Reach's or Halo 4's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gen3Dialect {
    Halo3,
    Reach,
    /// Halo 4's materials: translated from their own sources
    /// (`h4::materials::shaders`), never through `Gen3Sources`.
    Halo4,
}

/// Whose tool lays out a render method's sources: Halo 3's (and ODST's) or
/// Reach's. Halo 4 has materials, not render methods, so it has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenderMethodDialect {
    Halo3,
    Reach,
}

impl RenderMethodDialect {
    /// The kit's game's, by its rules.
    pub(super) fn of(source: &TagSource) -> Self {
        super::super::super::load::rules(source.game()).gen3.dialect
    }
}

impl From<RenderMethodDialect> for Gen3Dialect {
    fn from(dialect: RenderMethodDialect) -> Self {
        match dialect {
            RenderMethodDialect::Halo3 => Self::Halo3,
            RenderMethodDialect::Reach => Self::Reach,
        }
    }
}
