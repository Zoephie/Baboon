//! The preview data a load produces: the renderer-facing geometry, its
//! materials, markers and nodes, and the resolved variants beside it.

use super::super::GameId;
use super::{
    ChangeColors, MaterialTextures, PreviewAnimationEntry, antennas, ce, cloth, gen3, h2, h3, h4,
    particles, reach,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Renderer-facing preview geometry derived from a [`RenderModel`]. Lives in
/// Baboon (not blam-tags) since it is purely a GUI concern.
#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreview {
    /// The game the opened tag is from: the tag's own generation for Halo CE
    /// and Halo 2, else its kit's. None for geometry no game's tag gave.
    pub game: Option<GameId>,
    /// The opened tag's engine generation, which a preview of no game is
    /// drawn by. None for geometry no tag gave.
    pub generation: Option<blam_tags::game::Game>,
    pub regions: Vec<RenderModelPreviewRegion>,
    pub vertices: Vec<RenderModelPreviewVertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<RenderModelPreviewBatch>,
    /// One entry per `RenderModelPreviewBatch::material_index`, naming the
    /// shader tag that batch draws with. Kept as the raw reference rather than
    /// resolved textures: resolving reads other tags off disk, which belongs on
    /// a worker rather than in the geometry walk.
    pub materials: Vec<RenderModelPreviewMaterial>,
    pub markers: Vec<RenderModelPreviewMarker>,
    /// The render model's skeleton, for animation playback. Empty on derived
    /// previews (collision, physics, BSPs, particles), which cannot animate.
    pub nodes: Vec<RenderModelPreviewNode>,
    /// Import-tool diagnostics carried by render, collision, and physics
    /// tags. They stay separate from draw batches so the UI can paint them as
    /// an always-legible overlay and attach hover text to each primitive.
    pub errors: Vec<ModelErrorPrimitive>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// Where a held weapon pulls the unit's hands, in every game that holds
    /// weapons.
    pub weapon_ik: Vec<crate::app::model_preview::animation::ik::weapon::WeaponIk>,
    pub particles: Vec<Arc<particles::ParticleSystem>>,
    pub cloth: Vec<cloth::ClothAt>,
    pub instances: RenderModelInstances,
    /// Antennas, on the markers of the opened object.
    pub antennas: Vec<antennas::Antenna>,
    /// Halo 2's own features.
    pub h2: h2::H2Features,
    /// What more than one gen3 game shares.
    pub gen3: gen3::Gen3Features,
    /// Halo 3's and ODST's own features.
    pub h3: h3::H3Features,
    /// Reach's own features.
    pub reach: reach::ReachFeatures,
    /// Halo 4's own features.
    pub h4: h4::H4Features,
    /// Halo CE's own features. Last, as its fields were: the load gate's
    /// lines for a group read as if its fields were inline here.
    pub ce: ce::CeFeatures,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewRegion {
    pub name: String,
    pub permutations: Vec<String>,
}

/// A render model's instance placements: pieces the instance mesh draws one
/// subpart at a time, which a variant chooses by group rather than by region.
#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelInstances {
    /// Placement names, which instance groups match against.
    pub names: Vec<String>,
    /// Which placement each of its batches draws. Ordered maps here and
    /// below only so the loader's snapshot reads the same every run.
    pub batches: std::collections::BTreeMap<usize, usize>,
    pub vertex_count: usize,
    /// Each permutation's `instance mask`, by region and name.
    pub masks: std::collections::BTreeMap<(String, String), u128>,
    /// The placements each variant's instance group chooses.
    pub variants: Vec<u128>,
    /// Regions with no mesh of their own in any permutation.
    pub empty_regions: std::collections::BTreeSet<String>,
}

