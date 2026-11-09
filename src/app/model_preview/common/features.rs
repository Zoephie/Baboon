//! What a preview carries besides its geometry and materials: each kind of
//! light, flare, effect, widget and simulation an object brings, listed once
//! per game so the code that asks about all of them (whether anything draws,
//! whether the frame moves, which functions the panel offers, which show under
//! the chosen regions) asks the lists instead of naming every field.

use std::collections::HashMap;
use std::sync::Arc;

use super::super::ModelRegionSelection;
use super::super::RenderModelPreview;
use super::super::frame::marker_active;

/// One of the things a preview carries.
pub(in crate::app::model_preview) trait Feature {
    /// Whether it draws in the object's sprite pass, among the flares,
    /// antennas, effects and particles.
    const SPRITE: bool = false;

    /// Whether a function it names has the opened object's values worked
    /// out for it, though the object has no functions of its own.
    const NEEDS_VALUES: bool = false;

    /// Whether it names any function, on whichever object it hangs.
    fn names_a_function(&self) -> bool {
        false
    }

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
    const SPRITE: bool = T::SPRITE;
    const NEEDS_VALUES: bool = T::NEEDS_VALUES;

    fn names_a_function(&self) -> bool {
        T::names_a_function(self)
    }

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

    /// Whether it holds any that draw in the object's sprite pass.
    fn draws_sprites(&self) -> bool;

    /// Whether it holds any naming a function that needs the opened
    /// object's values ([`Feature::NEEDS_VALUES`]).
    fn needs_values(&self) -> bool;

    /// The function names those on `object` read, in order (each one's
    /// inputs, then each one's late inputs), empty names left out.
    fn inputs_of(&self, object: u8) -> Vec<&str>;

    /// Whether each shows under the chosen regions.
    fn shown(&self, selections: &HashMap<String, ModelRegionSelection>) -> Vec<bool>;

    /// Every one taken out, for the harness's `H2_SHOT_DROP`.
    #[cfg(test)]
    fn clear(&mut self);
}

fn inputs_of<'a, T: Feature + 'a>(
    items: impl Iterator<Item = &'a T> + Clone,
    object: u8,
) -> Vec<&'a str> {
    let early = items.clone().flat_map(|item| item.inputs(object));
    early
        .chain(items.flat_map(|item| item.late_inputs(object)))
        .filter(|name| !name.is_empty())
        .collect()
}

fn shown<'a, T: Feature + 'a>(
    items: impl Iterator<Item = &'a T>,
    selections: &HashMap<String, ModelRegionSelection>,
) -> Vec<bool> {
    items
        .map(|item| {
            item.only_under()
                .is_none_or(|only| marker_active(only, selections))
        })
        .collect()
}

impl<T: Feature> FeatureList for Vec<T> {
    fn is_empty(&self) -> bool {
        <[T]>::is_empty(self)
    }

    fn draws_sprites(&self) -> bool {
        T::SPRITE && !<[T]>::is_empty(self)
    }

    fn needs_values(&self) -> bool {
        T::NEEDS_VALUES && self.iter().any(Feature::names_a_function)
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

    #[cfg(test)]
    fn clear(&mut self) {
        Vec::clear(self);
    }
}

impl<T: Feature> FeatureList for Option<T> {
    fn is_empty(&self) -> bool {
        self.is_none()
    }

    fn draws_sprites(&self) -> bool {
        T::SPRITE && self.is_some()
    }

