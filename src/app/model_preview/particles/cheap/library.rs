//! The global cheap particle library: its types, render textures and turbulence, read once per game.

use super::super::super::{GameId, TagReader};
use super::{COLLISION, COLOR0, FADE, PHYSICS, RENDER, elements};
use blam_tags::{TagFieldData, TagStruct};

/// A map the cheap particle shaders fetch texels of: -1 to 1 for a signed
/// format (`q8w8v8u8`), 0 to 1 otherwise, in the decoder's r, g, b, a
/// [inferred: the standard D3D swizzle].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheapTexels {
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) texels: Vec<[f32; 4]>,
}

impl CheapTexels {
    /// The top level of a bitmap's first image.
    pub(super) fn read(reader: &TagReader, path: &str) -> Option<Self> {
        let tag = reader.load(path, "bitmap", b"bitm").ok()?;
        let bitmap = blam_tags::bitmap::Bitmap::new(&tag).ok()?;
        let image = bitmap.image(0)?;
        let format = image.format().ok()?;
        let (width, height) = (image.width(), image.height());
        let top = image
            .pixel_bytes()
            .ok()?
            .get(..format.level_bytes(width, height) as usize)?;
        let rgba = blam_tags::bitmap::decode::decode_to_rgba8(
            format,
            width,
            height,
            top,
            bitmap.p8_palette(),
        )
        .ok()?;
        Some(Self::of_bytes(
            width as usize,
            height as usize,
            &rgba,
            format.is_signed(),
        ))
    }

    /// From the decoder's bytes, which bias a signed byte by 128.
    fn of_bytes(width: usize, height: usize, rgba: &[u8], signed: bool) -> Self {
        let value = |byte: u8| {
            if signed {
                ((f32::from(byte) - 128.0) / 127.0).max(-1.0)
            } else {
                f32::from(byte) / 255.0
            }
        };
        let texels = rgba
            .chunks_exact(4)
            .map(|texel| std::array::from_fn(|channel| value(texel[channel])))
            .collect();
        Self {
            width,
            height,
            texels,
        }
    }

    /// `generate_random`: a point sample at `index · transform.xy +
    /// transform.zw`, wrapped (sampler address mode 0).
    pub(super) fn generate(&self, index: f32, transform: [f32; 4]) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [0.0; 4];
        }
        let (u, v) = (
            index * transform[0] + transform[2],
            index * transform[1] + transform[3],
        );
        let column =
            ((u * self.width as f32).floor() as i64).rem_euclid(self.width as i64) as usize;
        let row = ((v * self.height as f32).floor() as i64).rem_euclid(self.height as i64) as usize;
        self.texels
            .get(row * self.width + column)
            .copied()
            .unwrap_or([0.0; 4])
    }
}

/// `globals\global_render_texture`, as the array the pixel shader samples:
/// each level's layers one after another, RGBA.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheapRenderTexture {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) layers: usize,
    pub(crate) levels: Vec<Vec<u8>>,
}

impl CheapRenderTexture {
    fn read(reader: &TagReader, path: &str) -> Option<Self> {
        let tag = reader.load(path, "bitmap", b"bitm").ok()?;
        let bitmap = blam_tags::bitmap::Bitmap::new(&tag).ok()?;
        let image = bitmap.image(0).filter(|image| image.is_array())?;
        let format = image.format().ok()?;
        let (width, height, layers) = (image.width(), image.height(), image.depth());
        let levels = (0..image.mipmap_levels())
            .map(|level| {
                let (across, down) = ((width >> level).max(1), (height >> level).max(1));
                let mut rgba = Vec::new();
                for layer in 0..layers {
                    let bytes =
                        super::super::super::materials::array_layer_bytes(&image, layer, level)?;
                    let decoded = blam_tags::bitmap::decode::decode_to_rgba8(
                        format,
                        across,
                        down,
                        bytes,
                        bitmap.p8_palette(),
                    );
                    rgba.extend(decoded.ok()?);
                }
                Some(rgba)
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            width: width as usize,
            height: height as usize,
            layers: layers as usize,
            levels,
        })
    }
}

/// A `cheap_particle_type_library` type, as its tag has it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct CheapTypeTag {
    pub(super) drag: f32,
    pub(super) gravity: f32,
    pub(super) turbulence: f32,
    pub(super) depth_range: f32,
    pub(super) elasticity: f32,
    pub(super) death: f32,
    /// `change type`, none below 0.
    pub(super) change: i64,
    /// `orientation` `velocity`, else `screen facing`.
    pub(super) velocity_facing: bool,
    /// `color0`'s red, green, blue and alpha.
    pub(super) color: [f32; 4],
    pub(super) intensity: f32,
    pub(super) fade_start: f32,
    /// `size`'s upper bound; Reach never reads the lower.
    pub(super) size: f32,
    /// Halo 4's: `size`'s lower bound, and the `turbulence type` it takes.
    pub(super) size_lower: f32,
    pub(super) turbulence_type: i64,
    pub(super) stretch: f32,
    pub(super) texture: i64,
    pub(super) y_scale: f32,
}

