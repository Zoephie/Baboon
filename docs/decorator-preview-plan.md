# Decorator previews

Status: **plan**, 2026-11-26. Nothing built. Research notes, with addresses and excerpts:
the session scratchpad's `decorators/{ce,h2,h3,reach,h4,baboon}/findings.md`. Sources: the symbolled
databases (HCEX 360, H2 2003 xbe, H2 MCC tag_test, H3 dllcache, Reach and H4 360 tag_debug, H4 PC
tag_test), the MCC kits' shader source, and our own preview code.

A `decorator_set` gets its own Model Preview, built the way the object preview is (the same
viewport, header, Game Shaders and snapshot), with cards of its own. Shading is translated from
each game's **shader source**; the compiled shader tags are only a cross-check.

## What a decorator set is, by game

**Halo 3, ODST, Reach and Halo 4: one design.**
- **Geometry:** a render model (Reach and Halo 4: four LOD models) with one *unindexed
  triangle-strip* mesh of vertex type `decorator`. Each decorator type is one subpart, all padded to
  the same length. A type's `mesh` names the render model's instance placement it draws (resolve by
  name).
- **Texture:** the set's own `texture` bitmap is the only material. The models' materials are
  unused (Halo 4: `shaders\invalid`).
- **Shader:** one of a fixed table of explicit shaders, chosen by `render shader` (an index into
  `rasterizer_globals`' explicit shaders). Each is a tiny wrapper of `#define`s around one decorator
  shader.

| game | source | variants (`render shader` → defines) |
|---|---|---|
| Halo 3 | `shaders/decorators_hlsl` + includes (`hlsl_include` tags), complete at `DX_VERSION=11` | 0 default (WIND, DYNAMIC_LIGHTS), 1 no_wind (DYNAMIC_LIGHTS), 2 static, 3 sun (DOMINANT_LIGHT), 4 wavy (WAVY, DYNAMIC_LIGHTS), 5 shaded (DYNAMIC_LIGHTS, SHADED_LIGHT) |
| ODST | as Halo 3, with slightly different source files (diff them; branch where they differ) | as Halo 3 |
| Reach | `shaders/decorators/decorators` + includes. Only `decorators\decorators.h` is missing (the alpha-test threshold, 0.5 from the compiled shader): one shim line | 0 wind_billboard, 1 static_billboard, 2 mesh, 3 far_object, 4 underwater (WAVY), 5 volume (SHADED_LIGHT), 6 wind_volume |
| Halo 4 | the 36 source files in each `explicit_shaders/decorators/*.material_shader`'s source block; complete for DX11 | as Reach |

- **Per-instance data (16 bytes):**
  - position, compressed into the cluster block;
  - type index and motion (wind amount) byte;
  - a quaternion whose length carries the scale;
  - a baked light colour (Halo 3/ODST RGBE-ish `rgb·2^(a/4−31.75)`, Reach RGBE, Halo 4 RGBk).
- **Draw:** the shader reads instances from its own vertex inputs (`TEXCOORD1–3`, Halo 4
  `POSITION1/COLOR1/COLOR2/NORMAL1`) and is drawn instanced: one strip of `count` vertices per
  type, starting at `type·count`.
- **Wind** (default and wind variants):
  - The vertex shader samples a gust noise texture at the instance position, scrolled by
    `wind_data`.
  - It bends by `wind_data2` (bend, oscillation, time), scaled by motion and height, keeping the
    length.
  - The constants come from a `.wind` tag (`globals/defaults/default_wind.wind`, gust noise
    `random4_warp`).
- **Wavy:** a sine of position, height and time from the set's wavelength, speed and frequency.
- **Lighting:**
  - Each instance's colour is baked at lightmap time: the type's colour 0/1/2 mixed, tinted toward
    the ground, times the lightmap's SH at a 10-point sampling pattern.
  - The shaders add up to 8 simple lights and, for some variants, a sun term.
  - *Shaded* (volume) variants use the vertex's offset from the instance centre as a normal, with
    `shaded dark/bright` as contrast.
- **Fade and LOD:**
  - Halo 3/ODST: an alpha-test dissolve between `start fade` and `end fade`.
  - Reach/Halo 4: whole cluster blocks pick an LOD by distance and cross-fade to the next with a
    screen-door **stencil stipple**; LOD0 can decimate.
- **Postprocessed fields:** the kit tags store every postprocessed field as 0 (LOD scale/offset,
  translucency A/B/C, decimation). The preview recomputes them, as the game's postprocess does.

**Halo 2: its own design, in the set.**
- **Geometry:**
  - Classes (model, floating decal, projected decal, screen-facing quad, axis-rotating quad, cross
    quad) hold permutations.
  - Models come from the set's own raw vertices and indices; its cached data is empty in every kit
    tag.
  - Quads and floating decals are sprite quads, built from the shader's base-map bitmap (sequence =
    permutation index).
