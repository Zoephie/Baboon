//! The explicit shader a set's `render shader` chooses, by the option's name.

/// A `render shader` option. Halo 3 and ODST name six; Reach and Halo 4
/// renamed them and added a seventh, and the same position means a different
/// shader: Halo 3's third is `still + no lights`, Reach's `solid mesh +
/// dynamic lights`. Each name is its own variant, so neither is mistaken for
/// the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecoratorShader {
    /// Halo 3's `decorator_default`.
    WindDynamicLights,
    /// Halo 3's `decorator_no_wind`.
    StillDynamicLights,
    /// Halo 3's `decorator_static`.
    StillNoLights,
    /// Halo 3's `decorator_sun`.
    StillSunLight,
    /// Halo 3's `decorator_wavy`.
    WavyDynamicLights,
    /// Halo 3's `decorator_shaded`.
    ShadedDynamicLights,
    /// Reach's and Halo 4's `decorator_wind_billboard`.
    BillboardWindDynamicLights,
    /// Reach's and Halo 4's `decorator_static_billboard`.
    BillboardDynamicLights,
    /// Reach's and Halo 4's `decorator_mesh`.
    SolidMeshDynamicLights,
    /// Reach's and Halo 4's `decorator_far_object`.
    SolidMesh,
    /// Reach's and Halo 4's `decorator_underwater`.
    UnderwaterDynamicLights,
    /// Reach's and Halo 4's `decorator_volume`.
    VolumetricBillboardDynamicLights,
    /// Reach's and Halo 4's `decorator_wind_volume`.
    VolumetricBillboardWindDynamicLights,
}

/// Every option, as the game's layouts name it.
const NAMES: [(&str, DecoratorShader); 13] = [
    ("wind + dynamic lights", DecoratorShader::WindDynamicLights),
    (
        "still + dynamic lights",
        DecoratorShader::StillDynamicLights,
    ),
    ("still + no lights", DecoratorShader::StillNoLights),
    ("still + sun light only", DecoratorShader::StillSunLight),
    ("wavy + dynamic lights", DecoratorShader::WavyDynamicLights),
    (
        "shaded + dynamic lights",
        DecoratorShader::ShadedDynamicLights,
    ),
    (
        "billboard + wind + dynamic lights",
        DecoratorShader::BillboardWindDynamicLights,
    ),
    (
        "billboard + dynamic lights",
        DecoratorShader::BillboardDynamicLights,
    ),
    (
        "solid mesh + dynamic lights",
        DecoratorShader::SolidMeshDynamicLights,
    ),
    ("solid mesh", DecoratorShader::SolidMesh),
    (
        "underwater + dynamic lights",
        DecoratorShader::UnderwaterDynamicLights,
    ),
    (
        "volumetric billboard + dynamic lights",
        DecoratorShader::VolumetricBillboardDynamicLights,
    ),
    (
        "volumetric billboard + wind + dynamic lights",
        DecoratorShader::VolumetricBillboardWindDynamicLights,
    ),
];

impl DecoratorShader {
    /// The option named `name` as the editors show it (its `{alias}` and
    /// `#help` left out); `None` for one this build does not know.
    pub(crate) fn from_option_name(name: &str) -> Option<Self> {
        NAMES
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, shader)| *shader)
    }

    /// A game's first option, which a layout without the field reads as:
    /// Reach renamed Halo 3's `wind + dynamic lights`.
    pub(crate) fn first_option(game: Option<crate::core::game::GameId>) -> Self {
        use crate::core::game::GameId;
        match game {
            Some(GameId::HaloReach | GameId::Halo4 | GameId::Halo2Amp) => {
                Self::BillboardWindDynamicLights
            }
            _ => Self::WindDynamicLights,
        }
    }

    /// The explicit shader its game's `rasterizer_globals` name for it,
    /// compiled from source: Halo 3's and ODST's `get_decorator_explicit_shader`
    /// returns their table's 47, 70, 71, 72, 73 and 82; Reach's adds 164.
    pub(crate) fn source_file(self) -> &'static str {
        match self {
            Self::WindDynamicLights => "decorator_default",
            Self::StillDynamicLights => "decorator_no_wind",
            Self::StillNoLights => "decorator_static",
            Self::StillSunLight => "decorator_sun",
            Self::WavyDynamicLights => "decorator_wavy",
            Self::ShadedDynamicLights => "decorator_shaded",
            Self::BillboardWindDynamicLights => "decorator_wind_billboard",
            Self::BillboardDynamicLights => "decorator_static_billboard",
            Self::SolidMeshDynamicLights => "decorator_mesh",
            Self::SolidMesh => "decorator_far_object",
            Self::UnderwaterDynamicLights => "decorator_underwater",
            Self::VolumetricBillboardDynamicLights => "decorator_volume",
            Self::VolumetricBillboardWindDynamicLights => "decorator_wind_volume",
        }
    }

    /// Whether its shader is built with `DECORATOR_WIND` or `DECORATOR_WAVY`,
    /// which move it on the clock.
    pub(crate) fn moves(self) -> bool {
        matches!(
            self,
            Self::WindDynamicLights
                | Self::WavyDynamicLights
                | Self::BillboardWindDynamicLights
                | Self::UnderwaterDynamicLights
                | Self::VolumetricBillboardWindDynamicLights
        )
    }

    /// Whether the wind sways it: its shader is built with `DECORATOR_WIND`.
    pub(crate) fn moves_in_the_wind(self) -> bool {
        matches!(
            self,
            Self::WindDynamicLights
                | Self::BillboardWindDynamicLights
                | Self::VolumetricBillboardWindDynamicLights
        )
    }

    /// The option's name.
    pub(crate) fn name(self) -> &'static str {
        NAMES
            .iter()
            .find(|(_, shader)| *shader == self)
            .map_or("", |(name, _)| name)
    }
}

#[cfg(test)]
mod tests {
    use super::DecoratorShader;

    #[test]
    fn every_option_reads_back_by_its_name() {
        for (name, shader) in super::NAMES {
            assert_eq!(DecoratorShader::from_option_name(name), Some(shader));
            assert_eq!(shader.name(), name);
        }
        assert_eq!(DecoratorShader::from_option_name("wind + no lights"), None);
    }
}
