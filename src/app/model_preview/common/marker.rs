//! A marker as a rigid frame, and how a point or direction of one object
//! lands on another's marker.

use super::super::*;

/// A marker as a rigid transform: its three axes, then its position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app::model_preview) struct MarkerFrame {
    pub(in crate::app::model_preview) axes: [[f32; 3]; 3],
    pub(in crate::app::model_preview) position: [f32; 3],
    pub(in crate::app::model_preview) node: i16,
}

impl MarkerFrame {
    pub(in crate::app::model_preview) const ORIGIN: Self = Self {
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        position: [0.0; 3],
        node: 0,
    };

    /// `object_get_marker_by_name`: no name, or none found, is the origin.
    pub(in crate::app::model_preview) fn find(preview: &RenderModelPreview, name: &str) -> Self {
        preview
            .markers
            .iter()
            .find(|marker| !name.is_empty() && marker.name.eq_ignore_ascii_case(name))
            .map_or(Self::ORIGIN, |marker| Self {
                axes: marker.axes,
                position: marker.position,
                node: marker.node_index.max(0),
            })
    }

    /// A direction in this frame's axes, in the space the frame sits in.
    fn rotate(&self, value: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|axis| {
            (0..3)
                .map(|index| self.axes[index][axis] * value[index])
                .sum()
        })
    }

    /// `rotate`'s inverse: axes are unit and square to each other.
    fn unrotate(&self, value: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|index| {
            (0..3)
                .map(|axis| self.axes[index][axis] * value[axis])
                .sum()
        })
    }
}

/// Where a point of the child goes when its `child` marker meets `parent`.
pub(in crate::app::model_preview) fn attach_point(
    parent: &MarkerFrame,
    child: &MarkerFrame,
    point: [f32; 3],
) -> [f32; 3] {
    let local = child.unrotate(std::array::from_fn(|axis| {
        point[axis] - child.position[axis]
    }));
    let turned = parent.rotate(local);
    std::array::from_fn(|axis| turned[axis] + parent.position[axis])
}

pub(in crate::app::model_preview) fn attach_vector(
    parent: &MarkerFrame,
    child: &MarkerFrame,
    vector: [f32; 3],
) -> [f32; 3] {
    parent.rotate(child.unrotate(vector))
}

/// Where a marker of the opened object puts what is attached to it.
pub(in crate::app::model_preview) fn own_marker_frame(
    marker: &RenderModelPreviewMarker,
) -> (MarkerFrame, usize) {
    let place = MarkerFrame {
        axes: marker.axes,
        position: marker.position,
        node: marker.node_index,
    };
    (place, marker.node_index.max(0) as usize)
}
