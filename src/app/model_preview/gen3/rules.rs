//! What each Halo 3 family game's loads do differently: part of its
//! [`GameRules`](super::super::load::GameRules).

/// One game's gen3 choices, flag by flag.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Gen3Rules {
    /// Whether a render method takes its definition's version update and
    /// its reference shader before it resolves, as Reach's tool does.
    pub(crate) reference_shaders: bool,
}

impl Gen3Rules {
    /// Halo 3's, which the others start from.
    pub(crate) const DEFAULT: Self = Self {
        reference_shaders: false,
    };
}