impl CheapTypeTag {
    fn read(element: TagStruct<'_>) -> Self {
        let real = |name: &str| element.read_real(name).unwrap_or(0.0);
        let color = match element.field("color0").and_then(|field| field.value()) {
            Some(TagFieldData::RealArgbColor(color)) => {
                [color.red, color.green, color.blue, color.alpha]
            }
            _ => [0.0; 4],
        };
        let index = |name: &str| element.read_int_any(name).map_or(-1, |index| index as i64);
        Self {
            drag: real("drag"),
            gravity: real("gravity"),
            turbulence: real("turbulence"),
            depth_range: real("depth_range"),
            elasticity: real("elasticity"),
            death: real("death"),
            change: index("change type"),
            velocity_facing: element
                .field("orientation")
                .is_some_and(|field| field.enum_is("velocity")),
            color,
            intensity: real("intensity0"),
            fade_start: real("fade start"),
            size: element.read_real_bounds("size").upper,
            size_lower: element.read_real_bounds("size").lower,
            turbulence_type: index("turbulence type"),
            stretch: real("motion blur stretch"),
            texture: index("texture"),
            y_scale: real("texture y scale"),
        }
    }

    /// `type-library-compile` (tool 0x1403C60C0): the type's five rows of
    /// the type texture, as type `own` of a library with `textures` maps.
    /// No change of type is a change to itself, a texture out of range is
    /// the first, and a screen-facing type's y scale is negated.
    ///
    /// Halo 4's (`halo4`) are laid out as its shaders read them
    /// (`cheap_particles_update.fx`, `cheap_particles_render.fx`): the
    /// turbulence type's place in an array of `k_maxTypes` in `PHYSICS.w`,
    /// the depth range in `COLLISION.x`, and `size`'s bounds and the fade in
    /// row 3. No Halo 4 tool was at hand; the kit's shipped type texture
    /// agrees (`kit_halo_4s_type_table_matches_the_shipped_type_texture`).
    pub(super) fn compiled(&self, own: usize, textures: usize, halo4: bool) -> CheapType {
        let texture = usize::try_from(self.texture)
            .ok()
            .filter(|texture| *texture < textures)
            .unwrap_or(0);
        let change = usize::try_from(self.change).unwrap_or(own) as f32;
        let fade = (1.0 - self.fade_start).clamp(1e-4, 1.0).recip();
        let facing = if self.velocity_facing { 1.0 } else { -1.0 };
        let [red, green, blue, alpha] = self.color;
        let intensity = self.intensity;
        let mut rows = [[0.0; 4]; 5];
        rows[COLOR0] = [red * intensity, green * intensity, blue * intensity, alpha];
        let place = (texture as f32 + 0.5) / textures.max(1) as f32;
        rows[RENDER] = [place, self.size, self.stretch, facing * self.y_scale];
        if !halo4 {
            rows[PHYSICS] = [self.drag, self.gravity, self.turbulence, self.depth_range];
            rows[COLLISION] = [0.0, self.elasticity, self.death, change];
            rows[FADE] = [0.0, 0.0, 0.0, fade];
            return CheapType { rows, texture };
        }
        let turbulence = usize::try_from(self.turbulence_type)
            .unwrap_or(0)
            .min(MOST_TURBULENCE_TYPES - 1);
        let array = (turbulence as f32 + 0.5) / MOST_TURBULENCE_TYPES as f32;
        rows[PHYSICS] = [self.drag, self.gravity, self.turbulence, array];
        rows[COLLISION] = [self.depth_range, self.elasticity, self.death, change];
        rows[FADE] = [self.size_lower, self.size, 0.0, fade];
        CheapType { rows, texture }
    }
}

/// A type as the shaders read it: its rows of `global_type_texture`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CheapType {
    pub(super) rows: [[f32; 4]; 5],
    /// The render texture's layer. The shaders take `RENDER.x · count − 0.5`
    /// through `vs_array_texture_parameters`, whose values were not read
    /// [inferred: the layer `RENDER.x` was baked from].
    pub(super) texture: usize,
}

/// `CheapParticleTurbulenceType::k_maxTypes`, the layers of Halo 4's
/// turbulence array.
pub(super) const MOST_TURBULENCE_TYPES: usize = 8;

