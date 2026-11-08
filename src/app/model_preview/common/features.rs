//! What a preview carries besides its geometry and materials: each kind of
//! light, flare, effect, widget and simulation an object brings, listed once
//! per game so the code that asks about all of them (whether anything draws,
//! whether the frame moves, which functions the panel offers, which show under
//! the chosen regions) asks the lists instead of naming every field.

// The central sites move onto these one at a time; until then only the tests
// call them.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashMap;
use std::sync::Arc;

use super::super::ModelRegionSelection;
use super::super::RenderModelPreview;
use super::super::renderer::marker_active;

/// One of the things a preview carries.
pub(in crate::app::model_preview) trait Feature {
    /// The region and permutation it shows only under, if any.
    fn only_under(&self) -> Option<&(String, String)> {
        None
    }

    /// Whether it changes with time alone, so the preview keeps drawing.
    fn moves(&self) -> bool {
        false
    }

    /// The function names it reads on `object`, for the panel's sliders:
    /// none unless it hangs on `object`.
    fn inputs(&self, _object: u8) -> Vec<&str> {
        Vec::new()
    }
}

impl<T: Feature> Feature for Arc<T> {
    fn only_under(&self) -> Option<&(String, String)> {
        T::only_under(self)
    }

    fn moves(&self) -> bool {
        T::moves(self)
    }

    fn inputs(&self, object: u8) -> Vec<&str> {
        T::inputs(self, object)
    }
}

/// A preview field of one kind of [`Feature`], asked about as a whole.
pub(in crate::app::model_preview) trait FeatureList {
    /// Whether it holds none.
    fn is_empty(&self) -> bool;

    /// Whether any it holds moves.
    fn moves(&self) -> bool;

    /// The function names those on `object` read, in order, empty names left
    /// out.
    fn inputs_of(&self, object: u8) -> Vec<&str>;

    /// Whether each shows under the chosen regions.
    fn shown(&self, selections: &HashMap<String, ModelRegionSelection>) -> Vec<bool>;
}