- **Shading:** ordinary `shad` shaders on three templates, `decorator_tex`, `decorator_decal_paint`
  and `decorator_decal_paint_multiply`. They take dedicated vertex shaders
  (`decorator.vertex_shader` slots 7 screen, 8 axis, 0 cross, 1 model; `decorator_decal` slot 0)
  whose HLSL source is in the tags, and pixel passes whose `.psh` source we already translate.
- **Colour:** no lightmap texture or lights. Colour comes in through D0 or `c207`:
  `lerp(tint1, tint2, t) · (tint·bmt + 1−bmt) · (lightmap·lmt + (1−lmt)·0.5)`. Models also get
  `max(0, N·L)` weighted by an accuracy term.
- **Fade:** the permutation's fade distance (close, medium or far) shrinks quads and models to
  nothing, over 5.3–8, 10.7–16 or 16–24 world units.
- **Wind:** none. `wind scale` is never read.

**Halo CE: detail objects** (`detail_object_collection`), not decorators.
- **Geometry:** a bottom-anchored billboard per instance, screen-facing or viewer-facing
  (cylindrical about Z), sized by its sprite's texels × `size`.
- **Shading:** sprite-plate texture × the instance's baked 565 colour, faded between near and far,
  no lighting.
- **Source:** the kit ships the vertex shaders (`shaders/vsh/detail_object_type0/1.vsh`).
- **Sapien** (MCC `haloce_mcc_sapien.exe`) settles the rest:
  - `first frame` is the sum of the earlier sequences' sprite counts; `frame count` is the
    sequence's own.
  - Each instance's colour is `(ambient·a + (1−a)·L) · min(1, mix·((1−ov)·G + ov))`:
    - `mix` is min→max colour at one random `t`, in RGB or HSV. In HSV, *more colours* takes the
      long way round the hue wheel.
    - `L` is the lightmap under the instance, plus 0.1 (0.5 grey without one).
    - `G` is the ground's base map (0.5 grey without one).
    - `ov` is the colour override and `a` the ambient alpha.
  - The result is truncated to RGB565.
  - The brush lays instances on a fixed 8/255-unit lattice, kept at density %. Each patch has one
    type; frames are random within its sequence.
  - `global_z_offset` is added at load.

## What none of them have in a preview: a level

Every game bakes a decorator's placement and light into the level. A standalone preview has to make
its own:

- **Placement.** Two layouts, both seeded so a set always looks the same:
  - *Scatter*: a ground patch filled the way the game's brush fills it (`place_randomized_decorator`):
    - types picked by placement weight;
    - spaced by minimum distance;
    - random yaw, tilt in range, hover along the normal, scale in range;
    - a random mix of colours 0/1/2;
    - motion in the wind range.

    Hanging sets (light sampling pattern *hanging*) hang from a ceiling plane.
  - *Types*: a row per type (Halo 2: per class and permutation), columns sampling its scale, tilt and
    colour ranges.
- **Light.** Each instance's baked colour becomes its colour mix times an ambient level (Halo CE:
  Sapien's own formula), with the game's own defaults for the rest:
  - Halo 3's sun is (0,0,1), white;
  - no simple lights;
  - fog off;
  - exposure 1.

  Halo 2 gets white placement colours and a sun-like light direction. Halo CE gets Sapien's formula,
  with a lighting level standing in for the lightmap (default 0.6) and a ground colour for the base
  map (default 0.5 grey).
- **Wind.** The default wind tag, or any `.wind` tag picked, at the preview clock.
- **Ground.** A ground plane under the patch, for scale and for Halo 2's floating decals.

## Decided (2026-11-26)

- **Order:** Halo 3, ODST, Reach and Halo 4 first, then Halo 2, then Halo CE.
- **GPU instancing** for those four games, with Game Shaders on: one instanced strip draw per type.
- **Instance data:** the game's own packed 16-byte instance, encoded per game exactly as the game
  does. The translated shaders run unchanged, and a round-trip test proves each game's packing.
- **Game Shaders off** (Solid, Textured): instances are baked on the CPU into one vertex buffer and
  drawn like any model. This works on every driver (the shared path falls back to GLSL 1.20) and
  keeps selection, wireframe and snapshot as they are.
- **Picking and highlight per type:** hovering a type in the Types card, or clicking a decorator in
  the viewport, highlights every instance of that type.
- **Instance count: no cap.** The count box takes any number, with a warning past a large count, as
  the snapshot's size does.

## Proposed cards

Instead of Permutations, Weapons and Effects:

- **Decorator Set:**
  - the render shader variant (named), render flags and texture;
  - Halo 3/ODST: start/end fade;
  - Reach/Halo 4: the LOD models and their cross-fade ranges.
- **Types:** each type with a visibility toggle and *solo*; its mesh, scale/tilt/wind/hover ranges
  and colour swatches. Halo 2: classes with their permutations. Halo CE: types with frame thumbnails.
- **Layout:**
  - Scatter or Types;
  - patch size, density or count, seed;
  - ground plane on or off;
  - hanging.
- **Wind** (variants that move):
  - the wind tag, or the default;
  - direction and speed overrides;
  - for wavy sets, the wave parameters as the tag has them;
  - pause.
