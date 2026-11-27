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
8. **Options by name, never by number.** Tags carry their own layouts, and layouts add and drop
   options (the newer Halo 3 maps' sets have a third render flag, "don't render on low settings";
   type flag bits differ between layouts). Every flag and enum is read by its option name from the
   tag's own layout (`has_flag`, `read_enum_name`), `render shader` into a typed variant by name.
   An unknown name gets a notice, not a guess. So no definitions change is needed; Halo 2's
   `groups` bytes 1 and 2 are the class and permutation index whatever the tool calls them.
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

## Progress

- Halo 3 and ODST: done (loader, layout, Solid/Textured, Game Shaders instanced, wind, cards).
- Reach: Game Shaders through `shaders\decorators\*`, with `decorators\decorators.h` shimmed (the
  alpha test, 0.5) and the packed `POSITION1` fed as an integer.
- Halo 4: Game Shaders through each `explicit_shaders\decorators\*.material_shader`'s own
  source, its `RGBk` output decoded and exposed as its other passes are; positions at the vertex
  compression's constant scale of 5 (`|q|² = scale / 5`).
- Halo 2: loader, records and geometry as the cache builder builds them; Solid/Textured with the
  `shad`s cut out on their base map; Game Shaders through the dx9 classifications of
  `decorator.vertex_shader`/`decorator_decal.vertex_shader`, translated (a `Decorator(slot)` DX9
  stage carrying its source), the placements in a stream of their own (`POSITION0`, `COLOR0`,
  `TEXCOORD4`/`5`), the `c[]` constants, a model placement's `c203`–`c207` per batch, the colour of
  `sub_6AEF00`, and the decorator pass's only implementation in the lightmap layers.
- Halo CE: loader, Sapien's lattice and frame pick, billboards baked for Solid/Textured with the
  plate cut out; Game Shaders through the kit's `detail_object_type0`/`type1.vsh` and
  `modulate.psh`, translated (a `DetailObject` DX9 stage), the shared decorator stream (frame,
  type and corner in `COLOR1`), the vertex shaders' `c[]` tables, Sapien's colour in RGB565, and the
  game's alpha blend.

