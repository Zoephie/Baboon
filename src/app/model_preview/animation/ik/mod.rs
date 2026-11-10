//! Inverse kinematics the preview poses with: a held weapon's hand reaching
//! its marker (every game's), and the solvers a clip carries, which each
//! game reads (Reach's IK chains, Halo 4's IK sets).

/// The markers a target names, the two-bone solve, and the clip's solvers.
pub(in crate::app::model_preview) mod solve;
/// A held weapon's IK point: the arm bent so the hand reaches the weapon's
/// marker.
pub(in crate::app::model_preview) mod weapon;
