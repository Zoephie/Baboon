//! Which of the preview's features are Halo 2's own, by the path the load gate names, and how each
//! moves and what it reads.

use super::super::common::features::{attached_feature, features};

features!(super::H2Features {
    flares => "h2.flares",
    point_lights => "h2.point_lights",
    gel_lights => "h2.gel_lights",
    light_volumes => "h2.light_volumes",
    contrails => "h2.contrails",
    liquids => "h2.liquids";
    skip prt, hologram_control
});

// Lights and flares move when they animate (`object_moves`), and read their
// scale and the functions their channels name (`shader_input_names`).
attached_feature!(sprite values super::lights::H2FlareAt, moves: |at| at.animates(), inputs: |at| vec![at.scale_name()]);

attached_feature!(values super::lights::H2PointLight, moves: |at| at.animates(), inputs: |at| vec![at.scale_name()]);

attached_feature!(values super::lights::H2GelLight, moves: |at| at.animates(), inputs: |at| vec![at.scale_name()]);

// Halo 2's light volumes draw still at rest.
attached_feature!(sprite super::light_volumes::H2LightVolume, moves: |_at| false, inputs: |at| vec![at.scale_name()]);

attached_feature!(sprite super::contrails::H2ContrailAt, moves: |_at| true, inputs: |at| vec![at.scale_name()]);

attached_feature!(sprite super::liquid::H2LiquidAt, moves: |_at| true, inputs: |at| vec![at.scale_name()]);