- Cards: Decorator Set, Types, Layout, Distance (the game's fade, toggled), Lighting (the stand-in
  light's level), Wind (sway).
- Reach's and Halo 4's levels: every valid model read (a further level draws the nearest's subpart
  by index, as its placements are named otherwise), the postprocess's repairs, cull blocks gridded by
  `cull block size`, each game's level choice and decimation, and the stencil stipple from
  `default bitmaps[9]`. The gen3 decorators now draw their strips as the game does
  (`TRIANGLE_STRIP`, counting each strip from vertex 0 for `instance_data`).
- Reach's and Halo 4's *align random*: a type's up turned to a random direction after its spin and
  tilt, unless it also aligns to the normal, which the game takes first.

## Reach's and Halo 4's detail levels (researched 2026-11-27)

From Reach's 360 tag build (`render_decorators`, `c_decorator_lod_renderer`) checked against MCC's
`reach_tag_test`, and Halo 4's 360 `midnight_tag_debug` checked against MCC's `halo4_tag_test`.

- **Postprocess** (both): the four render models are compacted to the valid ones, each moving its
  transition with it. Then each valid level's `end` at or short of the last one's becomes the last
  end + 8; a `start` short of the last end or past its own end becomes `(last end + 3 · end) / 4`;
  `scale = −1 / max(end − start, 1e-4)`, `offset = 1 − start · scale`. Decimation's start and end
  become a scale and offset the same way (`offset = −end · scale`).
- **Blocks:** the game culls and picks a level per cull block (the level's decorators grouped by
  `cull block size`, clamped to 0.5–100), by its bounding sphere.
- **Reach:** `d = max(0, |eye − centre| − radius) · 1.7186729 · fov / throttle`, where `fov` is
  `tan(fov / 2)` (1.7186729 makes 60° whole) and the throttle is the rasterizer globals' default
  performance throttles' first element's `decorator fade dist scale` (0 draws none). A block is
  culled where the last level's `d · scale + offset ≤ 0`. The first level `k` with
  `f = d · scale + offset > 0` draws with the stipple at `min(f, 1)`; where that is short of whole and
  a next level exists, the next draws with the inverse stipple. Level 0 with *use decimation* draws
  only the strip's first `keep · count` vertices, `keep = min(1, x⁴ / √fov)` for
  `x = max(0, d · scale + offset)` of the decimation, and fades out past the count kept 3 units
  further (`instance_data = (count, kept, kept further, 0.5 / max(kept − kept further, 1))`, the
  shader's `1 − saturate((vertex − z) · w)` through the alpha test).
- **Halo 4:** the distance is to the block's centre, scaled by `throttle / max(0.785, 1.7186729 ·
  fov)`. A block is culled past `(radius + end point3 · scale) · (1 − early cull)` (none where that
  end is 0). Slot 0 with no end draws whole. Otherwise the first slot `k` the eye is short of the end
  of draws at `1 − clamp((d − start) / (end − start))`, and the next with the inverse where the eye is
  past its start. Decimation: `keep = min(1, max(decimate to / 100, clamp(max(d − start distance,
  1e-10)^(−0.667 · speed) / √fov)))`, and 10 units further for the fade; with the kit's hidden
  fields at 0 that keeps everything.
- **Stipple:** `rasterizer_stipple_initialize` writes `default bitmaps[9]` into the stencil, one
  texel a pixel: Reach's `dither_pattern4` (2 × 2: 0 3 / 2 1), Halo 4's `dither_pattern3` (4 × 4
  Bayer, 0–15). `rasterizer_stipple_set_fade_byte(b, 160, inverse)` tests against it: Reach
  `ref = (3b + 253) / 254` (0–4), mask 7, GREATER (inverse LESS_EQUAL); Halo 4 `ref = b >> 4`,
  mask 15, GREATER_EQUAL (inverse LESS_EQUAL). A plain draw is skipped where its byte is 0 (255 for
  the inverse).

## Seen in the preview, explained

- Reach's `tree_pine_forge_gulch` (static billboard) is a card lying flat in its model (0.01 thick,
  facing −Z, its height along +Y) with only *random rotation* and *align to normal*. Nothing in
  `place_randomized_decorator` stands it up: its spin is about up, its tilt within the type's
  range, and *align to normal* turns the model's up to the ground's normal, which on flat ground is
  up. *Rotate x axis down* (which it does not set) only turns the model's x down a slope. So on flat
  ground the game lays it flat as the preview does. It would stand only where the brush paints it on
  a steep face (inferred: no kit scenario places it).

## Not settled by the research

- **Gen3:**
  - the exact distribution of the colour 0/1/2 mix (barycentric, not otherwise visible);
  - the random rotation axis in `place_randomized_decorator`;
  - whether PC samples the gust texture signed;
  - Reach's `early cull` (no read found);
  - Halo 4's 5.0 compression constant;
  - how a level's build groups its decorators into cull blocks and sizes their spheres: the preview
    grids its patch by `cull block size` and bounds each cell's placements;
  - which screen corner the stipple's tile starts from (the preview tiles from GL's, bottom left).
  - whether Halo 3's lightmapper writes a sun visibility into a *still + sun light only* set's motion
    byte, which that shader reads as the sun's strength (`motion_scale * sun_color * N·L`). The
    scenario keeps the brush's motion, 0 for every sun set's wind range (Citadel's 192 snowy pines all
    store 0), and the kit's BSPs leave the instance buffer to the cache build, so the preview packs the
    brush's motion and those sets take no sun; they draw by their baked light alone.
- **Halo 2:** what the lightmapper writes into the placement's accuracy float.
- **Halo CE:** Sapien's random seed (the preview picks its own); the exact UV weights of its ground
  sample (irrelevant without a level).
