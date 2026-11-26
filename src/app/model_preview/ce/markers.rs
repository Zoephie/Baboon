//! An object's markers as Halo CE's `object_get_marker_by_name` finds them:
//! only those of the permutations its regions show, the lowest region, then
//! permutation, then node first, and its origin when a name finds none.

use super::super::common::marker::MarkerFrame;
use super::super::frame::marker_active;
use super::super::{ModelRegionSelection, RenderModelPreviewMarker, RenderModelPreviewRegion};
use std::collections::HashMap;

/// One marker where it sits in the preview.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CeMarker {
    pub(in crate::app::model_preview) node: usize,
    pub(in crate::app::model_preview) position: [f32; 3],
    /// Forward, left and up.
    pub(in crate::app::model_preview) axes: [[f32; 3]; 3],
}

impl CeMarker {
    /// The object's own frame: `object_get_marker_by_name`'s answer for no
    /// name, and what it leaves in the first slot when a name finds none.
    pub(in crate::app::model_preview) const ORIGIN: Self = Self {
        node: 0,
        position: [0.0; 3],
        axes: MarkerFrame::ORIGIN.axes,
    };

    pub(in crate::app::model_preview) fn forward(&self) -> [f32; 3] {
        self.axes[0]
    }

    fn of(marker: &RenderModelPreviewMarker) -> Self {
        Self {
            node: marker.node_index.max(0) as usize,
            position: marker.position,
            axes: marker.axes,
        }
    }
}

impl Default for CeMarker {
    fn default() -> Self {
        Self::ORIGIN
    }
}

impl From<MarkerFrame> for CeMarker {
    fn from(frame: MarkerFrame) -> Self {
        Self {
            node: frame.node.max(0) as usize,
            position: frame.position,
            axes: frame.axes,
        }
    }
}

/// What `object_get_marker_by_name` answers: how many it found, and its
/// slots, of which the first holds the object's origin when it found none.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app::model_preview) struct CeFound {
    count: usize,
    slots: Vec<CeMarker>,
}

impl CeFound {
    /// Those it found, at most as many as were asked for.
    pub(in crate::app::model_preview) fn found(&self) -> &[CeMarker] {
        &self.slots[..self.count]
    }

    /// The first slot, read whatever the count, as lights, antennas, flags
    /// and light volumes read it: the origin when none was found.
    pub(in crate::app::model_preview) fn first(&self) -> CeMarker {
        self.slots[0]
    }
}

/// An object's markers under the permutations it shows, and its origin.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app::model_preview) struct CeObjectMarkers {
    /// By name, lowercase, each in `model_get_marker_by_name`'s order.
    named: Vec<(String, Vec<CeMarker>)>,
    origin: CeMarker,
}

impl Default for CeObjectMarkers {
    fn default() -> Self {
        Self::none(CeMarker::ORIGIN)
    }
}

impl CeObjectMarkers {
    /// An object with no model: every name finds nothing, and its origin is
    /// `origin`.
    pub(in crate::app::model_preview) fn none(origin: CeMarker) -> Self {
        Self {
            named: Vec::new(),
            origin,
        }
    }

    /// An object's `markers` where the preview holds them, in the order of
    /// `regions` and their permutations, those of a permutation `picks`
    /// doesn't show left out, and its `origin`. The tool's postprocess sorts
    /// each marker's instances by region, permutation, then node
    /// (`model_get_marker_by_name` walks them in that order); a marker of no
    /// region always counts.
    pub(in crate::app::model_preview) fn of_markers(
        markers: &[RenderModelPreviewMarker],
        regions: &[RenderModelPreviewRegion],
        picks: &HashMap<String, ModelRegionSelection>,
        origin: CeMarker,
    ) -> Self {
        let rank = |only: &Option<(String, String)>| {
            only.as_ref().map_or((0, 0), |(region, permutation)| {
                let at = regions.iter().position(|each| each.name == *region);
                let within = at.and_then(|at| {
                    regions[at]
                        .permutations
                        .iter()
                        .position(|each| each == permutation)
                });
                (at.unwrap_or(usize::MAX), within.unwrap_or(usize::MAX))
            })
        };
        let mut shown: Vec<_> = markers
            .iter()
            .filter(|marker| {
                marker
                    .only_under
                    .as_ref()
                    .is_none_or(|only| marker_active(only, picks))
            })
            .map(|marker| {
                let (region, permutation) = rank(&marker.only_under);
                (
                    marker.name.to_lowercase(),
                    (region, permutation, marker.node_index),
                    CeMarker::of(marker),
                )
            })
            .collect();
        // Stable, so instances alike in all three keep the tag's order.
        shown.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut named: Vec<(String, Vec<CeMarker>)> = Vec::new();
        for (name, _, at) in shown {
            match named.last_mut() {
                Some((last, list)) if *last == name => list.push(at),
                _ => named.push((name, vec![at])),
            }
        }
        Self { named, origin }
    }