/// Halo 4's `cheap_particles_update.fx`: the turbulence type's array layer,
/// `PHYSICS.w · layers − 0.5`, and its transform, `(int)(4 · PHYSICS.w)`.
/// The two agree only for type 0; past it the transform is another type's
/// (type 1 and 3 take 0 and 1), as the game has it.
pub(super) fn turbulence_of(place: f32) -> (usize, usize) {
    let layer = (place * MOST_TURBULENCE_TYPES as f32 - 0.5)
        .round()
        .max(0.0) as usize;
    (layer, (4.0 * place).max(0.0) as usize)
}

/// A kind of turbulence: the map its push is read from, and `turb du dt`,
/// `turb dv dt`, `turb du dp` and `turb dv dp`.
#[derive(Debug)]
pub(super) struct CheapTurbulence {
    pub(super) texels: CheapTexels,
    pub(super) rates: [f32; 4],
    /// Halo 4's push spans -1 to 1 of what the map holds
    /// (`GenerateRandomInRange(…, -1, 1)`); Reach's is the map's own.
    pub(super) centered: bool,
}

impl CheapTurbulence {
    fn read(reader: &TagReader, holder: TagStruct<'_>, centered: bool) -> Option<Self> {
        let path = holder
            .read_tag_ref_path("turbulence")
            .filter(|path| !path.is_empty())?;
        Some(Self {
            texels: CheapTexels::read(reader, &path)?,
            rates: ["turb du dt", "turb dv dt", "turb du dp", "turb dv dp"]
                .map(|name| holder.read_real(name).unwrap_or(0.0)),
            centered,
        })
    }

    /// The push at ring slot `index`, by `transform`.
    pub(super) fn push(&self, index: usize, transform: [f32; 4]) -> [f32; 4] {
        let sample = self.texels.generate(index as f32, transform);
        if self.centered {
            sample.map(|value| value * 2.0 - 1.0)
        } else {
            sample
        }
    }
}

/// A `cheap_particle_type_library`, with the maps the shaders bind from it.
/// The game binds the one `rasterizer_globals` names (`shader_setup_type_library`,
/// the 360's 0x82DB2B50); every kit emitter names the same
/// `globals\global.cheap_particle_type_library`, so the emitter's stands in.
#[derive(Debug)]
pub(crate) struct CheapLibrary {
    pub(super) names: Vec<String>,
    pub(super) types: Vec<CheapType>,
    pub(super) random: CheapTexels,
    /// Reach's one turbulence; Halo 4's `turbulence types`.
    pub(super) turbulence: Vec<CheapTurbulence>,
    pub(super) render: Option<CheapRenderTexture>,
    /// Halo 4's: per-type turbulence, its own rows, sizes and exposure.
    pub(super) halo4: bool,
}

impl CheapLibrary {
    pub(super) fn read(reader: &TagReader, path: &str) -> Option<Self> {
        let tag = reader
            .load(path, "cheap_particle_type_library", b"cptl")
            .ok()?;
        let root = tag.root();
        let textures = elements(root, "textures").len();
        let types = elements(root, "types");
        let map = |name: &str| root.read_tag_ref_path(name).filter(|path| !path.is_empty());
        let halo4 = reader.source.game() == Some(GameId::Halo4);
        // Halo 4 binds `turbulenceTexture`, an array the tool builds from
        // these; the kit's is stale (8 layers for 4 types, none matching),
        // so each type's own map stands in [inferred].
        let turbulence: Vec<CheapTurbulence> = if halo4 {
            let kinds = elements(root, "turbulence types");
            kinds
                .iter()
                .map(|kind| CheapTurbulence::read(reader, *kind, true))
                .collect::<Option<_>>()?
        } else {
            vec![CheapTurbulence::read(reader, root, false)?]
        };
        Some(Self {
            names: types
                .iter()
                .map(|element| element.read_string_id("name").unwrap_or_default())
                .collect(),
            types: types
                .iter()
                .enumerate()
                .map(|(at, element)| CheapTypeTag::read(*element).compiled(at, textures, halo4))
                .collect(),
            random: CheapTexels::read(reader, &map("random")?)?,
            turbulence,
            render: map("render_texture").and_then(|path| CheapRenderTexture::read(reader, &path)),
            halo4,
        })
    }

    /// Row `row` of type `kind`. The texture is 256 types wide, and a type
    /// past the library's reads nothing.
    pub(super) fn row(&self, kind: usize, row: usize) -> [f32; 4] {
        self.types
            .get(kind)
            .map_or([0.0; 4], |found| found.rows[row])
    }

    pub(crate) fn render_texture(&self) -> Option<&CheapRenderTexture> {
        self.render.as_ref()
    }

    /// Whether it is Halo 4's, which lights by its own exposures.
    pub(crate) fn halo4(&self) -> bool {
        self.halo4
    }
}