- **Distance** (fades and LOD):
  - a viewing-distance slider that runs the game's fade, or the LOD choice with its stipple, so a set
    can be seen at range;
  - Reach/Halo 4: force an LOD.
- **Lighting:** ambient level and sun direction, standing in for the baked light.

The Materials card edits the set's texture-bearing tag where there is one (Halo 2's `shad`s). For
the later games, the shader is fixed and the only material is the texture.

## Building it

1. **The loader.**
   - A `decorator_set` (Halo 2–4) and a `detail_object_collection` (CE) become previewable
     groups. Add them to the predicate, a `load_tag_preview` branch, and a loader module, following
     the imposter's pattern.
   - Gen3: read each LOD render model's decorator mesh with blam-tags (`decode_decorator`,
     `extract_decorator_subparts` exist and are unused). Expand each subpart's strip to
     `raw_vertices[raw_indices[i]]`.
   - Halo 2: decode the set's raw vertices, indices and models. blam-tags' Halo 2 readers are private
     and nothing reads this inline geometry, so this needs an engine helper.
2. **Instances.**
   - Build placements on the CPU from the layout (seeded), each with position, rotation, scale,
     type, motion and colour.
   - Gen3: upload them to a per-instance vertex buffer with attribute divisors, and draw each type
     with `draw_arrays_instanced`. The preview has no mesh instancing today; it bakes instances into
     vertices. The decorator shaders expect instance inputs, so instancing keeps them as the game
     wrote them.
   - Halo 2: bake sprite and decal quads into vertices exactly as its cache builder does. Draw
     models one per instance with the model constants, as the game does.
3. **Shaders, from source.**
   - Gen3: `Gen3Sources::compile` with each variant's defines and `default_vs`/`default_ps`.
     Halo 4's sources come from the `material_shader` source block, which its material code already
     reads. Reach gets the one-line `decorators.h` shim.
   - New inputs:
     - the instance attributes;
     - decorator constants (`instance_compression_*`, `instance_data`, `LOD_constants`,
       `translucency`, `wave_flow`, `sun_direction`, `sun_color`, `contrast`);
     - the wind constants and texture;
     - simple lights at 0.
   - Halo 2: translate `decorator.vertex_shader`'s HLSL code (asm-converted vs 2.0, `float4 c[208]`)
     through `core/hlsl`, instead of a hand-written stage. The `.psh` passes go through the existing
     translator.
   - Halo CE: the `.vsh` source, if our CE path can take it; otherwise say so before writing a stage.
4. **Wind.** Read a `.wind` tag (direction, speed, bend, oscillation, frequency functions, gust
   size, gust bitmap) and compute `wind_data`/`wind_data2` each frame from the preview clock, as
   `s_wind_definition::update` does. Bind the gust bitmap to the vertex shader.
5. **Fades and LOD.**
   - Recompute the postprocessed fields: LOD scale/offset with the start/end repair rules,
     translucency A/B/C, and Halo 3's `LOD_constants`.
   - Halo 3/ODST: the alpha-test dissolve.
   - Reach/Halo 4: the stencil stipple between LODs, and decimation. The viewport's distance is
     the camera's; the Distance card can push it.
6. **Cards** as above, from the loaded set's data. Per-type visibility, layout and wind settings are
   preview state (the scenario selection's pattern), and a layout change reloads the instances
   only.
7. **Halo CE detail objects:**
   - Billboard quads expanded on the CPU, or in the translated `.vsh`;
   - the sprite plate's sequences, with first frame and count computed;
   - Sapien's colour formula and RGB565 truncation, over a stand-in lightmap level and ground
     colour;
   - its lattice and density for the Scatter layout, one type per patch.
8. **Definitions fixes:**
   - Halo 3's decorator type flags lack bit 2 ("don't render on low settings", used by 6 kit
     tags).
   - Halo 2's `scenario_decorators_resource` groups call byte 1 `decorator_type` and byte 2
     `shader index`; they are the class and permutation index.

   Edited in the group JSON directly.
9. **Docs and tests.**
   - A shot test per game over kit sets, in each variant: wind moving with time, wavy, shaded,
     LOD stipple, a Halo 2 sprite and decal set, and a CE grass collection.
   - Unit tests for:
     - the placement maths;
     - the postprocessed-field recomputation (against the game's formulas);
     - the 16-byte instance encoding round trip;
     - Halo 2's placement and colour formulas.
   - The snapshot and Game Shaders off (Solid and Textured draw the geometry with the texture) work
     as for other previews.

## Open questions for the owner

See the proposals that accompany this plan.

## Not settled by the research

- **Gen3:**
  - the exact distribution of the colour 0/1/2 mix (barycentric, not otherwise visible);
  - the random rotation axis in `place_randomized_decorator`;
  - whether PC samples the gust texture signed;
  - Reach's `early cull` (no read found);
  - Halo 4's 5.0 compression constant.
- **Halo 2:** what the lightmapper writes into the placement's accuracy float.
- **Halo CE:** Sapien's random seed (the preview picks its own); the exact UV weights of its ground
  sample (irrelevant without a level).
