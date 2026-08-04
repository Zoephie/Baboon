# Chimp level export — handoff

Exporting a Halo: Campaign Evolved level (geometry + world placement) out of
Chimp, for use in Blender. Paused mid-feature; this is what was learned, what
works, and what to do next.

## What a level actually is

`C10` is **World Partition**, not a package. There is a persistent level at
`/Game/Levels/Halo1/Solo/C10/C10` plus **2,334 `_Generated_` cell packages**,
each a `.umap` holding a handful of actors. "The level" is the union of what
those cells place.

Measured over a 467-cell sample, geometry arrives as:

| export class | count | carries a `StaticMesh` reference |
| --- | --- | --- |
| `InstancedStaticMeshComponent` | ~3,550 | 100% |
| `StaticMeshComponent` | ~1,320 | 67% |
| `StaticMeshActor` | ~730 | via its component |

The 33% of plain components without a `StaticMesh` inherit it from a Blueprint
class default, which lives in another package. Those are **not implemented** and
are counted as `LevelSkips::inherited_mesh`, ~0.4% of placements.

## What is done

### `blam-tags` (pushed, branch `codex/chimp-backend`)

- `82acfded3c59e60b0d33bf0ef408dbee3dfef2cc` — latest
- `iostore/asset/level.rs`: `read_instance_transforms(tail)` reads an instanced
  component's per-instance placements.

The instance array is **not reflected** — it sits in the export tail, past the
property block. Layout, established from shipped data:

```
[preamble][stride=128 (i32)][count (i32)][count x 128-byte FMatrix (16 x f64)][trailer]
```

Two things that cost real time and must not be re-broken:

1. **The preamble is not one fixed size.** It is 24 bytes for most components
   and 84 for the rest (the longer carries an `FBoxSphereBounds`). Assuming the
   first offset silently lost 6% of the level's foliage. The header is now
   *searched for* in a bounded window.
2. **The affine check is the proof, not decoration.** A candidate is accepted
   only if every matrix has a last column of `(0,0,0,1)` within 1e-6. That is
   what makes searching safe, and it is why a bad tail errors instead of placing
   geometry at an arbitrary point.

### Baboon (branch `chimp-level-export`)

- `src/app/chimp/level.rs` — reads a cell into a `LevelScene`: resolves
  `StaticMesh` via `read_import_slots`, composes `AttachParent` chains into world
  transforms, expands instances, counts skips by reason.
  - Composition is **row-vector, child-then-parent**.
  - Rotation is an `FRotator` in degrees stored as **(pitch, yaw, roll)** —
    rotations about Y, Z, X. Both have tests that fail on the transposed or
    reordered version.
- `src/app/chimp/level_export.rs` — writes a `.usda` (USD ASCII).

Validated on the real install (213 cells, every 11th):
`398 meshes, 22,069 placements, 82 skipped, extent ~600 x 720 x 335 m`.

## USD specifics that were learned the hard way

- **A reference does not change the referencing prim's type.** A
  `def Xform "inst" (references = </...Mesh>)` receives the geometry *attributes*
  but stays an Xform — it imports as a transform with no mesh. Prototypes are
  therefore `def Xform` **wrapping** a `def Mesh`, so the reference brings a
  child Mesh with it.
- **Prototypes must not be drawn.** They live under a `def Scope "Prototypes"`
  with `visibility = "invisible"`. `class` prims were tried first; some
  importers skip them entirely, which is the same empty result for a different
  reason.
- **Unreal winds triangles clockwise.** USD reads counter-clockwise as
  front-facing, so every surface imports back-facing and looks like broken
  normals. Fixed with `uniform token orientation = "leftHanded"` per mesh. The
  single-mesh exporters instead negate Y to mirror into right-handed space; a
  level cannot do that, because the world placements would have to be mirrored
  too.
- **Do not widen `f32` before printing.** `0.8423903` becomes
  `0.8423902988433838`, which cost 30% of the file size for nothing. Vertex data
  uses `number_f32`, matrices use `number`.
- Units are Unreal centimetres (`metersPerUnit = 0.01`), Z-up, and USD's
  `matrix4d` is row-major with the translation last — identical to Unreal's
  `FMatrix`, so placements are written through untransposed.

## Mesh detail: the open decision

`MeshDetail::{Fallback, Nanite}` selects the geometry source.

For a Nanite asset these are genuinely different meshes, not two LODs of one.
`UStaticMesh` keeps only a **coarse fallback** in its render data; the real
geometry is in the Nanite pages, and `decode_nanite` only emits the finest cut
(`FULL_LEAF`). There is nothing in between today.

