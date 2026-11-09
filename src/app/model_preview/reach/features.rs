//! Which of the preview's features are Reach's own, by the path the load gate names, and how each
//! moves and what it reads.

use super::super::common::features::{attached_feature, features};

features!(super::ReachFeatures {
    lights => "reach.lights",
    cheap_lights => "reach.cheap_lights";
    skip light_passes, shield, camo, shadow_caster, screen_noise
});

attached_feature!(super::lights::ReachLight, moves: |at| at.animates(), inputs: |at| at.inputs().collect());

attached_feature!(super::lights::CheapLight, moves: |at| at.animates(), inputs: |at| at.inputs().collect());