/// One vertex as the GL program consumes it. `#[repr(C)]` because the whole
/// buffer is uploaded as raw bytes; the attribute offsets in
/// `RenderCore::new` are hand-written to match this order and
/// a test pins them.
///
/// `tangent` and `binormal` are carried rather than reconstructed: a normal map
/// needs the handedness the tag authored, and `cross(normal, tangent)` alone
/// cannot recover a mirrored UV island's sign — which is most of a Halo
/// character, since they mirror left to right to halve texture space.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RenderModelPreviewVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub texcoord: [f32; 2],
    pub tangent: [f32; 3],
    pub binormal: [f32; 3],
    /// Skeleton node influences, as FLOATS: GLSL 100 (the GLES fallback
    /// dialect) has no integer vertex attributes, so the indices ride as
    /// floats and the shader casts. All zeros — weights included — on
    /// geometry that has no skeleton (derived previews, particles, chimp);
    /// the shader's animated path treats zero total weight as rigid-to-bind.
    pub node_indices: [f32; 4],
    pub node_weights: [f32; 4],
}

/// The shader one material draws with, as named by the render_model.
#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewMaterial {
    /// Tag-relative path, no extension — e.g.
    /// `objects\characters\masterchief\shaders\masterchief`. Empty when
    /// the tag_ref was null.
    pub shader_path: String,
    /// Group FOURCC of the shader reference (`rmsh`, `rmtr`, `shad`, ...).
    /// Decides which extension the path resolves with, and which resolver
    /// understands the tag behind it.
    pub shader_group: u32,
    /// Halo CE's: an overlay, and which image of each bitmap it draws.
    pub ce: super::ce::CeMaterialLook,
    /// Halo 2's: which shield shader it is.
    pub h2: super::h2::H2MaterialLook,
    /// Which object owns it, for its function outputs and change colors: 0
    /// for the opened one, then its attached objects in order.
    pub object: u8,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewBatch {
    pub region_name: String,
    pub permutation_name: String,
    pub material_index: u16,
    pub index_start: u32,
    pub index_count: u32,
    /// A fixed color instead of the cycling per-material palette. Derived
    /// geometry (collision, physics) sets it so an overlay keeps one
    /// recognizable color no matter where its material lands in the list.
    pub flat_color: Option<[u8; 3]>,
    /// Which independently-toggleable model layer owns this batch. Keeping
    /// this separate from `region_name` lets collision geometry retain its
    /// real region/permutation taxonomy for variant selection.
    pub layer: ModelPreviewLayer,
    /// Where its part sorts among the transparents, and the node that carries
    /// that point: Halo CE's part centroid, read only by the CE sort.
    pub sort_position: Option<([f32; 3], usize)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ModelPreviewLayer {
    #[default]
    Render,
    Collision,
    Physics,
}

#[derive(Debug, Clone)]
pub(crate) struct ModelErrorPoint {
    pub position: [f32; 3],
    pub node_indices: [i16; 4],
    pub node_weights: [f32; 4],
}

pub(super) const MODEL_ERROR_FALLBACK_COLOR: [u8; 4] = [255, 55, 45, 255];

#[derive(Debug, Clone)]
pub(crate) struct ModelErrorPrimitive {
    pub label: String,
    pub non_critical: bool,
    /// Authored debug-view color as RGBA bytes.
    pub color: [u8; 4],
    pub layer: ModelPreviewLayer,
    pub shape: ModelErrorShape,
}

#[derive(Debug, Clone)]
pub(crate) enum ModelErrorShape {
    Point(ModelErrorPoint),
    Vector {
        point: ModelErrorPoint,
        normal: [f32; 3],
        length: f32,
    },
    Polyline(Vec<ModelErrorPoint>),
    Face(Vec<ModelErrorPoint>),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewMarker {
    pub name: String,
    pub node_index: i16,
    pub position: [f32; 3],
    pub axes: [[f32; 3]; 3],
    pub scale: f32,
    /// The region and permutation it belongs to, if it belongs to only one:
    /// `render_model_get_markers_by_group_index` skips it under any other.
    pub only_under: Option<(String, String)>,
}

/// One skeleton node, carried so animation playback can run forward
/// kinematics and skinning without going back to the tag. Parent-before-child
/// order (the tag's own), which the FK pass relies on.
#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewNode {
    pub name: String,
    pub parent: i16,
    /// Parent-local bind pose.
    pub bind_rotation: [f32; 4],
    pub bind_translation: [f32; 3],
    /// Inverse of the accumulated bind-pose world transform, as three rows of
    /// an affine matrix — what turns a bind-space vertex into node-local
    /// space before the animated world transform takes it back out.
    pub inverse_bind: [[f32; 4]; 3],
}

pub(super) static NEXT_MODEL_GEOMETRY_ID: AtomicU64 = AtomicU64::new(1);

pub(super) fn model_preview_data(
    source_key: String,
    render_model_path: String,
    preview: RenderModelPreview,
    variants: Vec<ModelVariantPreview>,
) -> ModelPreviewData {
    let geometry_id = NEXT_MODEL_GEOMETRY_ID.fetch_add(1, Ordering::Relaxed);
    ModelPreviewData {
        source_key,
        render_model_path,
        preview: Arc::new(preview),
        geometry_id,
        mesh_id: geometry_id,
        textures_id: geometry_id,
        textures: None,
        textures_game_shaders: false,
        variants,
        default_variant: None,
        change_colors: Default::default(),
        ce: Default::default(),
        object_functions: None,
        weapons: None,
        attached: Vec::new(),
        scenario_bsps: Vec::new(),
        animations: None,
        notes: Vec::new(),
    }
}

#[derive(Clone)]
/// Source geometry and resolved variants used by the shared GPU renderer.
pub(in crate::app) struct ModelPreviewData {
    pub(in crate::app) source_key: String,
    pub(in crate::app) render_model_path: String,
    /// Shared so the deferred Glow callback can retain the immutable source
    /// geometry without cloning dense vertex/index buffers every frame.
    pub(in crate::app) preview: std::sync::Arc<RenderModelPreview>,
    /// Monotonic identity of this load of the preview: what the GL slot,
    /// the running effects and the function inputs belong to. Unlike a
    /// pointer, it cannot be accidentally reused after a preview reload. Kept
    /// when collision/physics overlays merge in, so they reset nothing.
    pub(in crate::app) geometry_id: u64,
    /// Identity of the vertices and indices: `geometry_id` when loaded, new
    /// each time overlays merge in, so the slot re-uploads its buffers.
    pub(in crate::app) mesh_id: u64,
    /// Identity of the material list `textures` is resolved against. Set when
    /// the preview loads and kept when collision/physics overlays merge in:
    /// the merge only appends flat-coloured materials after the model's own,
    /// so textures resolved for it (or still resolving) stay right. Keyed to
    /// `geometry_id` instead, every merge threw away a resolve in flight and
    /// re-uploaded textures already on the GPU.
    pub(in crate::app) textures_id: u64,
    /// One entry per `RenderModelPreview::materials`, once the worker has
    /// resolved them. `None` while that job is still running — the panel shows
    /// its spinner rather than drawing the model untextured, so a model never
    /// appears half-shaded and then changes under the cursor.
    pub(in crate::app) textures: Option<std::sync::Arc<Vec<MaterialTextures>>>,
    /// Whether `textures` were resolved with what only Game Shaders draws
    /// (see `resolve_model_textures`). Plain ones stay drawn with Game Shaders
    /// off; turned on, they are resolved again.
    pub(in crate::app) textures_game_shaders: bool,
    pub(in crate::app) variants: Vec<ModelVariantPreview>,
    /// The variant an object tag starts in, when it names one.
    pub(in crate::app) default_variant: Option<String>,
    /// The object beside this model's change colors; empty otherwise.
    pub(in crate::app) change_colors: ChangeColors,
    /// An opened Halo CE object's own: its functions, what it brings with
    /// it and what it can hold.
    pub(in crate::app) ce: super::ce::object::CeObjectData,
    /// Halo 2 and later: the functions of an opened object.
    pub(in crate::app) object_functions: Option<super::functions::ObjectFunctions>,
    /// Halo 2 and later units: what they can hold.
    pub(in crate::app) weapons: Option<super::weapons::UnitWeapons>,
    /// Halo 2 and later units: what they hold.
    pub(in crate::app) attached: Vec<super::weapons::attached::AttachedObject>,
    /// Scenario tags only: the `structure bsps` block's references (one per
    /// element, `None` where the reference is empty), driving the per-BSP
    /// checkbox list. Empty for every other tag group.
    pub(in crate::app) scenario_bsps: Vec<Option<String>>,
    /// `.model` tags only: the linked animation graph's animation list, once
    /// the worker has read it. `None` elsewhere and before it lands.
    pub(in crate::app) animations: Option<std::sync::Arc<Vec<PreviewAnimationEntry>>>,
    /// What the preview shows without, and why: a feature whose tags would
    /// not read, a weapon that would not attach. The geometry is still there.
    pub(in crate::app) notes: Vec<String>,
}

impl ModelPreviewData {
    /// The textures, if they were resolved for a preview with `game_shaders`
    /// as it is: ones resolved for Game Shaders serve it off as well.
    pub(in crate::app) fn textures_for(
        &self,
        game_shaders: bool,
    ) -> Option<&std::sync::Arc<Vec<MaterialTextures>>> {
        self.textures
            .as_ref()
            .filter(|_| self.textures_game_shaders || !game_shaders)
    }

    /// Take `textures`, resolved for `game_shaders`, unless it holds ones
    /// for Game Shaders and these are plain: off, those draw as plain ones.
    /// Whether it took them. Replacing what the GPU holds under
    /// `textures_id`, they take a new one, so the renderer uploads them.
    pub(in crate::app) fn set_textures(
        &mut self,
        textures: Vec<MaterialTextures>,
        game_shaders: bool,
    ) -> bool {
        if self.textures_game_shaders && !game_shaders {
            return false;
        }
        if self.textures.is_some() {
            self.textures_id = NEXT_MODEL_GEOMETRY_ID.fetch_add(1, Ordering::Relaxed);
        }
        self.textures = Some(Arc::new(textures));
        self.textures_game_shaders = game_shaders;
        true
    }
}

#[derive(Clone)]
/// Resolved model variant with an explicit removed-vs-inherited region boundary.
pub(in crate::app) struct ModelVariantPreview {
    pub(in crate::app) name: String,
    /// Region name → the permutation this variant draws it with. A name the
    /// render model lacks draws nothing, as the game resolves it to none.
    pub(in crate::app) regions: HashMap<String, String>,
    /// Regions the variant gives an entry naming no permutation, which is how
    /// a tag leaves one undrawn. A region in neither map is not this variant's
    /// to choose, and takes an ordinary permutation.
    pub(in crate::app) hidden_regions: std::collections::HashSet<String>,
    /// Halo 4's `muted nodes`: each, and everything under it, held at its
    /// bind pose while an animation plays.
    pub(in crate::app) muted_nodes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(count: usize) -> Vec<MaterialTextures> {
        (0..count).map(|_| MaterialTextures::default()).collect()
    }

    /// Plain textures serve the preview only with Game Shaders off; ones for
    /// Game Shaders replace them under a new `textures_id`, so the renderer
    /// uploads them, and plain ones never replace those.
    #[test]
    fn textures_for_game_shaders_replace_plain_ones_and_are_kept_over_them() {
        let mut data = model_preview_data(
            "a.model".to_owned(),
            "a".to_owned(),
            RenderModelPreview::default(),
            Vec::new(),
        );
        assert!(data.textures_for(false).is_none(), "nothing resolved yet");
        let first = data.textures_id;
        assert!(data.set_textures(resolved(1), false));
        assert_eq!(data.textures_id, first, "the first resolve keeps the load's id");
        assert!(data.textures_for(false).is_some());
        assert!(data.textures_for(true).is_none(), "Game Shaders wait for their own");

        assert!(data.set_textures(resolved(2), true));
        assert_ne!(data.textures_id, first, "replaced, so uploaded again");
        assert_eq!(data.textures_for(true).map(|t| t.len()), Some(2));
        assert_eq!(data.textures_for(false).map(|t| t.len()), Some(2), "and serve off too");

        let kept = data.textures_id;
        assert!(!data.set_textures(resolved(3), false), "plain ones are dropped");
        assert_eq!(data.textures_id, kept);
        assert_eq!(data.textures_for(true).map(|t| t.len()), Some(2));
    }
}
