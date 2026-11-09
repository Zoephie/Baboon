//! Which of the preview's features are Halo 3's and ODST's own, by the path the load gate names,
//! and how each moves and what it reads.

use super::super::common::features::{Feature, attached_feature, features};

features!(super::H3Features {
    lights => "h3.lights",
    leaves => "h3.leaves";
    skip shield, vision, motion_blur
});

attached_feature!(super::lights::H3Light, moves: |at| at.animates(), inputs: |at| at.inputs().collect());

impl Feature for super::leaves::H3LeavesAt {
    const SPRITE: bool = true;
    const PARTICLE_PROGRAM: bool = true;

    fn moves(&self) -> bool {
        true
    }
}
