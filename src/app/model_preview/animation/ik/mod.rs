//! Inverse kinematics the preview poses with: a held weapon's hand reaching
//! its marker (every game's), Reach's IK chains (also Halo 4's) and Halo 4's
//! IK sets.

/// Halo 4's IK sets.
pub(in crate::app::model_preview) mod h4;
/// Reach's IK chains.
pub(in crate::app::model_preview) mod reach;
/// A held weapon's IK point: the arm bent so the hand reaches the weapon's
/// marker.
pub(in crate::app::model_preview) mod weapon;
