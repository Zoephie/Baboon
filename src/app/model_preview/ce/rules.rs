//! Halo CE's load rules.

use super::super::load::GameRules;

/// A Halo CE object names its gbxmodel and attaches nothing by the later
/// games' readers.
pub(in crate::app::model_preview) const RULES: GameRules = GameRules::DEFAULT;