fn inputs_of<'a, T: Feature + 'a>(items: impl Iterator<Item = &'a T>, object: u8) -> Vec<&'a str> {
    items.flat_map(|item| item.inputs(object)).filter(|name| !name.is_empty()).collect()
}

fn shown<'a, T: Feature + 'a>(
    items: impl Iterator<Item = &'a T>,
    selections: &HashMap<String, ModelRegionSelection>,
) -> Vec<bool> {
    items.map(|item| item.only_under().is_none_or(|only| marker_active(only, selections))).collect()
}

impl<T: Feature> FeatureList for Vec<T> {
    fn is_empty(&self) -> bool {
        <[T]>::is_empty(self)
    }

    fn moves(&self) -> bool {
        self.iter().any(Feature::moves)
    }

    fn inputs_of(&self, object: u8) -> Vec<&str> {
        inputs_of(self.iter(), object)
    }

    fn shown(&self, selections: &HashMap<String, ModelRegionSelection>) -> Vec<bool> {
        shown(self.iter(), selections)
    }
}

impl<T: Feature> FeatureList for Option<T> {
    fn is_empty(&self) -> bool {
        self.is_none()
    }

    fn moves(&self) -> bool {
        self.iter().any(Feature::moves)
    }

    fn inputs_of(&self, object: u8) -> Vec<&str> {
        inputs_of(self.iter(), object)
    }

    fn shown(&self, selections: &HashMap<String, ModelRegionSelection>) -> Vec<bool> {
        shown(self.iter(), selections)
    }
}

/// A group of a preview's features: each field that is one, by its path as
/// the load gate names it after `preview.`.
pub(in crate::app::model_preview) trait Features {
    fn each(&self, visit: &mut dyn FnMut(&'static str, &dyn FeatureList));
}

impl RenderModelPreview {
    /// Every feature the preview carries, in its fields' order.
    pub(in crate::app::model_preview) fn each_feature(&self, visit: &mut dyn FnMut(&'static str, &dyn FeatureList)) {
        visit("particles", &self.particles);
        visit("antennas", &self.antennas);
        visit("cloth", &self.cloth);
        self.h2.each(visit);
        self.gen3.each(visit);
        self.h3.each(visit);
        self.reach.each(visit);
        self.h4.each(visit);
        self.ce.each(visit);
    }

    /// Whether any feature answers `test`.
    pub(in crate::app::model_preview) fn any_feature(&self, mut test: impl FnMut(&dyn FeatureList) -> bool) -> bool {
        let mut any = false;
        self.each_feature(&mut |_, list| any = any || test(list));
        any
    }
}

impl Features for super::super::ce::CeFeatures {
    fn each(&self, visit: &mut dyn FnMut(&'static str, &dyn FeatureList)) {
        let Self { lights, antennas, effects, firing: _, contrails, light_volumes, lightning, glows, flags, particle_systems } =
            self;
        visit("ce.lights", lights);
        visit("ce.antennas", antennas);
        visit("ce.effects", effects);
        visit("ce.contrails", contrails);
        visit("ce.light_volumes", light_volumes);
        visit("ce.lightning", lightning);
        visit("ce.glows", glows);
        visit("ce.flags", flags);
        visit("ce.particle_systems", particle_systems);
    }
}

impl Features for super::super::h2::H2Features {
    fn each(&self, visit: &mut dyn FnMut(&'static str, &dyn FeatureList)) {
        let Self { prt: _, hologram_control: _, flares, point_lights, gel_lights, light_volumes, contrails, liquids } = self;
        visit("h2.flares", flares);
        visit("h2.point_lights", point_lights);
        visit("h2.gel_lights", gel_lights);
        visit("h2.light_volumes", light_volumes);
        visit("h2.contrails", contrails);
        visit("h2.liquids", liquids);
    }
}

impl Features for super::super::gen3::Gen3Features {
    fn each(&self, visit: &mut dyn FnMut(&'static str, &dyn FeatureList)) {
        let Self { prt: _, cheap_emitters, ik_markers: _, flares, light_volumes, strips, screen_effects } = self;
        visit("gen3.cheap_emitters", cheap_emitters);
        visit("gen3.flares", flares);
        visit("gen3.light_volumes", light_volumes);
        visit("gen3.strips", strips);
        visit("gen3.screen_effects", screen_effects);
    }
}

impl Features for super::super::h3::H3Features {
    fn each(&self, visit: &mut dyn FnMut(&'static str, &dyn FeatureList)) {
        let Self { shield: _, lights, leaves, vision: _, motion_blur: _ } = self;
        visit("h3.lights", lights);
        visit("h3.leaves", leaves);
    }
}

impl Features for super::super::reach::ReachFeatures {
    fn each(&self, visit: &mut dyn FnMut(&'static str, &dyn FeatureList)) {
        let Self { lights, cheap_lights, light_passes: _, shield: _, camo: _, shadow_caster: _, screen_noise: _ } = self;
        visit("reach.lights", lights);
        visit("reach.cheap_lights", cheap_lights);
    }
}

impl Features for super::super::h4::H4Features {
    fn each(&self, visit: &mut dyn FnMut(&'static str, &dyn FeatureList)) {
        let Self {
            lights,
            cheap_lights,
            light_passes: _,
            flares,
            flare_pass: _,
            camo: _,
            shield: _,
            shadow_caster: _,
            tracers,
            color_grading: _,
            bounding_sphere: _,
            change_color_count: _,
        } = self;
        visit("h4.lights", lights);
        visit("h4.cheap_lights", cheap_lights);
        visit("h4.flares", flares);
        visit("h4.tracers", tracers);
    }
}

/// [`Feature`] for kinds that hang under a region through [`Attached`]: `$moves`
/// and `$inputs` are each kind's rule, `|at|` and `|at, object|`.
///
/// [`Attached`]: super::attached::Attached
macro_rules! attached_feature {
    ($kind:ty, moves: |$at:ident| $moves:expr, inputs: |$item:ident| $inputs:expr) => {
        impl Feature for $kind {
            fn only_under(&self) -> Option<&(String, String)> {
                super::attached::Attached::only_under(self)
            }

            fn moves(&self) -> bool {
                let $at = self;
                $moves
            }

            fn inputs(&self, object: u8) -> Vec<&str> {
                let $item = self;
                if $item.object == object { $inputs } else { Vec::new() }
            }
        }
    };
}

// Lights and flares move when they animate (`object_moves`), and read their
// scale and the functions their channels name (`h2_shader_inputs`).
attached_feature!(super::super::h2::lights::H2FlareAt, moves: |at| at.animates(), inputs: |at| vec![at.scale_name()]);
attached_feature!(super::super::h2::lights::H2PointLight, moves: |at| at.animates(), inputs: |at| vec![at.scale_name()]);
attached_feature!(super::super::h2::lights::H2GelLight, moves: |at| at.animates(), inputs: |at| vec![at.scale_name()]);
attached_feature!(super::super::h3::lights::H3Light, moves: |at| at.animates(), inputs: |at| at.inputs().collect());
attached_feature!(super::super::reach::lights::ReachLight, moves: |at| at.animates(), inputs: |at| at.inputs().collect());
attached_feature!(super::super::reach::lights::ReachCheapLight, moves: |at| at.animates(), inputs: |at| at.inputs().collect());
attached_feature!(super::super::h4::lights::H4Light, moves: |at| at.animates(), inputs: |at| at.inputs().collect());
attached_feature!(super::super::h4::lights::H4CheapLight, moves: |at| at.animates(), inputs: |at| at.inputs().collect());
attached_feature!(super::super::h4::flares::H4FlareAt, moves: |at| at.animates(), inputs: |at| at.inputs().collect());
attached_feature!(super::super::gen3::flares::H3FlareAt, moves: |at| at.animates(), inputs: |at| vec![at.scale_name()]);
attached_feature!(super::super::gen3::light_volumes::H3LightVolumeAt, moves: |at| at.animates(), inputs: |at| at.scale_names().into_iter().collect());
// Halo 2's light volumes draw still at rest.
attached_feature!(super::super::h2::light_volumes::H2LightVolume, moves: |_at| false, inputs: |at| vec![at.scale_name()]);
// Simulations run on whenever there are any.
attached_feature!(super::super::h4::tracers::H4TracerAt, moves: |_at| true, inputs: |at| at.scale_names().into_iter().collect());
attached_feature!(super::super::particles::H3ParticleSystem, moves: |_at| true, inputs: |at| at.scale_names().into_iter().collect());
attached_feature!(super::super::particles::cheap::ReachCheapEmitterAt, moves: |_at| true, inputs: |at| at.scale_names().into_iter().collect());
attached_feature!(super::super::gen3::strips::H3StripAt, moves: |_at| true, inputs: |at| at.scale_names().into_iter().collect());
attached_feature!(super::super::h2::contrails::H2ContrailAt, moves: |_at| true, inputs: |at| vec![at.scale_name()]);
attached_feature!(super::super::h2::liquid::H2LiquidAt, moves: |_at| true, inputs: |at| vec![at.scale_name()]);

impl Feature for super::super::antennas::Antenna {
    fn only_under(&self) -> Option<&(String, String)> {
        super::attached::Attached::only_under(self)
    }
}

impl Feature for super::super::gen3::screen_effects::H3ScreenEffectAt {
    fn moves(&self) -> bool {
        true
    }

    fn inputs(&self, object: u8) -> Vec<&str> {
        if self.object != object {
            return Vec::new();
        }
        std::iter::once(self.scale_name()).chain(self.object_inputs()).collect()
    }
}

impl Feature for super::super::cloth::H3ClothAt {
    fn moves(&self) -> bool {
        true
    }
}

impl Feature for super::super::h3::leaves::H3LeavesAt {
    fn moves(&self) -> bool {
        true
    }
}

// Halo CE's read no Halo 2 function names; an effect runs at rest only when
// no event plays it.
impl Feature for super::super::ce::lights::CeLight {}
impl Feature for super::super::antennas::ce::CeAntenna {}
impl Feature for super::super::ce::widgets::CeLightVolume {}

impl Feature for super::super::ce::particles::CeEffectAt {
    fn moves(&self) -> bool {
        self.event.is_none()
    }
}

impl Feature for super::super::ce::contrails::CeContrailAt {
    fn moves(&self) -> bool {
        true
    }
}

impl Feature for super::super::ce::widgets::CeLightning {
    fn moves(&self) -> bool {
        true
    }
}

impl Feature for super::super::ce::widgets::CeGlow {
    fn moves(&self) -> bool {
        true
    }
}

impl Feature for super::super::ce::flags::CeFlagAt {
    fn moves(&self) -> bool {
        true
    }
}

impl Feature for super::super::ce::particle_systems::CeParticleSystemAt {
    fn moves(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::app::model_preview::renderer::{h2_shader_inputs, has_effects, object_moves};
    use crate::app::model_preview::{ModelPreviewData, functions::FunctionInputs};

    fn paths(group: &dyn Features) -> Vec<&'static str> {
        let mut found = Vec::new();
        group.each(&mut |path, _| found.push(path));
        found
    }

    #[test]
    fn each_group_lists_its_features_by_their_gate_paths() {
        let preview = RenderModelPreview::default();
        assert_eq!(
            paths(&preview.ce),
            [
                "ce.lights",
                "ce.antennas",
                "ce.effects",
                "ce.contrails",
                "ce.light_volumes",
                "ce.lightning",
                "ce.glows",
                "ce.flags",
                "ce.particle_systems",
            ]
        );
        assert_eq!(
            paths(&preview.h2),
            ["h2.flares", "h2.point_lights", "h2.gel_lights", "h2.light_volumes", "h2.contrails", "h2.liquids"]
        );
        assert_eq!(
            paths(&preview.gen3),
            ["gen3.cheap_emitters", "gen3.flares", "gen3.light_volumes", "gen3.strips", "gen3.screen_effects"]
        );
        assert_eq!(paths(&preview.h3), ["h3.lights", "h3.leaves"]);
        assert_eq!(paths(&preview.reach), ["reach.lights", "reach.cheap_lights"]);
        assert_eq!(paths(&preview.h4), ["h4.lights", "h4.cheap_lights", "h4.flares", "h4.tracers"]);
        let mut all = Vec::new();
        preview.each_feature(&mut |path, _| all.push(path));
        assert_eq!(all.len(), 3 + 6 + 5 + 2 + 2 + 4 + 9);
        assert_eq!(all[..3], ["particles", "antennas", "cloth"]);
    }

    /// Every object the load gate loads, by game.
    fn gate_objects() -> Vec<(String, ModelPreviewData)> {
        let mut loaded = Vec::new();
        for (variable, game, objects) in crate::app::model_preview::gates::LOAD_SET {
            let Ok(root) = std::env::var(variable) else {
                continue;
            };
            for (path, extension, group) in *objects {
                let data = crate::app::model_preview::load::kit_object_tests::kit_preview(&root, *game, path, extension, group);
                loaded.push((format!("{game:?} {path}"), data));
            }
        }
        loaded
    }

    /// `data` with everything but its features taken out, so today's central
    /// functions answer for the features alone.
    fn features_only(data: &ModelPreviewData) -> ModelPreviewData {
        let mut data = data.clone();
        let mut preview = (*data.preview).clone();
        preview.materials.clear();
        preview.reach.shield = None;
        preview.h4.shield = None;
        data.preview = Arc::new(preview);
        data.textures = None;
        data.object_functions = None;
        data.ce_object_functions = None;
        data.attached.clear();
        data.ce_attached.clear();
        data
    }

    #[test]
    fn the_lists_answer_as_todays_central_functions_do() {
        let loaded = gate_objects();
        if loaded.is_empty() {
            eprintln!("skipping: no kit variables set");
            return;
        }
        for (name, data) in &loaded {
            let preview = &data.preview;
            assert_eq!(preview.any_feature(|list| !list.is_empty()), has_effects(preview), "{name}: has_effects");
            let alone = features_only(data);
            let moves = object_moves(&alone, &FunctionInputs::default(), 0.0, None);
            assert_eq!(preview.any_feature(|list| list.moves()), moves, "{name}: object_moves");
            for object in 0..=8 {
                let today: BTreeSet<String> = h2_shader_inputs(&alone, object).into_iter().collect();
                let mut listed = BTreeSet::new();
                preview.each_feature(&mut |_, list| listed.extend(list.inputs_of(object).into_iter().map(str::to_owned)));
                assert_eq!(listed, today, "{name}: h2_shader_inputs for object {object}");
            }
        }
    }

    #[test]
    fn the_lists_show_under_the_regions_as_todays_shown_lists_do() {
        use crate::app::model_preview::renderer::H2Shown;
        use crate::app::model_preview::state::ModelPreviewState;
        let loaded = gate_objects();
        if loaded.is_empty() {
            eprintln!("skipping: no kit variables set");
            return;
        }
        let state = ModelPreviewState::default();
        for (name, data) in &loaded {
            let preview = &data.preview;
            let today = H2Shown::of(preview, &state);
            let selections = &state.region_selections;
            assert_eq!(preview.h2.point_lights.shown(selections), today.points, "{name}: points");
            assert_eq!(preview.h2.gel_lights.shown(selections), today.gels, "{name}: gels");
            assert_eq!(preview.h2.light_volumes.shown(selections), today.volumes, "{name}: volumes");
            assert_eq!(preview.h2.contrails.shown(selections), today.contrails, "{name}: contrails");
            assert_eq!(preview.h2.liquids.shown(selections), today.liquids, "{name}: liquids");
        }
    }
}