Measured on the same 12-cell area (45 prototypes, 1,550 instances):

| detail | size |
| --- | --- |
| fallback | 7.5 MB |
| Nanite | 344 MB |

The user has judged the **fallback quality unacceptable**, so Nanite is required.
Extrapolated, all of C10 at Nanite is roughly **65 GB of USDA** — so whole-level
export in this format is not viable, only per-area.

Attempts to reduce it:

- **Precision trimming** gave only ~10% (344 → 310 MB), because ~40% of the file
  is `faceVertexIndices` and `faceVertexCounts`, which are integers. Reverted at
  the user's request.
- **Vertex welding** merged 551,633 duplicate vertices across 45 meshes — Nanite
  decodes cluster by cluster and each cluster repeats its boundary vertices, so
  the surface arrives as disconnected triangles. It **did not reduce size**
  (indices dominate and got wider), but it is *topologically correct* and worth
  restoring if the mesh needs to be editable. It was reverted only to get back to
  the baseline. Weld on **position + normal + UV**, never position alone: hard
  edges, UV seams and smoothing splits are legitimately co-located vertices with
  different attributes.

## Open issue: UVs in Blender

The user reports meshes arriving in Blender without UVs. **The data is in the
file and well-formed** — verified on a prototype: `points 11596,
primvars:normals 11596, primvars:st 11596`, values sensible and tiled, array
length matching the point count as `interpolation = "vertex"` requires.

So this is the import side, not the export. Two untested hypotheses:

1. Blender may not read primvars from **prototype** meshes when instancing is
   preserved. Test by re-importing the small fallback file with **Import Instance
   Proxies enabled** — if UVs appear, it is the instancing path.
2. The UV layer may be imported but inactive. Check Object Data Properties → UV
   Maps for a layer named `st`; with no textures bound, untextured is expected
   either way.

## Where to pick up

The user's preferred direction is to **bypass file formats and stream into
Blender**: Baboon writes a small binary sidecar we define (header, raw `f32`
arrays, `u32` indices, a placement table of matrices), and a **Blender addon**
builds the scene through `foreach_set`, creating one mesh datablock per prototype
and one object per placement referencing it — linked duplicates by construction.

Why it is attractive:

- The ~4x saving is from never serialising to text (a position is ~24 characters
  of ASCII versus 12 bytes), not from `.blend` compression, which is a bonus on
  top. Estimated ~85 MB binary for the 12-cell area against 344 MB of USDA, and
  perhaps 30–50 MB saved as `.blend`.
- It removes Blender's USD importer from the equation, which is where the UV
  problem lives.

Costs to weigh: it is a second deliverable to ship and version, and it serves
only Blender, where USD and GLB work in any DCC.

Alternatives, honestly ranked:

- **GLB** — roughly 2.3x smaller than USDA (~135 MB for the area), node-to-mesh
  reuse gives instancing natively, Blender-native import. Much less work than
  USDC.
- **A coarser Nanite cluster cut** — the real quality-vs-size dial, and better
  than any container change: far better than the fallback, potentially 4–10x
  smaller than full leaf, and it stacks with any format. Meaningful work in
  `blam-tags`.
- **USDC** — best on size, but implementing the Crate binary format (TOC,
  token/string/field/path tables, LZ4 sections, integer-compressed index arrays)
  is a multi-day job, and linking OpenUSD's C++ library does not fit this
  project.

## Running things

Everything is driven by tests; there is **no UI yet** (that was the next phase).

```sh
# All level work, with the real-data checks active
BABOON_PROBE_PAKS="D:/SteamLibrary/steamapps/common/Halo Campaign Evolved/Meteorite/Content/Paks" \
  cargo test chimp::level

# Write a sample for inspection
BABOON_PROBE_PAKS=...            # the Paks folder
BABOON_SAMPLE_OUT=out.usda       # destination
BABOON_SAMPLE_STEP=200           # read every Nth cell (200 ≈ 12 cells)
BABOON_SAMPLE_NANITE=1           # omit for the fallback mesh
  cargo test write_sample_usd -- --ignored --nocapture
```

Real-data tests **skip cleanly** when `BABOON_PROBE_PAKS` is unset, matching how
the editing-kit fixture tests behave, so the ordinary suite is unaffected.

## Not started

- Chimp levels view: group `_Generated_` cells under their persistent level so
  C10 is one browsable entry, loaded in bounded batches.
- 3D level preview, sharing the `LevelScene` with the exporter so what is on
  screen is what exports.
- Blueprint-inherited meshes (the `inherited_mesh` skips).
