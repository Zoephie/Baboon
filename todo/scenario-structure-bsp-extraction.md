# scenario_structure_bsp extraction — Phase 1 analysis

**Status:** Phase 1 analysis complete; **Patch 1 implemented** in `blam-tags` on
2026-08-04 (see [Implementation status](#implementation-status)). Written 2026-08-04.

**Fixture:** `Meteorite/Content/Tags/Levels/Halo1/Solo/C10/_Generated_/level_a-scenario_structure_bsp.ubulk`,
33,018,844 bytes, extracted from `pakchunk310-Windows.utoc` of the Steam install at
`D:\SteamLibrary\steamapps\common\Halo Campaign Evolved`. The task brief left the local
path blank; the file did not exist on disk, so it was pulled out of the paks with a
throwaway tool built against `blam_tags::iostore::IoStoreArchive` (read-only; neither repo
was modified). See [Reproducing the fixture](#reproducing-the-fixture).

---

## Summary of findings

Both hypotheses in the brief were tested against the bytes. They split:

| # | Hypothesis | Verdict |
|---|---|---|
| 1 | CE's BSP layout is closer to Halo 4's than to Reach's | **Refuted.** CE's BSP is a *strict, order-preserving subset of Reach's*, with zero type mismatches on all 50 shared top-level fields. It is further from H4 than from Reach on every measure taken. |
| 2 | H4-era BSPs drop the sealed world and put most geometry in instanced blocks | **Confirmed for CE**, though for a reason unrelated to H4. CE's sealed world is vestigial (1 cluster, 0 portals, 51 surfaces) while 3,492 instance placements over 775 definitions carry 206,652 collision surfaces. This was not verified for Halo 4 itself — see [Open questions](#5-open-questions--blockers). |

The more consequential finding is that **neither hypothesis identifies the actual blocker**:

- **Parsing already works, completely.** `definitions/haloce_evolved/scenario_structure_bsp.json`
  already exists and is already correct. Parsing the fixture through it produces output
  **byte-identical** to parsing it through the tag's own embedded `blay` schema (`diff` = 0 lines).
  No definitions work is needed.
- **The CE BSP contains no render geometry at all.** `render geometry/per mesh temporary` is a
  block of **0 elements** (Reach: 470). Every one of the 776 meshes has `index buffer index = -1`.
  The visual meshes are not in the tag — they live in Unreal assets, which is Chimp's domain.
- **What the tag does carry is a complete collision model**, and the existing exporter drops
  ~99.9% of it at two `continue` statements.

So this is not a "new engine generation" problem. It is a narrower one: the gen3 ASS exporter
assumes render buffers are inline, and CE is the first supported source where they never are.

---

## 1. Prior art survey

### Definitions (submodule — separate ownership)

`scenario_structure_bsp.json` ships for all eight games:

| Game | root fields | version |
|---|---|---|
| `haloce_mcc` | — | (classic) |
| `halo2_mcc` | — | — |
| `halo3_mcc` / `halo3odst_mcc` | — | — |
| `haloreach_mcc` | 70 | 5 |
| `halo4_mcc` | 82 | 5 |
| `halo2amp_mcc` | 82 | 5 |
| **`haloce_evolved`** | **50** | **5** |

`definitions/haloce_evolved/scenario_structure_bsp.json` (196 KB) is present and correct
*today*. Confirmed empirically:

```
blam-tag-shell --game haloce_evolved inspect level_a.scenario_structure_bsp  >  A
blam-tag-shell                        inspect level_a.scenario_structure_bsp  >  B   # embedded blay
diff A B   →   0 lines
```

Note `halo4_mcc/scenario_structure_bsp.json` is 20 MB against Reach's 285 KB and CE's 196 KB —
two orders of magnitude larger, presumably heavily inlined. Worth knowing before touching it,
but not on this path.

### Crate: how a BSP is currently walked

- **`blam-tags/src/extract/geometry.rs:60`** — `scenario_geometry()`, the scenario-level walk.
  Iterates `structure bsps[]`, resolves each `sbsp` + its paired `stli`, emits one file per BSP.
  Game dispatch is at `:67-68`:
  ```rust
  let is_ce = Game::of(scenario) == Game::Halo1;
  let is_h2 = Game::of(scenario) == Game::Halo2;
  ```
  - `:112` — Halo 1 branch → render + collision JMS via `emit_ce_bsp_jms()` (`:222`).
  - `:140-149` — everything else → ASS, v2 for H2 (`from_scenario_structure_bsp_h2`),
    **v7 for all of H3 / ODST / Reach / H4 / H2A** (`from_scenario_structure_bsp`).

- **`blam-tags/src/game.rs:72-78`** — `Game::of()` has only three values and derives them from
  `classic_engine()` (`blam-tags/src/file.rs:448`), which returns `None` for any MCC-container
  tag. **A CE-Evolved tag therefore classifies as `Game::Halo3`** and takes the gen3 ASS path.
  The enum doc at `game.rs:23-26` states the assumption explicitly: H3/ODST/Reach/H4/H2A "share
  `render_model` (per-mesh-temporary) geometry". CE is the first tag source where that is false.

- **`blam-tags/src/ass.rs:308`** — `AssFile::from_scenario_structure_bsp()`, the gen3 BSP → ASS
  builder. Three geometry sources:
  1. `:316` binds `render geometry/per mesh temporary` (hard-required — `MissingField` if absent).
  2. `:340-360` clusters → OBJECTs, skipping at `:356` `if (mesh_idx as usize) >= pmt.len()`.
  3. `:432-470` `resource interface/raw_resources[0]/raw_items/instanced geometries definitions`
     → OBJECTs, skipping at `:451` with the **same** `pmt.len()` guard, then
     `instanced geometry instances[]` → INSTANCE placements.
  4. `:643-700` `resource interface/raw_resources[0]/raw_items/collision bsp` → a single
     `@collision_only` MESH object, walking the edge ring via
     `crate::geometry::walk_surface_ring` (**`blam-tags/src/geometry.rs:210`**).

- **`blam-tags/src/render_geometry/hydrate.rs:1-15`** — bridges the *other* buffer-location case:
  Halo 4 X360 monolithic builds keep buffers in `cache_N` behind a `tgxc` pageable resource and
  leave `per mesh temporary` empty; this module decodes them into the author-format blocks so
  downstream code sees one shape. **This is the closest existing precedent for CE's problem** —
  but it does not apply, because CE has no GPU resource to hydrate *from* (see §2).

- **`blam-tags/src/iostore/mod.rs:1-20`** — states the CE model outright: each tag is a
  `<name>-<group_longname>.ubulk` whose bytes "**are** a byte-complete, self-describing Reach MCC
  tag file". This is a documented design assumption in the crate, and §3 confirms it holds for BSPs.

### Baboon side

- `src/app/export/geometry.rs:11-22` — `extract_geometry_for_entry()` dispatches on group tag;
  the `b"sbsp"` arm calls `AssFile::from_scenario_structure_bsp()` directly and writes one `.ASS`.
- `src/app/export/geometry.rs:371-380` — the `b"scnr"` arm delegates to
  `blam_tags::extract::geometry::scenario_geometry_to_dir()`.
- `src/app/controller.rs:3375` — `begin_extract_geometry()`, reached from
  `BrowserAction::ExtractGeometry` (`controller.rs:2993`).

Baboon adds no BSP knowledge of its own — it is a thin caller. **Nothing in Baboon needs to change
for the geometry itself to come out correctly**; it already routes `sbsp` entries to the crate.

---

## 2. Binary survey of the fixture

### Header — confirmed from the bytes

```
0030  70 73 62 73  05 00 00 00  a9 cc 61 b3  4d 41 4c 42   psbs......a.MALB
0040  21 67 61 74  00 00 00 00  90 d3 f7 01  79 61 6c 62   !gat........yalb
```

| Offset | Bytes | Meaning | Confidence |
|---|---|---|---|
| 0x30 | `psbs` | group tag `sbsp`, byte-reversed | confirmed |
| 0x34 | `05 00 00 00` | group version **5** | confirmed |
| 0x38 | `a9 cc 61 b3` | checksum `0xB361CCA9` | confirmed |
| 0x3C | `MALB` | `BLAM` little-endian ⇒ **LE, writable class** | confirmed |
| 0x40 | `!gat` | `tag!` stream marker | confirmed |
| 0x4C | `yalb` | `blay` — **embedded schema stream** | confirmed |

Field-name strings begin in plaintext at ~0xC0 (`build identifier`, `struct`, `manifest_id0`,
`long integer`, …) — the `blay` string table. **The tag is self-describing**; it parses with no
external definition at all.

`blam-tag-shell header` agrees: `Group: sbsp / Group version: 5 / Build: 1.2 / Streams: tag!`.
Only one stream — no separate resource stream.

### Top-level content — confirmed by parsing

50 root fields. Populated blocks of interest:

```
flags:                            0x0001 [has instance groups]
failed content policy flags:      0x0001 [has working pathfinding]
leaves:                           36723        super aabbs:               11077
super node parent mappings:        2048        instance kd hierarchy:  (17465 nodes)
large structure surfaces:            51        clusters:                      1
cluster portals:                      0        materials:                     9
collision materials:                 11        pathfinding data:              0
instanced geometry instances:      3492        instanced geometry instance names: 0
render geometry / meshes:           776        render geometry / compression info: 775
render geometry / per mesh temporary: 0
resource interface / raw_resources:   1
```

`world bounds`: x −54.56…−4.92, y −59.06…5.58, z −9.85…21.01.

### Where the geometry is — confirmed

`resource interface/raw_resources[0]/raw_items`:

```
collision bsp:                        1 element
large collision bsp:                  0 elements
instanced geometries definitions:   775 elements
```

The sealed-world `collision bsp[0]`: 27,108 bsp3d nodes, 2,048 supernodes, 25,756 planes,
36,723 leaves, 1,378 bsp2d references, 159 bsp2d nodes — but only **51 surfaces, 99 edges,
50 vertices**. A large spatial index over an almost non-existent hull.

Every one of the 775 `instanced geometries definitions` carries a populated `collision info`
(bsp3d nodes / planes / leaves / bsp2d references / surfaces / edges / vertices). Totals across
all 775, computed by walking the parsed tag:

```
instanced geometry definitions: 775  (775 of 775 with collision surfaces)
  total collision surfaces:  206,652
  total collision edges:     321,858
  total collision vertices:  112,803
instanced geometry instances: 3,492
render geometry: meshes=776  per_mesh_temporary=0
```

`instanced geometry instances[0]` carries a full placement: `scale`, `forward`/`left`/`up` basis
vectors, `position`, `instance definition` (block index), `flags [collidable]`, `mesh_index`,
`compression_index`, world bounding sphere, imposter distance. Placement data is complete.

### Render geometry — confirmed absent

Sampled meshes 0, 1, 100, 400, 774, 775 — **all** report `index buffer index = -1`, and all
`vertex buffer indices` entries are 0. `parts[0]` still carries counts (mesh 0: `index count 114`,
`budget vertex count 40`), so topology *sizes* are described while the buffers themselves are
not present. `per mesh temporary` has zero elements, so there is nowhere inline for them to live.
`api resource` is a null pageable, `tag_resources` and `cache_file_resources` both null.

There is no sibling chunk to recover them from. The only other `.ubulk` for this BSP in the pak
set is its lighting info:

```
pakchunk310-Windows.utoc  scenario_structure_bsp          level_a  .../level_a-scenario_structure_bsp.ubulk
pakchunk310-Windows.utoc  scenario_structure_lighting_info level_a  .../level_a-scenario_structure_lighting_info.ubulk
```

**Conclusion (confirmed):** the CE BSP tag is a *collision and placement* tag. Its render geometry
struct is a sized stub. The renderable meshes are Unreal assets.

### Material references — confirmed

`materials[0..8]` and `collision materials[0..10]` are `rmsh:` tag references to
`shaders\hard_metal_thick_for`, `shaders\soft_floodflesh`, `shaders\brittle_glass_for`,
`shaders\energy_hologram`, `shaders\default_material`, several `shaders\invalid`. These are
**physics-material names, not visual shaders** — consistent with a collision-only tag.

### Inferred (not confirmed)

- `mesh index 775` on `clusters[0]` = the last of 776 meshes ⇒ meshes 0–774 correspond 1:1 with the
  775 instanced definitions and mesh 775 is the cluster's own. Consistent with
  `compression info: 775`, but not independently verified.
- The 2,048-element blocks (`super node parent mappings`, `recursable_masks`, `in use masks`) are
  assumed to be a fixed-size supernode acceleration structure by analogy to Reach. Not decoded.

### Unknown

- Whether `instance kd hierarchy` (17,465 nodes / 21,831 hash entries) is needed for extraction, or
  is purely a runtime query accelerator. Not decoded.
- Whether any CE BSP anywhere has a non-empty `per mesh temporary`. Only `level_a` was walked in
  full; ~100 other CE BSPs exist across `pakchunk115/130/150/230/240/310/320/…`.

---

## 3. Delta vs the nearest supported generation

**Nearest match: Halo Reach — decisively, not Halo 4.**

Comparing root-struct fields of `definitions/haloce_evolved` against `haloreach_mcc` and `halo4_mcc`:

```
CE fields: 50    Reach: 70    H4: 82

CE \ Reach  (in CE, absent from Reach):  []            ← empty
CE \ H4     (in CE, absent from H4):     ['environment object palette*',
                                          'environment objects*',
                                          'pad64_01', 'pad64_last']

|CE ∩ Reach| = 50/50          |CE ∩ H4| = 46/50
CE is an ordered subsequence of Reach:  True
Type mismatches on shared names:        0
```

CE is a **strict, order-preserving, type-identical subset of Reach**. There is no field
reordering, no field-size change, and no struct misalignment. Every divergence is a *deletion*.

The 18 Reach root fields CE drops:

```
detail objects*              sky owner cluster*           sound PAS data*
marker light palette*        marker light palette index*  runtime decals*
instance imposters           decorator info               decorator sets*
decorator instance buffer!*  decals info                  preplaced decal sets*
preplaced decals*            preplaced decal geometry!*   transparent planes*
debug info*                  widget references            cheap light references
```

Every one is a **rendering** concern — decals, decorators, imposters, detail objects, transparent
planes, sky, marker lights. Exactly what you would delete if Unreal took over rendering and the
Blam tag were demoted to collision and placement. The deletions corroborate the §2 finding rather
than merely coexisting with it.

### The behavioural delta

Same binary, same code path, two inputs:

| | CE `level_a` | Reach `cex_beaver_creek` |
|---|---|---|
| ASS output | 12 mats, **1 object**, 2 instances, **198 verts, 96 tris** | 196 mats, 484 objects, 1,115 instances, **250,261 verts, 220,995 tris** |
| `per mesh temporary` | **0** | 470 (populated: e.g. pmt[100] = 1,853 raw vertices / 3,624 raw indices) |
| `raw_items` | `collision bsp: 1`, `large collision bsp: 0`, defs: 775 | `collision bsp: 0`, `large collision bsp: 1`, defs: 428 |
| collision surfaces in defs | 206,652 (775/775 populated) | 79,736 (340/428 populated) |

The single CE object is the `@collision_only` sealed-world mesh from `ass.rs:643`. The arithmetic
confirms it exactly: 51 surfaces triangle-fanned gives `198 − 2×51 = 96` triangles. **Nothing else
in the tag reached the output.**

The cause is two `continue` statements, both reading `pmt.len()`, which is 0:

- **`blam-tags/src/ass.rs:356`** — drops every cluster.
- **`blam-tags/src/ass.rs:451`** — drops all 775 instanced definitions.

Note also the `raw_items` swap: CE populates `collision bsp` where Reach populates
`large collision bsp`. `ass.rs:643` only reads `collision bsp`, which is why CE got its 51
surfaces and Reach's sealed world contributed nothing to that path. Any patch must handle both.

---

## 4. Proposed extraction path

Ownership matters here, and it lands favourably:

- **`definitions/` submodule — no changes required.** Already correct and already shipped.
- **`blam-tags` crate — all of the work.**
- **Baboon — nothing required for correctness.** Optional UX only.

### Patch 1 — collision geometry from instanced definitions ✅ *implemented*

Entirely within `blam-tags/src/ass.rs`. In `from_scenario_structure_bsp()`, when a definition is
skipped for want of render buffers, fall back to its `collision info` and emit an OBJECT from it,
reusing the edge-ring walker (`geometry.rs:210`) already used at `ass.rs:673`. Then place those
objects through the existing `instanced geometry instances[]` INSTANCE loop, which needs no change.

Also read `large collision bsp` alongside `collision bsp` at `:643` so the sealed-world path is
generation-agnostic.

Yield on the fixture: 775 objects, 206,652 surfaces, 3,492 placements — versus 1 object today.
Confined to one file; no API change; no behaviour change for H3/ODST/Reach/H4, where the render
path still hits first.

**Guard against regression:** gate the fallback on the definition's render mesh being genuinely
unavailable, so Reach's 88 definitions that lack collision surfaces keep their render objects.

### Patch 2 — make "no render buffers" a first-class, reported state

`ass.rs:316` hard-requires `per mesh temporary`, and the cluster/definition loops silently
`continue`. Today a CE BSP exports "successfully" with 0.1% of its content and no warning — which
is how this went unnoticed. Thread a count of skipped meshes into the returned summary so the
caller can say "776 meshes had no render buffers". Small, and it makes patches 1 and 4 verifiable.

### Patch 3 — decide the output format for collision-only BSPs

ASS is a *render* source format; `@collision_only` is a convention, not a first-class carrier for
206k surfaces. Options, in the order I'd consider them:

1. Keep ASS with `@collision_only` — zero new format code, immediately usable in Blender/Max.
2. Emit collision JMS per definition, mirroring the existing Halo 1 path at
   `extract/geometry.rs:222` (`emit_ce_bsp_jms`) which already splits render and collision files.
3. A neutral mesh format (OBJ/glTF).

I'd start with (1) because it needs no new writer and validates patch 1 end-to-end, then
reconsider once the output has been looked at in a DCC tool. This decision should be made before
patch 1 lands, since it shapes patch 1's output shape — but it does not block *writing* patch 1.

### Patch 4 — render geometry via Chimp *(large; separate effort)*

The visual meshes are Unreal static meshes. `blam-tags/src/iostore/asset/static_mesh.rs`,
`nanite.rs`, and `level.rs` already exist and Chimp already extracts them. What is missing is the
**join**: a mapping from a BSP's `instanced geometry instances[i]` to the UE actor/component that
renders it. Whether that mapping is even recoverable is the first open question below, and it
should be answered before any of this is scheduled.

### Patch 5 — Baboon (optional)

`src/app/export/geometry.rs:14-22` already routes `sbsp` correctly. Once patches 1–2 land, the only
worthwhile additions are surfacing the skipped-mesh count in the export toast and, if patch 3 picks
a multi-file format, mirroring the scenario-level directory layout. Do this last.

### Suggested order

```
Patch 3 (decide format)  →  Patch 1  →  Patch 2  →  [re-evaluate]  →  Patch 4  →  Patch 5
                                 └── Patches 1–2 are the bounded, high-value core.
```

---

## 5. Open questions / blockers

1. **Is the BSP-instance → Unreal-mesh mapping recoverable at all?** This is the single question
   that decides whether CE level *render* extraction is feasible or permanently out of reach.
   Everything in patch 4 is downstream of it. Worth a focused spike before scheduling anything.
2. **Is `level_a` representative?** It is one of ~100 CE BSPs. Before generalising, run the
   `per mesh temporary == 0` check across all of them — if even one has render buffers, the model
   in §2 is wrong. This is cheap: the tooling already exists (see below).
3. **Format decision for patch 3.** Needs a human call; see the three options above.
4. **Does Halo 4 actually behave as hypothesis 2 claims?** Refuting hypothesis 1 for CE says
   nothing about H4 itself, and no H4 kit was available on this machine (the index holds only
   HREK, H3EK, H3ODSTEK). H4 BSP support is a genuinely separate piece of work from CE's, and this
   report should not be read as covering it.
5. **Should `Game` gain a fourth variant?** `Game::of()` returning `Halo3` for CE is currently
   harmless — CE genuinely is Reach-shaped. But if CE-specific branching accumulates, the
   three-value enum will become the wrong axis. `Title` already exists for engine-level
   distinctions and may be the better lever. Not urgent; worth deciding before, not after,
   the branches multiply.
6. **`instance kd hierarchy` semantics** — believed to be a runtime accelerator, not needed for
   extraction. Should be confirmed before anything relies on ignoring it.

---

## Implementation status

**Patch 1 — done.** `blam-tags/src/ass.rs`, uncommitted on `codex/chimp-backend`.
Format decision (patch 3) settled up front: **ASS, collision-only**, which is the
correct and expected output for a Blam/Unreal hybrid — the Blam side owns collision,
Unreal owns rendering.

What changed:

- New `append_collision_surfaces()` helper — the edge-ring walk + fan triangulation,
  lifted verbatim out of the structure-collision path so the same code serves both the
  sbsp's own collision BSP and each definition's `collision info`.
- `from_scenario_structure_bsp()` detects `per mesh temporary` being **entirely empty**
  and, in that mode only, sources definition meshes from `collision info` under
  `@collision_only`.
- The structure-collision block now calls the shared helper (pure refactor).

The tag-level gate was not the original plan — measurement forced it. A *per-definition*
fallback would have silently added collision objects to Reach exports for definitions
whose render mesh is missing: **51 of 428** on `cex_beaver_creek`, 43 of 434 on
`cex_damnation`, 11 of 809 on `cex_hangemhigh_000`. Gating on the whole block being
empty keeps every existing engine bit-for-bit unchanged.

Results on the fixture:

| | before | after |
|---|---|---|
| objects | 1 | **776** |
| instances | 2 | **3,494** |
| vertices | 198 | **643,914** |
| triangles | 96 | **230,508** |

Other CE BSPs: `holdouts` 754 objects / 11,382 instances / 1.17M tris; `sb_main` 151 /
617 / 59k; `BSP_library_middle_down` 554 / 2,948 / 286k.

Verification:

- **No regression.** Six BSPs across Reach and Halo 3 (`cex_beaver_creek`,
  `cex_damnation`, `cex_hangemhigh_000`, `armory`, `midship`, `farm`) exported with
  pre-patch and post-patch binaries are **byte-identical**.
- **Geometrically correct.** Transforming each definition's collision vertices by its
  placement and comparing to the bounds the tag stores per placement: 3,488 of 3,492
  exact (≤0.01 world units), worst escape 0.0166 units (~1.7 cm) on a ~60-unit world —
  float rounding. Triangle count is exactly consistent with the surface data:
  `643,914 − 2×206,703 = 230,508`.
- **Structurally valid ASS.** Declared section counts match emitted blocks (12/776/3,494),
  all instance object indices in range, exactly one Scene Root.
- **Test added.** `blam-tags/tests/sbsp_collision_only_ass.rs` — asserts the CE fallback
  runs *and* that Reach still exports render geometry. Verified to fail without the patch
  ("produced 1 object(s)") and pass with it. Skips gracefully; point `BLAM_TEST_CE_SBSP`
  at an extracted CE BSP to run the CE half. Full `blam-tags` suite passes.

Not committed — no commit was requested.

### Sealed-world collision across generations — also done

`ass.rs` read only `raw_items/collision bsp`, so any BSP that stores its sealed world in
`large collision bsp` silently lost it. Now both fields are read.

This turned out **not** to be Reach-only, as first assumed — CE is split across the two,
so the CE work was incomplete without it:

| BSP | `collision bsp` | `large collision bsp` |
|---|---|---|
| CE `c10/level_a`, `b30/sb_main` | 1 | 0 |
| CE `c20/BSP_library_middle_down`, `a30/holdouts` | **0** | **1** |
| Reach `cex_beaver_creek` | 0 | 1 (46,178 surfaces) |
| H3 / ODST | 1 | *field does not exist* |

Effect is strictly additive — one OBJECT + one INSTANCE, appended, with the
`@collision_only` material already present in every prior export so no material indices
shift. Verified before/after across five BSPs on three engines: ODST `c100`/`c200` and
H2AMP `ca_ascension_bsp01` byte-identical; Reach `cex_beaver_creek` and `cex_damnation`
each gained exactly their sealed world; Reach `cex_hangemhigh_000` unchanged (its
`large collision bsp` is empty — a fully instanced level). The added geometry is
arithmetically exactly the sealed world: `149,238 − 2×46,178 = 56,882` added triangles
on `cex_beaver_creek`.

(H3EK was unavailable for the second pass — the user was cleaning that tag tree — so
ODST, which shares H3's schema and has no `large collision bsp` field, stands in for the
H3 generation. H3EK was byte-identical in the first pass.)

### Patch 5 — Baboon UI — done

Both entry points are wired into the browser's Extract submenu
(`src/app/browser/tree.rs`):

- **`sbsp` → "Extract BSP geometry"** — one ASS for that BSP.
- **`scnr` → "Extract level geometry (one file per BSP)"** — walks
  `structure_bsps[]` and emits one ASS per referenced BSP under
  `<output>/<scenario>/structure/`.

Both dispatch the existing `BrowserAction::ExtractGeometry`; `extract_geometry_for_entry`
already had `sbsp` and `scnr` arms, so no new export plumbing was needed. The gap was
purely that `supports_tag_geometry_extraction` listed only `hlmt|mode|phmo|coll|mod2`, so
neither group ever reached the menu. Added `supports_bsp_geometry_extraction` and
`supports_scenario_geometry_extraction` rather than widening that predicate — the menu
wording differs and the two land in different arms.

#### Bug found and fixed: CE scenario BSP references did not resolve

The end-to-end test failed first time with *"no geometry emitted — all structure_bsps
entries failed to load"*. CE's cook moves a level's generated tags into a `_Generated_`
folder, but the references baked into the tag data keep the pre-cook path:

```
c10.scenario  structure bsps[5]/structure bsp  →  levels\halo1\solo\c10\level_a
mounted payload                                →  levels/halo1/solo/c10/_generated_/level_a
```

`ContainerTagIndex::lookup` (`src/source/mod.rs`) now retries through `_generated_` when
the literal key misses. Written as a fallback, not a mount-time alias, so an exact hit is
never shadowed by a `_Generated_` neighbour. This affects *every* CE reference consumer,
not just geometry export.

#### Verification

- `src/app/export/geometry.rs` — `#[ignore]`d end-to-end test against a real install
  (`CE_ROOT=... cargo test ce_structure_bsp -- --ignored`). Exports `c10/level_a` as a
  single ASS, then the `c10` scenario, asserting >1 file out and none empty. Now emits
  all **8** BSPs: `elevator_a/b/c`, `level_a/b`, `swamp_a/b`, `swamp_b_end` (24 MB–248 MB
  each).
- `src/source/mod.rs` — two CI-safe unit tests for the `_generated_` fallback, including
  that an exact match still wins.
- `src/app/browser/filter.rs` — the existing menu-coverage test asserted `scnr` had *no*
  extract menu; updated to assert both `sbsp` and `scnr` now do.
- Full suite: 601 passed, 0 failed.

#### Dependency — resolved

blam-tags is committed and pushed: `e0222185` ("Export a structure BSP's collision when
that is all it carries") on `codex/chimp-backend` at `Zoephie/blam-tags`. Baboon's
`Cargo.toml` pin is bumped to it and the temporary `[patch]` override is gone, so the
tree builds standalone — no sibling checkout required, CI included. Re-verified after the
bump: clean build from the git rev, the CE end-to-end test still emits all 8 c10 BSPs,
and the full suite passes (601).

## Reproducing the fixture

CE BSPs live inside the UE5 paks, so there is no loose file to open and nothing small
enough to commit — the smallest shipped CE BSP is ~9.9 MB. Getting one out needs no new
code: a throwaway binary depending on `blam-tags` with `features = ["iostore"]` and using
only the public read-only API is enough.

```rust
// IoStoreArchive::open(utoc) → ublock_entries() → read(&entry.path)
// filter with blam_tags::iostore::is_tag_payload  (real tag vs. UE bulk data)
// split names with parse_ublock_stem              ("<name>-<group_longname>")
```

Iterate every `.utoc` under `<install>/Meteorite/Content/Paks`, keep entries whose group
longname is `scenario_structure_bsp`, and write the bytes out — they are a byte-complete
Reach MCC tag file and open directly in `blam-tag-shell` or Baboon. `c10/level_a` lands at
33,018,844 bytes. The same sweep answers open question 2 (whether any CE BSP anywhere has
a non-empty `per mesh temporary`) by checking that block on each instead of writing files.

Baselines used throughout this analysis, for anyone re-running the comparisons: Reach
`HREK/tags/levels/dlc/cex_beaver_creek`, ODST `H3ODSTEK/tags/levels/atlas/c100`, H2A
`H2AMPEK/tags/levels/sway/ca_ascension/ca_ascension_bsp01`.