    fn needs_values(&self) -> bool {
        T::NEEDS_VALUES && self.iter().any(Feature::names_a_function)
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

    #[cfg(test)]
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
        self.lists
            .iter()
            .find(|(at, _)| *at == path)
            .map_or(&[], |(_, list)| list.as_slice())
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

    /// [`each`](Features::each), lending each field to change, for the harness.
    #[cfg(test)]
    fn each_mut(&mut self, visit: &mut dyn FnMut(&'static str, &mut dyn FeatureList));
}

impl RenderModelPreview {
    /// Every feature the preview carries, in its fields' order.
    pub(in crate::app::model_preview) fn each_feature(
        &self,
        visit: &mut dyn FnMut(&'static str, &dyn FeatureList),
    ) {
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
    pub(in crate::app::model_preview) fn feature_inputs(
        &self,
        path: &str,
        object: u8,
    ) -> Vec<String> {
        let mut names = Vec::new();
        self.each_feature(&mut |at, list| {
            if at == path {
                names.extend(list.inputs_of(object).into_iter().map(str::to_owned));
            }
        });
        names
    }

    /// [`each_feature`](Self::each_feature), lending each to change.
    #[cfg(test)]
    pub(in crate::app::model_preview) fn each_feature_mut(
        &mut self,
        visit: &mut dyn FnMut(&'static str, &mut dyn FeatureList),
    ) {
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
    #[cfg(test)]
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
    #[cfg(test)]
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
    pub(in crate::app::model_preview) fn any_feature(
        &self,
        mut test: impl FnMut(&dyn FeatureList) -> bool,
    ) -> bool {
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
        impl $crate::app::model_preview::common::features::Features for $group {
            fn each(&self, visit: &mut dyn FnMut(&'static str, &dyn $crate::app::model_preview::common::features::FeatureList)) {
                let Self { $($field,)* $($skipped: _,)* } = self;
                $(visit($path, $field);)*
            }

            #[cfg(test)]
            fn each_mut(&mut self, visit: &mut dyn FnMut(&'static str, &mut dyn $crate::app::model_preview::common::features::FeatureList)) {
                let Self { $($field,)* $($skipped: _,)* } = self;
                $(visit($path, $field);)*
            }
        }
    };
}

pub(in crate::app::model_preview) use features;

/// [`Feature`] for kinds that hang under a region through [`Attached`]: `$moves`
/// and `$inputs` are each kind's rule, `|at|` and `|at, object|`. A kind
/// that draws in the sprite pass is marked `sprite` first, and one whose
/// functions need the opened object's values `values` (see
/// [`Feature::NEEDS_VALUES`]).
///
/// [`Attached`]: super::attached::Attached
macro_rules! attached_feature {
    (sprite values $kind:ty, moves: |$at:ident| $moves:expr, inputs: |$item:ident| $inputs:expr) => {
        $crate::app::model_preview::common::features::attached_feature!(@impl true, true, $kind, moves: |$at| $moves, inputs: |$item| $inputs);
    };
    (values $kind:ty, moves: |$at:ident| $moves:expr, inputs: |$item:ident| $inputs:expr) => {
        $crate::app::model_preview::common::features::attached_feature!(@impl false, true, $kind, moves: |$at| $moves, inputs: |$item| $inputs);
    };
    (sprite $kind:ty, moves: |$at:ident| $moves:expr, inputs: |$item:ident| $inputs:expr) => {
        $crate::app::model_preview::common::features::attached_feature!(@impl true, false, $kind, moves: |$at| $moves, inputs: |$item| $inputs);
    };
    ($kind:ty, moves: |$at:ident| $moves:expr, inputs: |$item:ident| $inputs:expr) => {
        $crate::app::model_preview::common::features::attached_feature!(@impl false, false, $kind, moves: |$at| $moves, inputs: |$item| $inputs);
    };
    (@impl $sprite:literal, $values:literal, $kind:ty, moves: |$at:ident| $moves:expr, inputs: |$item:ident| $inputs:expr) => {
        impl $crate::app::model_preview::common::features::Feature for $kind {
            const SPRITE: bool = $sprite;
            const NEEDS_VALUES: bool = $values;

            fn names_a_function(&self) -> bool {
                let $item = self;
                let names: Vec<&str> = $inputs;
                names.iter().any(|name| !name.is_empty())
            }

            fn only_under(&self) -> Option<&(String, String)> {
                $crate::app::model_preview::common::attached::Attached::only_under(self)
            }

            fn moves(&self) -> bool {
                let $at = self;
                $moves
            }

            fn inputs(&self, object: u8) -> Vec<&str> {
                let $item = self;
                if $item.object == object {
                    $inputs
                } else {
                    Vec::new()
                }
            }
        }
    };
}

pub(in crate::app::model_preview) use attached_feature;

attached_feature!(sprite super::super::particles::ParticleSystem, moves: |_at| true, inputs: |at| at.scale_names().into_iter().collect());
attached_feature!(sprite super::super::particles::cheap::CheapEmitterAt, moves: |_at| true, inputs: |at| at.scale_names().into_iter().collect());

impl Feature for super::super::antennas::Antenna {
    const SPRITE: bool = true;

    fn only_under(&self) -> Option<&(String, String)> {
        super::attached::Attached::only_under(self)
    }
}

impl Feature for super::super::cloth::ClothAt {
    fn moves(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
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
            [
                "h2.flares",
                "h2.point_lights",
                "h2.gel_lights",
                "h2.light_volumes",
                "h2.contrails",
                "h2.liquids"
            ]
        );
        assert_eq!(
            paths(&preview.gen3),
            [
                "gen3.cheap_emitters",
                "gen3.flares",
                "gen3.light_volumes",
                "gen3.strips",
                "gen3.screen_effects"
            ]
        );
        assert_eq!(paths(&preview.h3), ["h3.lights", "h3.leaves"]);
        assert_eq!(
            paths(&preview.reach),
            ["reach.lights", "reach.cheap_lights"]
        );
        assert_eq!(
            paths(&preview.h4),
            ["h4.lights", "h4.cheap_lights", "h4.flares", "h4.tracers"]
        );
        let mut all = Vec::new();
        preview.each_feature(&mut |path, _| all.push(path));
        assert_eq!(all.len(), 3 + 6 + 5 + 2 + 2 + 4 + 9);
        assert_eq!(all[..3], ["particles", "antennas", "cloth"]);
    }

    /// Every object the load gate loads, by game.
    fn gate_objects() -> Vec<(String, ModelPreviewData)> {
        let mut loaded = Vec::new();
        for (_, game, objects) in crate::app::model_preview::gates::LOAD_SET {
            let Some(root) = crate::app::model_preview::harness::kit::kit_root(*game) else {
                continue;
            };
            for (path, extension, group) in *objects {
                let data = crate::app::model_preview::harness::kit::kit_preview(
                    &root, *game, path, extension, group,
                );
                loaded.push((format!("{game:?} {path}"), data));
            }
        }
        loaded
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
        let list = vec![
            Named("scale a", "object a"),
            Named("scale b", ""),
            Named("", "object c"),
        ];
        assert_eq!(
            list.inputs_of(0),
            ["scale a", "scale b", "object a", "object c"]
        );
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
                    assert_eq!(
                        list.is_empty(),
                        at == path || data.preview.feature_is_empty(at),
                        "{name}: clearing {path} left {at}"
                    );
                });
            }
        }
    }

    #[test]
    fn every_feature_with_slider_inputs_is_a_slider_source_once() {
        use crate::app::model_preview::frame::{SLIDER_SOURCES, SliderSource};
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
                    assert!(
                        !offers || listed.contains(&path),
                        "{name}: {path} offers inputs but is not a slider source"
                    );
                });
            }
        }
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
                        assert!(
                            !text.contains(spelled),
                            "{}: a path spelled by hand",
                            path.display()
                        );
                    }
                }
            }
        }
    }

    /// With `BLAM_TEST_HREK`: the marine's radio antenna is drawn only while
    /// its `trooper_radio` backpack is.
    #[test]
    fn kit_reach_antenna_shows_only_with_its_permutation() {
        use crate::app::model_preview::harness::kit::kit_root;
        use crate::app::model_preview::{ModelPreviewState, ModelRegionSelection};
        use crate::core::game::GameId;
        let Some(root) = kit_root(GameId::HaloReach) else {
            return;
        };
        let path = "objects\\characters\\marine\\marine";
        let data = crate::app::model_preview::harness::kit::kit_preview(
            &root,
            crate::core::game::GameId::HaloReach,
            path,
            "biped",
            b"bipd",
        );
        let shown = |permutation: &str| {
            let mut state = ModelPreviewState::default();
            let selection = ModelRegionSelection {
                enabled: true,
                permutation: permutation.to_owned(),
            };
            state
                .region_selections
                .insert("flair_backpack".to_owned(), selection);
            let shown = super::FeatureShown::of(&data.preview, &state.region_selections);
            shown.list(super::paths::ANTENNAS).to_vec()
        };
        assert_eq!(shown("trooper_radio"), [true]);
        assert_eq!(shown("trooper"), [false]);
    }
}
