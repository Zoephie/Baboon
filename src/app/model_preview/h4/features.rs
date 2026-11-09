//! Which of the preview's features are Halo 4's own, by the path the load gate names, and how each
//! moves and what it reads.

use super::super::common::features::{attached_feature, features};

features!(super::H4Features {
    lights => "h4.lights",
    cheap_lights => "h4.cheap_lights",
    flares => "h4.flares",
    tracers => "h4.tracers";
    skip light_passes, flare_pass, camo, shield, shadow_caster, color_grading, bounding_sphere, change_color_count
});

attached_feature!(super::lights::H4Light, moves: |at| at.animates(), inputs: |at| at.inputs().collect());

attached_feature!(super::lights::H4CheapLight, moves: |at| at.animates(), inputs: |at| at.inputs().collect());

attached_feature!(sprite super::flares::H4FlareAt, moves: |at| at.animates(), inputs: |at| at.inputs().collect());

// Simulations run on whenever there are any.
attached_feature!(sprite super::tracers::H4TracerAt, moves: |_at| true, inputs: |at| at.scale_names().into_iter().collect());
