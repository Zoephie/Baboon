//! Which of the preview's features are the gen3 games' own, by the path the load gate names, and
//! how each moves and what it reads.

use super::super::common::features::{Feature, attached_feature, features};

features!(super::Gen3Features {
    cheap_emitters => "gen3.cheap_emitters",
    flares => "gen3.flares",
    light_volumes => "gen3.light_volumes",
    strips => "gen3.strips",
    screen_effects => "gen3.screen_effects";
    skip prt, ik_markers
});

attached_feature!(sprite super::effects::flares::Gen3FlareAt, moves: |at| at.animates(), inputs: |at| vec![at.scale_name()]);

attached_feature!(sprite super::effects::light_volumes::Gen3LightVolumeAt, moves: |at| at.animates(), inputs: |at| at.scale_names().into_iter().collect());

attached_feature!(sprite particles super::effects::strips::Gen3StripAt, moves: |_at| true, inputs: |at| at.scale_names().into_iter().collect());

impl Feature for super::effects::screen::Gen3ScreenEffectAt {
    fn moves(&self) -> bool {
        true
    }

    fn inputs(&self, object: u8) -> Vec<&str> {
        if self.object != object {
            return Vec::new();
        }
        vec![self.scale_name()]
    }

    /// The object functions its falloffs read, offered after every screen
    /// effect's scale.
    fn late_inputs(&self, object: u8) -> Vec<&str> {
        if self.object != object {
            return Vec::new();
        }
        self.object_inputs().collect()
    }
}
