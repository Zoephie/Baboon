//! Halo 4's light volumes, which it draws through their material from each
//! profile's packed state: `light_volume_update.fx`'s outputs worked out
//! over the shared layout a frame at a time.

use super::super::super::common::view::ViewFrame;
use super::super::super::functions::FunctionValues;
use super::super::super::gen3::effects::light_volumes::{
    Gen3LightVolumeAt, Gen3VolumeInput, VolumeLayout, percentile,
};
use super::super::super::gen3::effects::light_volumes::{
    Gen3VolumeDefinition, read_material_definition,
};
use super::super::super::h4::materials::resolve_h4_effect_material;
use super::super::super::load::TagReader;
use super::particles::{f32_to_f16, pack_argb8};
use blam_tags::TagStruct;

/// `c_light_volume_definition::postprocess_block_proc` (MCC 0x1403ABF50):
/// Reach's, through the `actual material`, and never precompiled, which MCC
/// clears every time and could not draw ("not implemented for D3D11").
pub(in crate::app::model_preview) fn read_h4_definition(
    reader: &TagReader,
    element: TagStruct<'_>,
) -> Option<Gen3VolumeDefinition> {
    let material =
        blam_tags::material::Material::from_struct(&element.descend("actual material?")?);
    let resolved = resolve_h4_effect_material(reader, &material, "light_volume")
        .map_err(|error| eprintln!("h4 light volume material: {error}"))
        .ok()?;
    let appearance = element.read_int_any("appearance flags").unwrap_or(0) as u32;
    let sort_layer = i32::from(material.sort_layer).clamp(1, 3);
    read_material_definition(element, sort_layer, (resolved, appearance))
}

/// `light_volume_state_input_enum`'s order, the order of
/// `LightVolumeOverallState.inputs`; `allocate_random_seed` leaves nothing
/// reading `random`.
pub(in crate::app::model_preview) const H4_STATE_ORDER: [Gen3VolumeInput; 10] = [
    Gen3VolumeInput::Percentile,
    Gen3VolumeInput::GameTime,
    Gen3VolumeInput::Age,
    Gen3VolumeInput::Random,
    Gen3VolumeInput::Seed(0),
    Gen3VolumeInput::Seed(1),
    Gen3VolumeInput::Lod,
    Gen3VolumeInput::EffectA,
    Gen3VolumeInput::EffectB,
    Gen3VolumeInput::Invalid,
];

/// `ProfileMemexportedState`, as `light_volume_main` writes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app::model_preview) struct H4ProfileState {
    pub(in crate::app::model_preview) position: [f32; 3],
    pub(in crate::app::model_preview) percentile: f32,
    /// Unclamped; the packing saturates it.
    pub(in crate::app::model_preview) color: [f32; 4],
    pub(in crate::app::model_preview) thickness: f32,
    pub(in crate::app::model_preview) intensity: f32,
}

/// `WriteProfileMemexportedState`: `RawProfileState`'s eight words, the
/// thickness and intensity as halves, the colour as ARGB8, the pad 0.
pub(in crate::app::model_preview) fn pack_profile_state(state: &H4ProfileState) -> [u32; 8] {
    let [x, y, z] = state.position.map(f32::to_bits);
    let misc = f32_to_f16(state.thickness) | (f32_to_f16(state.intensity) << 16);
    [
        x,
        y,
        z,
        state.percentile.to_bits(),
        misc,
        pack_argb8(state.color),
        0,
        0,
    ]
}

/// One Halo 4 volume's frame: `LightVolumeOverallState` and its profiles'
/// packed states, in row order.
#[derive(Debug, Clone)]
pub(crate) struct H4VolumeDraw {
    pub(crate) appearance: f32,
    pub(crate) brightness_ratio: f32,
    pub(crate) offset: f32,
    /// `numProfiles`: `length × density`, whole or not.
    pub(crate) profiles: f32,
    /// `profileDistance`.
    pub(crate) distance: f32,
    /// `profileLength`.
    pub(crate) side_length: f32,
    pub(crate) origin: [f32; 3],
    pub(crate) direction: [f32; 3],
    pub(crate) inputs: [f32; 10],
    pub(crate) states: Vec<[u32; 8]>,
}

impl Gen3LightVolumeAt {
    /// Halo 4's draw this frame: `SetMaterialState`'s overall state and,
    /// for each profile, the state `light_volume_update.fx` writes, packed.
    pub(crate) fn h4_draw(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
    ) -> Option<H4VolumeDraw> {
        let (_, appearance) = self.definition.effect_material.as_ref()?;
        self.h4_frame(*appearance, values, bones, view, time, onset)
    }

    /// [`Self::h4_draw`] with `appearance flags` of `appearance`.
    pub(in crate::app::model_preview) fn h4_frame(
        &self,
        appearance: u32,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
    ) -> Option<H4VolumeDraw> {
        let volume = &*self.definition;
        let VolumeLayout {
            mut state,
            origin,
            direction,
            count,
            profiles,
            distance,
            offset,
            side_length,
        } = self.layout(values, bones, view, time, onset)?;
        if count <= 0 {
            return None;
        }
        // `inputs[]`: every state but the profile's own place, which the
        // update reads from the profile.
        let inputs = H4_STATE_ORDER.map(|input| match input {
            Gen3VolumeInput::Percentile => 0.0,
            other => other.of(&state),
        });
        let states = (0..count)
            .map(|index| {
                state.percentile = percentile(index, profiles);
                let along = offset + distance * index as f32;
                let position: [f32; 3] =
                    std::array::from_fn(|axis| origin[axis] + direction[axis] * along);
                let [red, green, blue] = volume.color.color(&state);
                pack_profile_state(&H4ProfileState {
                    position,
                    percentile: state.percentile,
                    color: [red, green, blue, volume.alpha.scalar(&state)],
                    thickness: volume.thickness.scalar(&state),
                    intensity: volume.intensity.scalar(&state),
                })
            })
            .collect();
        Some(H4VolumeDraw {
            appearance: appearance as f32,
            brightness_ratio: volume.brightness_ratio,
            offset,
            profiles,
            distance,
            side_length,
            origin,
            direction,
            inputs,
            states,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `WriteProfileMemexportedState`: the position and percentile as they
    /// are, thickness and intensity as halves, the colour saturated into
    /// ARGB8 (`PackARGB8`), which an intensity over 1 survives.
    #[test]
    fn a_profile_packs_as_the_update_writes_it() {
        let state = H4ProfileState {
            position: [1.0, -2.0, 3.5],
            percentile: 0.25,
            color: [2.0, 0.5, -1.0, 1.0],
            thickness: 0.5,
            intensity: 3.0,
        };
        let words = pack_profile_state(&state);
        assert_eq!(words[..4], [1.0f32, -2.0, 3.5, 0.25].map(f32::to_bits));
        // Half 0.5 is 0x3800, half 3 is 0x4200.
        assert_eq!(words[4], 0x3800 | (0x4200 << 16));
        // Red saturates to 255, blue to 0; green truncates 127.5 to 127.
        assert_eq!(words[5], (255 << 24) | (255 << 16) | (127 << 8));
        assert_eq!(words[6..], [0, 0]);
        let dimmer = pack_profile_state(&H4ProfileState {
            intensity: 1.0,
            ..state
        });
        assert_ne!(
            dimmer[4], words[4],
            "intensity rides in the halves, not the colour"
        );
        assert_eq!(dimmer[5], words[5]);
    }
}