    /// `object_get_marker_by_name` asked for at most `most`: no name is the
    /// origin, found once; a name it finds none of is nothing found, with
    /// the origin in the first slot.
    pub(in crate::app::model_preview) fn get(&self, name: &str, most: usize) -> CeFound {
        if name.is_empty() {
            return CeFound {
                count: 1,
                slots: vec![self.origin],
            };
        }
        let name = name.to_lowercase();
        let found: Vec<CeMarker> = self
            .named
            .iter()
            .find(|(each, _)| *each == name)
            .map(|(_, list)| list.iter().take(most.max(1)).copied().collect())
            .unwrap_or_default();
        if found.is_empty() {
            CeFound {
                count: 0,
                slots: vec![self.origin],
            }
        } else {
            CeFound {
                count: found.len(),
                slots: found,
            }
        }
    }
}

/// Which permutation each region a pick could move a marker in shows, as a
/// load resolved the object's markers with: those with more than one
/// permutation, under which any marker sits. `None` for a region not shown.
pub(in crate::app::model_preview) type CeMarkerPicks = Vec<(String, Option<String>)>;

/// The picks `markers` depend on, as `picks` has them.
pub(in crate::app::model_preview) fn marker_picks(
    markers: &[RenderModelPreviewMarker],
    regions: &[RenderModelPreviewRegion],
    picks: &HashMap<String, ModelRegionSelection>,
) -> CeMarkerPicks {
    regions
        .iter()
        .filter(|region| region.permutations.len() > 1)
        .filter(|region| {
            markers.iter().any(|marker| {
                marker
                    .only_under
                    .as_ref()
                    .is_some_and(|(at, _)| *at == region.name)
            })
        })
        .map(|region| (region.name.clone(), picked(&region.name, picks)))
        .collect()
}

fn picked(region: &str, picks: &HashMap<String, ModelRegionSelection>) -> Option<String> {
    picks
        .get(region)
        .filter(|selection| selection.enabled)
        .map(|selection| selection.permutation.clone())
}

/// Whether `picks` still shows what `loaded` resolved markers with.
pub(in crate::app::model_preview) fn picks_match(
    loaded: &CeMarkerPicks,
    picks: &HashMap<String, ModelRegionSelection>,
) -> bool {
    loaded
        .iter()
        .all(|(region, permutation)| picked(region, picks) == *permutation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(
        name: &str,
        region: &str,
        permutation: &str,
        node: i16,
        x: f32,
    ) -> RenderModelPreviewMarker {
        RenderModelPreviewMarker {
            name: name.to_owned(),
            node_index: node,
            position: [x, 0.0, 0.0],
            axes: MarkerFrame::ORIGIN.axes,
            scale: 1.0,
            only_under: Some((region.to_owned(), permutation.to_owned())),
        }
    }

    fn regions() -> Vec<RenderModelPreviewRegion> {
        vec![
            RenderModelPreviewRegion {
                name: "body".into(),
                permutations: vec!["base".into(), "damaged".into()],
            },
            RenderModelPreviewRegion {
                name: "arm".into(),
                permutations: vec!["base".into()],
            },
        ]
    }

    fn picks(body: &str) -> HashMap<String, ModelRegionSelection> {
        [("body", body), ("arm", "base")]
            .into_iter()
            .map(|(region, permutation)| {
                (
                    region.to_owned(),
                    ModelRegionSelection {
                        enabled: true,
                        permutation: permutation.to_owned(),
                    },
                )
            })
            .collect()
    }

    /// `model_get_marker_by_name`: only the shown permutation's, the lowest
    /// region, permutation and node first, whatever order the tag lists
    /// them in; the name matched in any case.
    #[test]
    fn a_name_finds_the_shown_permutations_markers_in_the_tools_order() {
        let markers = [
            marker("Light", "arm", "base", 0, 4.0),
            marker("light", "body", "damaged", 0, 3.0),
            marker("light", "body", "base", 2, 2.0),
            marker("light", "body", "base", 1, 1.0),
        ];
        let at = |body: &str, most: usize| {
            CeObjectMarkers::of_markers(&markers, &regions(), &picks(body), CeMarker::ORIGIN)
                .get("LIGHT", most)
                .found()
                .iter()
                .map(|marker| marker.position[0])
                .collect::<Vec<_>>()
        };
        assert_eq!(at("base", 8), [1.0, 2.0, 4.0]);
        assert_eq!(at("damaged", 8), [3.0, 4.0]);
        assert_eq!(at("base", 1), [1.0], "asked for one, the first");
    }

    /// `object_get_marker_by_name`: no name is the origin, found; a missing
    /// name finds nothing but leaves the origin in the first slot.
    #[test]
    fn no_name_is_the_origin_and_a_missing_one_finds_nothing() {
        let origin = CeMarker {
            node: 3,
            position: [5.0, 6.0, 7.0],
            axes: MarkerFrame::ORIGIN.axes,
        };
        let markers = CeObjectMarkers::none(origin);
        let empty = markers.get("", 8);
        assert_eq!(empty.found(), [origin]);
        let missing = markers.get("gun", 8);
        assert!(missing.found().is_empty());
        assert_eq!(missing.first(), origin);
    }

    /// A load records the picks of regions a pick could move a marker in,
    /// and a pick there no longer matches it.
    #[test]
    fn only_a_pick_that_moves_a_marker_is_recorded() {
        let markers = [marker("light", "body", "base", 0, 1.0)];
        let loaded = marker_picks(&markers, &regions(), &picks("base"));
        assert_eq!(loaded, [("body".to_owned(), Some("base".to_owned()))]);
        assert!(picks_match(&loaded, &picks("base")));
        assert!(!picks_match(&loaded, &picks("damaged")));
    }
}
