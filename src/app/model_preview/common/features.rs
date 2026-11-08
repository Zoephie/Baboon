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

    /// Names it reads on `object` that the sliders offer only after every one
    /// of its kind has given its [`inputs`](Feature::inputs).
    fn late_inputs(&self, _object: u8) -> Vec<&str> {
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

    fn late_inputs(&self, object: u8) -> Vec<&str> {
        T::late_inputs(self, object)
    }
}

/// A preview field of one kind of [`Feature`], asked about as a whole.
pub(in crate::app::model_preview) trait FeatureList {
    /// Whether it holds none.
    fn is_empty(&self) -> bool;

    /// Whether any it holds moves.
    fn moves(&self) -> bool;

    /// The function names those on `object` read, in order (each one's
    /// inputs, then each one's late inputs), empty names left out.
    fn inputs_of(&self, object: u8) -> Vec<&str>;

    /// Whether each shows under the chosen regions.
    fn shown(&self, selections: &HashMap<String, ModelRegionSelection>) -> Vec<bool>;

    /// Every one taken out.
    fn clear(&mut self);
}

fn inputs_of<'a, T: Feature + 'a>(items: impl Iterator<Item = &'a T> + Clone, object: u8) -> Vec<&'a str> {
    let early = items.clone().flat_map(|item| item.inputs(object));
    early.chain(items.flat_map(|item| item.late_inputs(object))).filter(|name| !name.is_empty()).collect()
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

    fn clear(&mut self) {
        Vec::clear(self);
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

    fn clear(&mut self) {
        *self = None;
    }
}

/// Which of each feature show under the chosen regions, for one frame, by the
/// feature's path: `object_get_markers_by_string_id` finds only the markers of
/// the permutations the object shows, so what hangs on the others is not made.
#[derive(Debug, Clone, Default)]
pub(in crate::app::model_preview) struct FeatureShown {
    lists: Vec<(&'static str, Vec<bool>)>,
}

impl FeatureShown {
    /// Each feature of `preview`, under `selections`.
    pub(in crate::app::model_preview) fn of(
        preview: &RenderModelPreview,
        selections: &HashMap<String, ModelRegionSelection>,
    ) -> Self {
        let mut lists = Vec::new();
        preview.each_feature(&mut |path, list| lists.push((path, list.shown(selections))));
        Self { lists }
    }

    /// Whether each of the feature at `path` shows; empty for a path it
    /// doesn't have.
    pub(in crate::app::model_preview) fn list(&self, path: &str) -> &[bool] {
        self.lists.iter().find(|(at, _)| *at == path).map_or(&[], |(_, list)| list.as_slice())
    }

    /// Whether the `index`th of the feature at `path` shows: true for one it
    /// doesn't know, as nothing hides it.
    pub(in crate::app::model_preview) fn is(&self, path: &str, index: usize) -> bool {
        self.list(path).get(index).copied().unwrap_or(true)
    }
}

/// The paths the drawing code asks [`FeatureShown`] about, one constant each,
/// so no call names a path by hand.
pub(in crate::app::model_preview) mod paths {
    /// Each constant, and [`ALL`] of them, from one list.
    macro_rules! paths {
        ($($(#[$doc:meta])* $name:ident => $path:literal),* $(,)?) => {
            $($(#[$doc])* pub(in crate::app::model_preview) const $name: &str = $path;)*
            /// Every path above.
            #[cfg(test)]
            pub(in crate::app::model_preview) const ALL: &[&str] = &[$($name),*];
        };
    }

    paths! {
        PARTICLES => "particles",
        ANTENNAS => "antennas",
        H2_FLARES => "h2.flares",
        H2_POINT_LIGHTS => "h2.point_lights",
        H2_GEL_LIGHTS => "h2.gel_lights",
        H2_LIGHT_VOLUMES => "h2.light_volumes",
        H2_CONTRAILS => "h2.contrails",
        H2_LIQUIDS => "h2.liquids",
        GEN3_CHEAP_EMITTERS => "gen3.cheap_emitters",
        GEN3_FLARES => "gen3.flares",
        GEN3_LIGHT_VOLUMES => "gen3.light_volumes",
        GEN3_STRIPS => "gen3.strips",
        H3_LIGHTS => "h3.lights",
        REACH_LIGHTS => "reach.lights",
        REACH_CHEAP_LIGHTS => "reach.cheap_lights",
        H4_LIGHTS => "h4.lights",
        H4_CHEAP_LIGHTS => "h4.cheap_lights",
        H4_FLARES => "h4.flares",
        H4_TRACERS => "h4.tracers",
    }
}

/// A group of a preview's features: each field that is one, by its path as
/// the load gate names it after `preview.`.
pub(in crate::app::model_preview) trait Features {
    fn each(&self, visit: &mut dyn FnMut(&'static str, &dyn FeatureList));

    /// [`each`](Features::each), lending each field to change.
    fn each_mut(&mut self, visit: &mut dyn FnMut(&'static str, &mut dyn FeatureList));
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

    /// The names the feature at `path` reads on `object`, as
    /// [`FeatureList::inputs_of`] gives them; none for a path it doesn't have.
    pub(in crate::app::model_preview) fn feature_inputs(&self, path: &str, object: u8) -> Vec<String> {
        let mut names = Vec::new();
        self.each_feature(&mut |at, list| {
            if at == path {
                names.extend(list.inputs_of(object).into_iter().map(str::to_owned));
            }
        });
        names
    }

    /// [`each_feature`](Self::each_feature), lending each to change.
    pub(in crate::app::model_preview) fn each_feature_mut(&mut self, visit: &mut dyn FnMut(&'static str, &mut dyn FeatureList)) {
        visit("particles", &mut self.particles);
        visit("antennas", &mut self.antennas);
        visit("cloth", &mut self.cloth);
        self.h2.each_mut(visit);
        self.gen3.each_mut(visit);
        self.h3.each_mut(visit);
        self.reach.each_mut(visit);
        self.h4.each_mut(visit);
        self.ce.each_mut(visit);
    }

    /// The feature at `path` taken out; false when the preview has none there.
    pub(in crate::app::model_preview) fn clear_feature(&mut self, path: &str) -> bool {
        let mut found = false;
        self.each_feature_mut(&mut |at, list| {
            if at == path {
                list.clear();
                found = true;
            }
        });
        found
    }

    /// Whether the feature at `path` holds none (true for a path it doesn't have).
    pub(in crate::app::model_preview) fn feature_is_empty(&self, path: &str) -> bool {
        let mut empty = true;
        self.each_feature(&mut |at, list| {
            if at == path {
                empty = list.is_empty();
            }
        });
        empty
    }

    /// Whether any feature answers `test`.
    pub(in crate::app::model_preview) fn any_feature(&self, mut test: impl FnMut(&dyn FeatureList) -> bool) -> bool {
        let mut any = false;
        self.each_feature(&mut |_, list| any = any || test(list));
        any
    }
}

/// `Features` for a group: `each` and `each_mut` from one list of its feature
/// fields by path, the rest named as skipped, so the struct is taken apart
/// whole and a new field must be listed or skipped.
macro_rules! features {
    ($group:path { $($field:ident => $path:literal),* $(,)? ; skip $($skipped:ident),* $(,)? }) => {
        impl Features for $group {
            fn each(&self, visit: &mut dyn FnMut(&'static str, &dyn FeatureList)) {
                let Self { $($field,)* $($skipped: _,)* } = self;
                $(visit($path, $field);)*
            }

            fn each_mut(&mut self, visit: &mut dyn FnMut(&'static str, &mut dyn FeatureList)) {
                let Self { $($field,)* $($skipped: _,)* } = self;
                $(visit($path, $field);)*
            }
        }
    };
}

features!(super::super::ce::CeFeatures {
    lights => "ce.lights",
    antennas => "ce.antennas",
    effects => "ce.effects",
    contrails => "ce.contrails",
    light_volumes => "ce.light_volumes",
    lightning => "ce.lightning",
    glows => "ce.glows",
    flags => "ce.flags",
    particle_systems => "ce.particle_systems";
    skip firing
});

features!(super::super::h2::H2Features {
    flares => "h2.flares",
    point_lights => "h2.point_lights",
    gel_lights => "h2.gel_lights",
    light_volumes => "h2.light_volumes",
    contrails => "h2.contrails",
    liquids => "h2.liquids";
    skip prt, hologram_control
});

features!(super::super::gen3::Gen3Features {
    cheap_emitters => "gen3.cheap_emitters",
    flares => "gen3.flares",
    light_volumes => "gen3.light_volumes",
    strips => "gen3.strips",
    screen_effects => "gen3.screen_effects";
    skip prt, ik_markers
});

features!(super::super::h3::H3Features {
    lights => "h3.lights",
    leaves => "h3.leaves";
    skip shield, vision, motion_blur
});

features!(super::super::reach::ReachFeatures {
    lights => "reach.lights",
    cheap_lights => "reach.cheap_lights";
    skip light_passes, shield, camo, shadow_caster, screen_noise
});

features!(super::super::h4::H4Features {
    lights => "h4.lights",
    cheap_lights => "h4.cheap_lights",
    flares => "h4.flares",
    tracers => "h4.tracers";
    skip light_passes, flare_pass, camo, shield, shadow_caster, color_grading, bounding_sphere, change_color_count
});

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
    /// An event's effect runs only when played, and asks for frames then.
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
    use crate::app::model_preview::renderer::h2_shader_inputs;
    use crate::app::model_preview::ModelPreviewData;

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
            assert_eq!(preview.any_feature(|list| !list.is_empty()), has_effects_as_it_was(preview), "{name}: has_effects");
            let alone = features_only(data);
            assert_eq!(preview.any_feature(|list| list.moves()), feature_moves_as_it_was(preview), "{name}: object_moves");
            for object in 0..=8 {
                for (what, data) in [("whole", data), ("features alone", &alone)] {
                    let before = h2_shader_inputs_as_it_was(data, object);
                    assert_eq!(h2_shader_inputs(data, object), before, "{name} ({what}): h2_shader_inputs for object {object}");
                }
            }
        }
    }

    /// A stand-in kind with early and late inputs.
    struct Named(&'static str, &'static str);

    impl Feature for Named {
        fn inputs(&self, _object: u8) -> Vec<&str> {
            vec![self.0]
        }

        fn late_inputs(&self, _object: u8) -> Vec<&str> {
            vec![self.1]
        }
    }

    #[test]
    fn a_list_offers_every_ones_inputs_before_any_ones_late_inputs() {
        let list = vec![Named("scale a", "object a"), Named("scale b", ""), Named("", "object c")];
        assert_eq!(list.inputs_of(0), ["scale a", "scale b", "object a", "object c"]);
    }

    #[test]
    fn a_feature_is_cleared_by_its_path_and_both_walks_agree() {
        let mut reading = Vec::new();
        RenderModelPreview::default().each_feature(&mut |path, _| reading.push(path));
        let mut changing = Vec::new();
        let mut preview = RenderModelPreview::default();
        preview.each_feature_mut(&mut |path, _| changing.push(path));
        assert_eq!(reading, changing);
        assert!(!preview.clear_feature("h2.prt"), "not a feature");
        for (name, data) in &gate_objects() {
            let mut filled = Vec::new();
            data.preview.each_feature(&mut |path, list| {
                if !list.is_empty() {
                    filled.push(path);
                }
            });
            for path in filled {
                let mut preview = (*data.preview).clone();
                assert!(preview.clear_feature(path), "{name}: {path}");
                preview.each_feature(&mut |at, list| {
                    assert_eq!(list.is_empty(), at == path || data.preview.feature_is_empty(at), "{name}: clearing {path} left {at}");
                });
            }
        }
    }

    #[test]
    fn every_feature_with_slider_inputs_is_a_slider_source_once() {
        use crate::app::model_preview::renderer::{SLIDER_SOURCES, SliderSource};
        let listed: Vec<&str> = SLIDER_SOURCES
            .iter()
            .filter_map(|source| match source {
                SliderSource::Feature(path) => Some(*path),
                SliderSource::Gen3Materials => None,
            })
            .collect();
        let unique: BTreeSet<&str> = listed.iter().copied().collect();
        assert_eq!(unique.len(), listed.len(), "a path listed twice");
        let mut paths = Vec::new();
        RenderModelPreview::default().each_feature(&mut |path, _| paths.push(path));
        for path in &listed {
            assert!(paths.contains(path), "{path} is no feature");
        }
        for (name, data) in &gate_objects() {
            for object in 0..=8 {
                data.preview.each_feature(&mut |path, list| {
                    let offers = !list.inputs_of(object).is_empty();
                    assert!(!offers || listed.contains(&path), "{name}: {path} offers inputs but is not a slider source");
                });
            }
        }
    }

    /// The region selections the shown lists are checked under: none chosen,
    /// then each region turned off, then each region on each of its
    /// permutations.
    fn selections_of(preview: &RenderModelPreview) -> Vec<HashMap<String, ModelRegionSelection>> {
        let mut all = vec![HashMap::new()];
        for region in &preview.regions {
            let off = ModelRegionSelection { enabled: false, permutation: String::new() };
            all.push(HashMap::from([(region.name.clone(), off)]));
            for permutation in &region.permutations {
                let on = ModelRegionSelection { enabled: true, permutation: permutation.clone() };
                all.push(HashMap::from([(region.name.clone(), on)]));
            }
        }
        all
    }

    #[test]
    fn the_shown_lists_answer_as_the_frame_lists_did() {
        use crate::app::model_preview::state::ModelPreviewState;
        let loaded = gate_objects();
        if loaded.is_empty() {
            eprintln!("skipping: no kit variables set");
            return;
        }
        let mut limited = 0;
        for (name, data) in &loaded {
            let preview = &data.preview;
            let mut visit = |only: Option<&(String, String)>| limited += usize::from(only.is_some());
            preview.gen3.flares.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.h4.flares.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.reach.lights.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.reach.cheap_lights.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.h4.lights.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.h4.cheap_lights.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.h3.lights.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.gen3.light_volumes.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.h4.tracers.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.particles.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.gen3.strips.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.gen3.cheap_emitters.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.h2.flares.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.h2.point_lights.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.h2.gel_lights.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.h2.light_volumes.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.h2.contrails.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.h2.liquids.iter().for_each(|at| visit(Feature::only_under(at)));
            preview.antennas.iter().for_each(|at| visit(Feature::only_under(at)));
            for selections in selections_of(preview) {
                let state = ModelPreviewState { region_selections: selections.clone(), ..Default::default() };
                let shown = FeatureShown::of(preview, &selections);
                let check = |path: &str, before: &[bool]| {
                    assert_eq!(shown.list(path), before, "{name}: {path} under {:?}", selections.keys().collect::<Vec<_>>());
                    for (index, was) in before.iter().enumerate() {
                        assert_eq!(shown.is(path, index), *was, "{name}: {path} {index}");
                    }
                    assert!(shown.is(path, before.len()), "{name}: {path} past the end shows");
                };
                let flares = h3_flares_shown_as_it_was(preview, &state);
                let gen3_flares = preview.gen3.flares.len();
                check(paths::GEN3_FLARES, &flares[..gen3_flares]);
                check(paths::H4_FLARES, &flares[gen3_flares..]);
                let lights = reach_lights_shown_as_it_was(preview, &state);
                let reach = preview.reach.lights.len();
                let reach_cheap = reach + preview.reach.cheap_lights.len();
                let h4 = reach_cheap + preview.h4.lights.len();
                check(paths::REACH_LIGHTS, &lights[..reach]);
                check(paths::REACH_CHEAP_LIGHTS, &lights[reach..reach_cheap]);
                check(paths::H4_LIGHTS, &lights[reach_cheap..h4]);
                check(paths::H4_CHEAP_LIGHTS, &lights[h4..]);
                check(paths::H3_LIGHTS, &h3_lights_shown_as_it_was(preview, &state));
                check(paths::GEN3_LIGHT_VOLUMES, &h3_volumes_shown_as_it_was(preview, &state));
                check(paths::H4_TRACERS, &h4_tracers_shown_as_it_was(preview, &state));
                check(paths::PARTICLES, &h3_systems_shown_as_it_was(preview, &state));
                check(paths::GEN3_STRIPS, &h3_strips_shown_as_it_was(preview, &state));
                check(paths::GEN3_CHEAP_EMITTERS, &reach_cheap_shown_as_it_was(preview, &state));
                check(paths::H2_FLARES, &h2_flares_shown_as_it_was(preview, &state));
                let h2 = H2ShownAsItWas::of(preview, &state);
                check(paths::H2_POINT_LIGHTS, &h2.points);
                check(paths::H2_GEL_LIGHTS, &h2.gels);
                check(paths::H2_LIGHT_VOLUMES, &h2.volumes);
                check(paths::H2_CONTRAILS, &h2.contrails);
                check(paths::H2_LIQUIDS, &h2.liquids);
                check(paths::ANTENNAS, &h2.antennas);
            }
        }
        eprintln!("shown lists: {limited} items under a region, over {} objects", loaded.len());
        assert!(limited > 0, "no item hangs under a region: the test would be vacuous");
    }

    #[test]
    fn every_path_asked_about_is_a_feature_and_none_is_spelled_by_hand() {
        let mut visited = Vec::new();
        RenderModelPreview::default().each_feature(&mut |path, _| visited.push(path));
        for path in paths::ALL {
            assert!(visited.contains(path), "{path} is no feature");
        }
        // The drawing code reads the frame's `shown` only through the
        // constants: no call of its `is` or `list` with a string literal.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/model_preview");
        let mut stack = vec![root];
        while let Some(folder) = stack.pop() {
            for entry in std::fs::read_dir(&folder).expect("the source folder") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    let text = std::fs::read_to_string(&path).expect("a source file");
                    // Built in pieces, so this file's own text holds neither.
                    for spelled in [concat!("shown", ".is(\""), concat!("shown", ".list(\"")] {
                        assert!(!text.contains(spelled), "{}: a path spelled by hand", path.display());
                    }
                }
            }
        }
    }

    /// The frame's shown lists as they were before `FeatureShown`, kept as
    /// the oracles it is checked against.
    mod as_it_was {
        use crate::app::model_preview::RenderModelPreview;
        use crate::app::model_preview::common::attached::Attached;
        use crate::app::model_preview::renderer::marker_active;
        use crate::app::model_preview::state::ModelPreviewState;

        /// Which of the preview's Halo 3 flares have a marker under what is selected.
        pub(super) fn h3_flares_shown_as_it_was(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
            let shown = |only| marker_active(only, &state.region_selections);
            let h4 = preview.h4.flares.iter().map(|at| at.only_under().is_none_or(shown));
            preview.gen3.flares.iter().map(|at| at.only_under().is_none_or(shown)).chain(h4).collect()
        }

        /// The same of Reach's lights and then its cheap lights, then Halo 4's.
        pub(super) fn reach_lights_shown_as_it_was(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
            let shown = |only| marker_active(only, &state.region_selections);
            let lights = preview.reach.lights.iter().map(|light| light.only_under().is_none_or(shown));
            let cheap = preview.reach.cheap_lights.iter().map(|light| light.only_under().is_none_or(shown));
            let h4 = preview.h4.lights.iter().map(|light| light.only_under().is_none_or(shown));
            let h4_cheap = preview.h4.cheap_lights.iter().map(|light| light.only_under().is_none_or(shown));
            lights.chain(cheap).chain(h4).chain(h4_cheap).collect()
        }

        /// The same of Halo 3's lights.
        pub(super) fn h3_lights_shown_as_it_was(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
            let shown = |only| marker_active(only, &state.region_selections);
            preview.h3.lights.iter().map(|light| light.only_under().is_none_or(shown)).collect()
        }

        /// The same of Halo 3's light volumes.
        pub(super) fn h3_volumes_shown_as_it_was(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
            let shown = |only| marker_active(only, &state.region_selections);
            preview.gen3.light_volumes.iter().map(|at| at.only_under().is_none_or(shown)).collect()
        }

        /// The same of Halo 4's tracers.
        pub(super) fn h4_tracers_shown_as_it_was(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
            let shown = |only| marker_active(only, &state.region_selections);
            preview.h4.tracers.iter().map(|at| at.only_under().is_none_or(shown)).collect()
        }

        /// The same of Reach's cheap particle emitters.
        pub(super) fn reach_cheap_shown_as_it_was(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
            let shown = |only| marker_active(only, &state.region_selections);
            preview.gen3.cheap_emitters.iter().map(|at| at.only_under().is_none_or(shown)).collect()
        }

        /// The same of Halo 3's beams and contrails.
        pub(super) fn h3_strips_shown_as_it_was(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
            let shown = |only| marker_active(only, &state.region_selections);
            preview.gen3.strips.iter().map(|at| at.only_under().is_none_or(shown)).collect()
        }

        /// The same of Halo 3's particle systems.
        pub(super) fn h3_systems_shown_as_it_was(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
            let shown = |only| marker_active(only, &state.region_selections);
            preview.particles.iter().map(|at| at.only_under().is_none_or(shown)).collect()
        }

        /// The same of Halo 2's.
        pub(super) fn h2_flares_shown_as_it_was(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
            let shown = |only| marker_active(only, &state.region_selections);
            preview.h2.flares.iter().map(|at| at.only_under().is_none_or(shown)).collect()
        }

        /// Which of Halo 2's other lights and effects have a marker under what is
        /// selected: `object_get_markers_by_string_id` finds only those of the
        /// permutations the object shows, so what hangs on the others is not made.
        pub(super) struct H2ShownAsItWas {
            pub(super) points: Vec<bool>,
            pub(super) gels: Vec<bool>,
            pub(super) volumes: Vec<bool>,
            pub(super) contrails: Vec<bool>,
            pub(super) liquids: Vec<bool>,
            pub(super) antennas: Vec<bool>,
        }

        impl H2ShownAsItWas {
            pub(super) fn of(preview: &RenderModelPreview, state: &ModelPreviewState) -> Self {
                let shown = |only: Option<&(String, String)>| only.is_none_or(|only| marker_active(only, &state.region_selections));
                Self {
                    points: preview.h2.point_lights.iter().map(|at| shown(at.only_under())).collect(),
                    gels: preview.h2.gel_lights.iter().map(|at| shown(at.only_under())).collect(),
                    volumes: preview.h2.light_volumes.iter().map(|at| shown(at.only_under())).collect(),
                    contrails: preview.h2.contrails.iter().map(|at| shown(at.only_under())).collect(),
                    liquids: preview.h2.liquids.iter().map(|at| shown(at.only_under())).collect(),
                    antennas: preview.antennas.iter().map(|at| shown(at.only_under())).collect(),
                }
            }
        }
    }

    use as_it_was::*;

    /// `has_effects` as it was before the feature lists, field by field, kept
    /// for one commit as the oracle the loop is checked against.
    fn has_effects_as_it_was(preview: &RenderModelPreview) -> bool {
        !(preview.ce.lights.is_empty()
            && preview.ce.antennas.is_empty()
            && preview.ce.effects.is_empty()
            && preview.ce.contrails.is_empty()
            && preview.ce.light_volumes.is_empty()
            && preview.ce.lightning.is_empty()
            && preview.ce.glows.is_empty()
            && preview.ce.particle_systems.is_empty()
            && preview.ce.flags.is_empty()
            && preview.antennas.is_empty()
            && preview.h2.flares.is_empty()
            && preview.h2.point_lights.is_empty()
            && preview.h2.gel_lights.is_empty()
            && preview.h2.light_volumes.is_empty()
            && preview.h2.contrails.is_empty()
            && preview.h2.liquids.is_empty()
            && preview.h3.lights.is_empty()
            && preview.reach.lights.is_empty()
            && preview.reach.cheap_lights.is_empty()
            && preview.h4.lights.is_empty()
            && preview.h4.cheap_lights.is_empty()
            && preview.h4.flares.is_empty()
            && preview.gen3.flares.is_empty()
            && preview.gen3.light_volumes.is_empty()
            && preview.h4.tracers.is_empty()
            && preview.particles.is_empty()
            && preview.gen3.cheap_emitters.is_empty()
            && preview.gen3.strips.is_empty()
            && preview.h3.leaves.is_empty()
            && preview.cloth.is_empty()
            && preview.gen3.screen_effects.is_empty())
    }

    /// The feature part of `object_moves` as it was before the feature lists,
    /// kept as the oracle the loop is checked against.
    fn feature_moves_as_it_was(preview: &RenderModelPreview) -> bool {
        preview.h2.flares.iter().any(|at| at.animates())
            || preview.h2.point_lights.iter().any(|light| light.animates())
            || preview.h2.gel_lights.iter().any(|light| light.animates())
            || preview.h3.lights.iter().any(|light| light.animates())
            || preview.reach.lights.iter().any(|light| light.animates())
            || preview.reach.cheap_lights.iter().any(|light| light.animates())
            || preview.h4.lights.iter().any(|light| light.animates())
            || preview.h4.cheap_lights.iter().any(|light| light.animates())
            || preview.h4.flares.iter().any(|at| at.animates())
            || preview.gen3.flares.iter().any(|at| at.animates())
            || preview.gen3.light_volumes.iter().any(|at| at.animates())
            || !preview.h4.tracers.is_empty()
            || !preview.particles.is_empty()
            || !preview.gen3.cheap_emitters.is_empty()
            || !preview.gen3.strips.is_empty()
            || !preview.gen3.screen_effects.is_empty()
            || !preview.cloth.is_empty()
            || !preview.h2.liquids.is_empty()
            || !preview.h2.contrails.is_empty()
            // An event's effect runs only when played, and asks for frames then.
            || preview.ce.effects.iter().any(|effect| effect.event.is_none())
            || !preview.ce.contrails.is_empty()
            || !preview.ce.lightning.is_empty()
            || !preview.ce.glows.is_empty()
            || !preview.ce.particle_systems.is_empty()
            || !preview.ce.flags.is_empty()
            || !preview.h3.leaves.is_empty()
    }

    /// `h2_shader_inputs` as it was before the feature lists, kept as the
    /// oracle the loop is checked against, order and all.
    fn h2_shader_inputs_as_it_was(data: &ModelPreviewData, object: u8) -> Vec<String> {
        use crate::app::model_preview::dx9::passes::H2Layer;
        use crate::app::model_preview::gen3;
        use crate::app::model_preview::materials::MaterialTextures;
        let mut names: Vec<String> = Vec::new();
        let textures = data.textures.iter().flat_map(|textures| textures.iter());
        let owned: Vec<&MaterialTextures> = data
            .preview
            .materials
            .iter()
            .zip(textures)
            .filter(|(material, _)| material.object == object)
            .map(|(_, texture)| texture)
            .collect();
        let passes = owned.iter().flat_map(|texture| texture.h2_layers.iter().flatten());
        let flares = data.preview.h2.flares.iter().filter(|at| at.object == object).map(|at| at.scale_name());
        let lights = data.preview.h2.point_lights.iter().filter(|light| light.object == object);
        let h3_lights = data.preview.h3.lights.iter().filter(|light| light.object == object);
        let reach_lights = data.preview.reach.lights.iter().filter(|light| light.object == object);
        let reach_cheap = data.preview.reach.cheap_lights.iter().filter(|light| light.object == object);
        let h4_lights = data.preview.h4.lights.iter().filter(|light| light.object == object);
        let h4_cheap = data.preview.h4.cheap_lights.iter().filter(|light| light.object == object);
        let h4_flares = data.preview.h4.flares.iter().filter(|at| at.object == object);
        let h3_flares = data.preview.gen3.flares.iter().filter(|at| at.object == object);
        let h3_volumes = data.preview.gen3.light_volumes.iter().filter(|at| at.object == object);
        let h4_tracers = data.preview.h4.tracers.iter().filter(|at| at.object == object);
        let h3_systems = data.preview.particles.iter().filter(|at| at.object == object);
        let h3_strips = data.preview.gen3.strips.iter().filter(|at| at.object == object);
        let reach_emitters = data.preview.gen3.cheap_emitters.iter().filter(|at| at.object == object);
        let h3_screens = data.preview.gen3.screen_effects.iter().filter(|at| at.object == object);
        let gels = data.preview.h2.gel_lights.iter().filter(|light| light.object == object);
        let h2_volumes = data.preview.h2.light_volumes.iter().filter(|at| at.object == object).map(|at| at.scale_name());
        let h2_contrails = data.preview.h2.contrails.iter().filter(|at| at.object == object).map(|at| at.scale_name());
        let h2_liquids = data.preview.h2.liquids.iter().filter(|at| at.object == object).map(|at| at.scale_name());
        let h3 = owned.iter().filter_map(|texture| texture.h3.as_ref()).flat_map(|material| &material.animations);
        let h3 = h3.flat_map(|animation| [animation.input.as_str(), animation.range.as_str()]);
        let h4 = owned.iter().filter_map(|texture| texture.h3.as_ref()).filter_map(|material| material.h4());
        // A hologram's screen materials read its object's functions too.
        let post = h4.clone().filter_map(|submit| submit.post());
        let post = post.flat_map(|post| post.combine.submit.inputs().chain(post.composite.submit.inputs()));
        let h3 = h3.chain(h4.flat_map(|submit| submit.inputs())).chain(post);
        let flares = flares
            .chain(h3)
            .chain(h3_lights.flat_map(|light| light.inputs()))
            .chain(reach_lights.flat_map(|light| light.inputs()))
            .chain(reach_cheap.flat_map(|light| light.inputs()))
            .chain(h4_lights.flat_map(|light| light.inputs()))
            .chain(h4_cheap.flat_map(|light| light.inputs()))
            .chain(h4_flares.flat_map(|at| at.inputs()))
            .chain(h3_flares.map(|at| at.scale_name()))
            .chain(h3_volumes.flat_map(|at| at.scale_names()))
            .chain(h4_tracers.flat_map(|at| at.scale_names()))
            .chain(h3_systems.flat_map(|at| at.scale_names()))
            .chain(h3_strips.flat_map(|at| at.scale_names()))
            .chain(reach_emitters.flat_map(|at| at.scale_names()))
            .chain(h3_screens.clone().map(|at| at.scale_name()))
            .chain(h3_screens.flat_map(|at| at.object_inputs()))
            .chain(lights.map(|light| light.scale_name()))
            .chain(gels.map(|light| light.scale_name()))
            .chain(h2_volumes)
            .chain(h2_contrails)
            .chain(h2_liquids)
            .filter(|name| !name.is_empty());
        // `sub_6C18F0` gates the shield overlay on this itself.
        let h3_shield = owned.iter().filter_map(|texture| texture.h3.as_ref()).any(|h3| h3.shield.is_some());
        let shield = data.preview.materials.iter().any(|m| m.object == object && m.shield == Some(H2Layer::ShieldDamaged));
        let reach_shield = object == 0 && (data.preview.reach.shield.is_some() || data.preview.h4.shield.is_some());
        let gate = (shield || h3_shield || reach_shield).then_some("current_shield_damage");
        let gate = gate.into_iter().chain(h3_shield.then_some(gen3::pipeline::H3_OVERSHIELD));
        for name in passes.flat_map(|pass| pass.function_inputs()).chain(flares).chain(gate) {
            if !names.iter().any(|known| known == name) {
                names.push(name.to_owned());
            }
        }
        names
    }
}
