# Campaign Evolved custom-tag authoring — implementation plan

Companion to [`campaign-evolved-tag-packages.md`](campaign-evolved-tag-packages.md),
which is the measured specification. This document is the plan: what to build,
in what order, with what acceptance gates, and what each step de-risks.

**The defect being fixed.** A CE tag is a UE package whose `.uasset` wrapper
carries the tag's identity, its hard-reference preload list, and (for 22 of 101
groups) a binding to an Unreal asset. Today Baboon never builds that wrapper —
it copies one. `add_override_to_writer` re-emits the base game's `.uasset`
verbatim, patching only `serial_size` and only on a size change;
`add_new_package_to_writer` clones a same-group template including its import
map and export body. So a new tag inherits the template's `AssetReference` and
dependencies, and a reference repointed at a new tag never gains the import that
would preload it.

---

## Phase 0 — The spike (do this first)

**Purpose.** Three unknowns remain, all about the *load model*, and one branch
of them is project-ending: if the tag registry is populated from a fixed list a
mod cannot write, custom tags are impossible rather than merely harder.
Discovering that after two weeks of building would be a bad trade. This spike
answers all three for about a day, **using only code that exists today**.

**The unknowns**

1. Is the runtime tag registry populated by enumerating packages, or from a
   fixed list?
2. Does resolution *load* on demand (`TryLoad`) or only find an already-loaded
   object (`ResolveObject`)?
3. Does the engine honour a mod-supplied `ContainerHeader` package-store entry
   for a brand-new package?

**The experiment**

1. Take `marine-biped` — the one override already confirmed loading in-game.
2. Edit its blob so a single tag reference points at a **new** path, e.g.
   `objects/characters/marine/my_test` in group `collision_model`.
3. Ship a new `my_test-collision_model` package cloned from an existing
   `collision_model`. Cloning is genuinely correct here: `collision_model` is one
   of the 47 bare groups, so the template wrapper carries nothing
   group-specific.
4. **Leave `marine-biped`'s `.uasset` import map untouched.** Nothing in the
   game will import the new package.
5. Build with `write_mod_container_ex` (one override + one new package) and run.

**Reading the result**

| Outcome | Conclusion |
|---|---|
| Marine still has collision | The new tag resolved **by name** with nothing importing it → registry is dynamic, load-on-demand works, mod `ContainerHeader` is honoured. All three unknowns answered positively; the plan below stands unchanged. |
| Collision missing / `Default/default-collision_model` behaviour | Name resolution alone is insufficient. Re-run with the import added to `marine-biped`'s wrapper (requires Phase 1+2) to separate "needs the hard reference" from "new packages don't register at all". |
| Game fails to mount the container | The `ContainerHeader` path is wrong — isolate before anything else. |

**Also worth capturing in the same session**, since the build is already made:
try the debug-menu config override (see §"Debug surface" below). Low expected
value, five minutes.

---

## Phase 1 — blam-tags: the encoder and the builder

Everything downstream depends on these two, and both are policed by corpus
gates. Build the gates first: this investigation was wrong twice under corpus
pressure (the `imported_public_export_hashes` dedup rule, worth 8 bytes; and the
`FUnversionedHeader` fragment bit layout), and both were caught only by checking
against all 12,291 packages.

### 1.1 `iostore::unversioned::write_export_struct`

The mirror of the existing reader. The complete surface it must cover, measured
across every tag export:

| Kind | Encoding |
|---|---|
| `Object` | `i32` `FPackageIndex` |
| `Name` | `i32` name-map index + `i32` number |
| `Array` | `i32` count, then elements |
| `Map` | `i32` NumToRemove (always 0), `i32` Num, then key/value pairs |
| `Struct` | nested unversioned block (`FBlamVariant`, `FBlamCustomizationGlobalsTagDataIndices` only) |
| `Bool` | one byte |
| `Int`/`UInt32` | 4 bytes |
| `SoftObjectPath` | three `FName`s (package, asset, sub-path) |

Header rules:

- fragment layout: `skip` = bits 0–6, `has_zeroes` = bit 7, `is_last` = bit 8,
  `value_num` = bits 9+
- **no zero mask** — not one tag export uses a has-zeroes fragment. Assert
  rather than implement, so a future case surfaces loudly.
- body frame: `[00 00 00 00] [fragments] [values] [00 00 00 00]` — both
  4-byte pads are inside `cooked_serial_size`
- max 5 fragments per export, 3.12 average
- `DefaultAssetReference`, `BinaryBlobSize`, `NativeClass`,
  `Permutations_EMPTY`, `ObjectTagDataAsset`, and model's `Variants`/
  `RegionTable` are never emitted — the encoder reaches them only as skip counts

Supporting work: `FNameMap::store` must fold a trailing `_<n>` back into
`(base, number+1)` when the base is already interned (ordinary UE `FName`
semantics; three model tags depend on it).

**Gate 1.1** — re-encode all 12,291 shipped export bodies byte-for-byte.

### 1.2 `iostore::tag_package::TagPackageBuilder`

Synthesize a complete `.uasset` from `(package path, group, referenced packages,
AssetReference, group-specific extras)`. Rules, all at 100% over the corpus:

- `FPackageId` = `cityhash64` of the lowercased UTF-16LE package name
- `public_export_hash` = same hash of the *object* (leaf) name
- class / template = ScriptImport hashes of
  `/Script/BlamSynchronization.Blam<Pascal(group)>TagDataAsset` and its
  `Default__` form
- exactly 3 script imports: class, CDO, and the module package
  `/Script/BlamSynchronization`
- one Null import slot per imported package
- `imported_packages` sorted ascending by `FPackageId`, names parallel
- `imported_public_export_hashes`: one entry per unique
  `(imported_package_index, hash)` pair, in import-map first-use order
- dependency bundle = exactly the property-referenced imports, ordered by
  **reversed** usmap property order (base-class properties first)
- name map = case-insensitively sorted unique property `FName` bases, then
  object name, then package name
- one export; one bulk entry with `flags = 66817`, `serial_size = len(.ubulk)`
- flags: `0xb`/`0x80002200` for every group **except** `scenario`,
  `scenario_structure_bsp`, `scenario_structure_lighting_info`,
  `structure_design`, `structure_seams`, which use `0x1`/`0x88002200`
  (`PKG_CookGenerated`)
- package trailer `c1 83 2a 9e` after the export body

**Gate 1.2a (byte-identity)** — parse each shipped package, rebuild it through
the builder *reusing the parsed import order*, re-serialize: byte-identical for
all 12,291. This proves the serializer and every derivation.

**Gate 1.2b (semantic)** — build from the builder's own canonical import order,
parse back, and assert identical resolved property targets, dependency set,
identity hashes, flags and name map. Necessary because the cooker's import-map
slot order is linker-derived and not reproducible; it does not need to be, since
the array is only indexed from inside the package.

---

## Phase 2 — blam-tags: derivations and writer rewiring

### 2.1 Group tables

Derived from the corpus, **not** the usmap —
`BlamFrameEventListTagDataAsset` ships 130 tags but is absent from both the usmap
and the UHT dump, and must fall back to `BlamTagDataAssetBase`.

- group → class name (mechanical `Blam<Pascal>TagDataAsset`)
- group → wrapper base, which decides whether `AssetReference` applies and which
  hash rule it uses
- group → flag pair

### 2.2 `CookedAssetsReferencedByTag`

Walk the `TagFile`'s `tag_reference` fields (reuse the existing walker) and emit
every ref that resolves to a package we ship or that exists in the base game.
This is a superset of shipped behaviour and demonstrably safe — the cooker itself
emits supersets on 34 tags and omits the array entirely on 124 others.

Dangling references must **warn, not error**: 1,381 shipped reference instances
already point at packages that do not exist.

### 2.3 `AssetReference`

Computable from the target package path alone — 6404/6404, no donor tag and no
Blueprint parsing:

- `TSubclassOf` groups (the 14 object groups plus `effect`):
  `cityhash64("<PackageLeaf>_C")`
- plain `UObject*` groups (`sound`, `sound_looping`, `sound_combiner`,
  `cinematic`, `damage_response_definition`): `cityhash64("<PackageLeaf>")`

### 2.4 `RuntimeVariants` (model)

`{VariantName: variants[i].name, Permutations: {region.name →
region.permutations[0] or "None"}}` — 407/408 exact. Only `RuntimeVariants`
ships; `Variants` and `RegionTable` are never serialized. Without this, a variant
edit in Baboon is invisible in-game.

### 2.5 `BlamModelRegionStringTable` synthesis

A second, much simpler package builder reusing 1.1/1.2 — the asset is 370 bytes,
no bulk data, 3 script imports. Content: `Regions` from the model tag's variant
regions (matches 278/364), `Permutations` from the **mesh-side** vocabulary that
blam-tags already reads from the mesh-sync data (the tag's designated subset is
*not* sufficient — `DA_Marine_Regions` carries 88 permutations shared across
several models).

### 2.6 Rewire the writer

- `add_override_to_writer`: **rebuild** the wrapper from the edited blob rather
  than copy-and-patch `serial_size`. Always reissue the `.uasset`, not only on a
  size change — a reference edit changes the wrapper even when the blob length
  does not.
- `add_new_package_to_writer`: **build** rather than clone a template.
- Preserve the existing `_P` suffix / `.pak` stub / priority behaviour, which is
  already proven in-game.

### 2.7 Robustness

`FOL_MC_GearMove_Land-sound` ships with no `.ubulk` chunk at all despite
declaring `serial_size = 4700`. Degrade gracefully on a missing bulk chunk rather
than erroring.

---

## Phase 3 — In-game validation matrix

Phase 0 covers the first new-package case. This is the full matrix; each row is
one mod build.

| # | Case | Currently |
|---|---|---|
| 1 | Same-size blob edit | **proven** (`marine-biped`) |
| 2 | Size-changing blob edit (reissues the `.uasset`) | unproven |
| 3 | New bare-group tag, referenced by an edited tag | Phase 0 |
| 4 | New wrapper-bearing tag — object group with `AssetReference` = `BP_EmptyActor` | unproven |
| 5 | New `model` tag with `RuntimeVariants` + a generated region table | unproven |
| 6 | Override of a tag in a **non-pakchunk0** (per-level) pak | unproven |
| 7 | A `_Generated_` group tag (`0x1`/`0x88002200` flags) | unproven |
| 8 | Rename via package redirect | unproven |

Row 6 matters because the only proven override lives in `pakchunk0`; chunk ids
are global so it should be identical, but level tags are exactly where scenario
editing will land.

---

## Phase 4 — Baboon

### 4.1 Persistence for the Unreal-only values

The single highest-risk piece, and textbook
[unconsulted-state](../../.claude/projects) territory — a value written and never
read. The layering:

1. **Seed** from the tag's existing `.uasset` when it has one, so untouched tags
   round-trip identically.
2. **Override** in the workspace project file, keyed by tag path, for new tags
   and user edits.
3. **Fall back** to the group default.

Every one of those three paths needs a test that asserts on the value the export
actually emits, not on the value that was stored.

### 4.2 Unreal binding panel

Shown for CE container tags:

- derived `CookedAssetsReferencedByTag` — read-only, with a count and which
  entries are unresolved
- `AssetReference` path field plus picker, for the 22 groups that have one. The
  picker offers targets already used by tags of the same group (22 for `biped`,
  29 for `projectile`, 257 for `crate`, …) plus free search over the package
  index, pre-filled with `BP_EmptyActor` for object groups.
- `effect`: `bSpawnPerInstance` checkbox
- `model`: `ModelRegionStringTable` picker defaulting to
  `/Game/Tags/objects/shared/RT_default_object_regions` (214 of 364 shipped model
  tags use it), a "Generate region table…" action, and a read-only preview of the
  derived `RuntimeVariants`
- `player_model_customization_globals`: a small map editor. One tag in the game;
  lowest priority.

### 4.3 New Tag

Drop the "needs a same-group template" restriction — all 139 defined groups
become authorable, with the binding fields inline for the 22.

### 4.4 Pre-export lint

- new tags that nothing references (the silent-death case)
- dangling references — **warn**
- object-group tags with no `AssetReference`
- model tags whose variants name regions absent from the chosen string table
- new tags in the `_Generated_` groups (flag them; creating scenarios is
  deferred)

### 4.5 Export preview

Extend the existing diff view to show wrapper changes, not just blob changes —
otherwise the most consequential part of an export is invisible in review.

---

## Phase 5 — Loose ends

None block; each touches at most 11 tags. Worth a pass once the trunk is
working.

1. The second extra script import on `player_model_customization_globals`
   (`3870AE7534B9FC0D`) is unidentified. The first is
   `/Script/GameplayTags.GameplayTag`.
2. Why struct-typed properties sometimes need a `UScriptStruct` script import and
   sometimes not — `FBlamVariant` gets none across 408 model tags,
   `FGameplayTag` gets one. Matters only if a new group uses a struct property.
3. The 122-vs-220 `squad_template` split (array absent vs present with identical
   shape) — presumed cook non-determinism, unproven.
4. Eleven `model_animation_graph` tags whose cooked array names a
   *differently titled* `frame_event_list` (`plasma_rifle_red` →
   `plasma_rifle`).
5. The single `RuntimeVariants` outlier (`rocket_launcher_ammo` dropping `hits`).
6. Why `FOL_MC_GearMove_Land-sound` ships with no `.ubulk`.

---

## Debug surface (opportunistic)

The retail simulation DLL retains the Blam debug command set —
`tag_is_active`, `dump_loaded_tags`, `tag_load_force`, `tag_unload_force`,
`tag_reload_force`, and the `scenario_load_all_tags` global. Those would settle
the load model directly, but everything reaches them through the single
`CreateBlamEngineShell` vtable and the dispatch path is not established.

The retail exe also ships ImGui and a Blam debug menu gated by
`UDebugMenuSettings` (`Config=Game`, `DefaultConfig`), whose three `*Shipping`
flags default false. A user-layer `Game.ini` override is worth trying:

```ini
[/Script/Meteorite.DebugMenuSettings]
bEnableDebugMenuDefaultShipping=True
bEnableDebugMenuReleaseShipping=True
bEnableDebugMenuBetaShipping=True
```

placed beside the `HaloGlobalGameUserSettings.ini` the game already writes.
Caveat: `UBlamDebugMenuWidget` looks like an object/tag-name inspector
(`SetShowTagDebugNames`), not a command console, and `HaloImGuiUtils` is only a
Blueprint wrapper around ImGui drawing primitives. Low expected value; not on
the critical path.

---

## The Unreal side of an object — what is actually authorable

The tag→Unreal binding is **bidirectional**, and only one direction was mapped
in the sections above:

- **tag → Unreal**: the object tag's `AssetReference` → a BP actor class
  *(mapped, 6404/6404)*
- **Unreal → tag**: the BP's `BlamMeshSynchronizationComponent` →
  `BlamMeshSynchronizationDataAsset` → `ModelTag`, a hard reference **back** at a
  model tag. 120/120 data assets carry one, hitting 116 distinct model tags.

`BP_EmptyActor` — the recommended default for a new object tag — has **5
exports, zero imported packages and nothing but a `DefaultSceneRoot`**. So a new
object tag pointed at it is fully understood by the simulation and renders as
nothing. Pointing at a real BP instead inherits that BP's geometry *and* its
bound model tag.

### Where the geometry actually lives

`RuntimeRegions` is on the **component template inside the Blueprint**, not on
the data asset. For `BP_BruteBipedActor` (51 exports, 71,776 bytes) it is
export[0], `BlamMeshSynchronization_GEN_VARIABLE`, 2,704 bytes:

```
AnimationClass               = HARD import
MeshSynchronizationDataAsset = HARD import
RuntimeRegions = Map[region] -> { Permutations: Map[perm] -> {
    SkeletalMeshes: [ { Asset: SOFT('/Game/Characters/Brute/Default/Mesh/SK_Brute'),
                        Class: SOFT('/Game/Blueprints/CVW/BPC_SkeletalMesh'),
                        MaterialOverrides: [], ComponentTags: [] } ],
    StaticMeshes:   [ { Asset: SOFT('…/SM_Brute_Head_M_Default'), ParentBoneName, Transform, … } ] } }
```

**The mesh and component-class references are `FSoftObjectPath` — plain
strings.** Only `AnimationClass` and `MeshSynchronizationDataAsset` are
import-map entries.

That means swapping an object's geometry is *rewriting strings in one export's
property blob* — no import-map surgery, no Kismet, no new package. And the tool
for it is the **same `write_export_struct` from Phase 1.1**; the tag work and
the Unreal work share one encoder.

### The capability ladder

| Level | What it takes | Feasibility |
|---|---|---|
| Retexture / re-material | `MaterialOverrides` in `RuntimeRegions` (soft), or MIC parameter overrides | high — encoder only |
| **Mesh swap on an existing object** | rewrite soft paths in the BP's component template | high — encoder only |
| New object reusing existing behaviour | clone a BP package, repoint `MeshSynchronizationDataAsset` (hard import), rewrite `RuntimeRegions`, new mesh-sync DA, new model tag | medium — needs import-map editing on a cloned BP |
| Genuinely new behaviour | Kismet bytecode (`BndEvt__…` UFunctions, delegate bindings) | out of reach |

### Swept over all 335 mesh-sync packages

- **336 components, 333 decoded** (3 failures, uninvestigated)
- **Hard import refs: exactly 270, and only ever `AnimationClass` (139) and
  `MeshSynchronizationDataAsset` (131). Nothing else.**
- **Soft path refs: 10,078** — every mesh and component class, without exception
- 146 components serialize `RuntimeRegions`; 187 leave it default (the FP and
  cinematic variants, populated at runtime)
- region counts run 1–64, median 1
- exports per package: min 7, median 13, max 250

### 188 of 335 have no Kismet at all

Those are pure-data Blueprints. Their entire export composition is:

```
BlueprintGeneratedClass, SimpleConstructionScript, SCS_Node ×N,
InheritableComponentHandler, the CDO, and component templates
(SceneComponent, StaticMeshComponent, SkeletalMeshComponent,
BlamMeshSynchronizationComponent, Niagara*, Blam*Component…)
```

The smallest is **7 exports / 2,637 bytes**
(`BP_DeviceMover_DeviceMachineActor`); the Sentinel garbage actors are 8 exports
/ ~3.5 KB.

**A minimally viable actor Blueprint is therefore ~7 exports of pure property
data and no bytecode — synthesizable with the same encoder and package builder
as everything else.** That raises the ceiling from "new tags reusing an existing
actor" to **new objects with new geometry, authored end to end**:

1. new model tag (+ collision / physics / skeleton / animation tags)
2. new `BlamMeshSynchronizationDataAsset` — one property, `ModelTag`
3. new Kismet-free BP — 7–13 data exports, `RuntimeRegions` pointing at meshes
   by soft path
4. the object tag's `AssetReference` → the new BP class

Only actors needing *new behaviour* (the 147 packages that do carry
`BndEvt__…` UFunctions) remain out of reach.

---

## Blocking bug found in `Usmap::flattened_properties`

`flattened_properties` walks derived→base and concatenates each struct's own
properties sorted by `schema_index`. It **ignores `array_dim`**. A static-array
property occupies `array_dim` *consecutive schema slots*, so any class
containing one has every subsequent property mis-indexed, and the
unversioned reader desyncs.

`MaterialInstance::PhysicalMaterialMap` has `array_dim = 8`, which is exactly
why `Parent` sits at `schema_index 9` and why every
`MaterialInstanceConstant` fails to decode with
`MaterialParameterInfo: present schema index N beyond 3 props`.

**Verified rule** — expanding each property `array_dim` times, rebasing per
struct in the chain, makes position == `schema_index` for **10,647 of 10,647
classes in the usmap, zero exceptions.**

Why the tag work never hit it: every property on every `Blam*TagDataAsset` has
`array_dim = 1`, so the flattening is accidentally correct there — which is why
all 12,291 tags decode. The bug is invisible until a class uses a static array.

**Impact:** `MaterialInstanceConstant` (5,764 packages) and `Material` (1,397)
currently cannot be decoded at all. Fix this before any material work.

**Fixed.** Both classes now decode completely — see the coverage report below.

## Asset anatomy — measured so far

### `Texture2D` — 5,524 packages, 5,524 decoded, 0 failed

15 properties total, all simple: `ImportedSize` and `LightingGuid` (native),
`CompressionSettings`, `LODGroup`, `Filter`, `LODBias`, `AddressX`/`AddressY`,
`Availability`, `MipLoadOptions` (ints), `SRGB`, `VirtualTextureStreaming`,
`NeverStream` (bools), `AssetUserData`, `Downscale`.

**Only 13 hard refs in the entire set (all `AssetUserData`), and zero soft
refs.** Bulk entries run 1–3 for a normal texture (mip streaming levels).

The property block is tiny — median 874 bytes — but export sizes reach 16 MB,
because `UTexture2D::Serialize` writes `FTexturePlatformData` (pixel format, mip
descriptors, inline mips) *after* the property block as native serialization the
usmap does not describe. **That structure is the remaining work for texture
authoring**; the reflected properties are trivial.

## Coverage report — measured 2026-07-28

Produced by `ce_coverage_matrix 1`, which decodes **every export of every
package** in the shipped paks and tallies per class. Run it at threshold `1`;
a higher min-exports argument hides the small failing classes and makes the
remainder look far shorter than it is.

Corpus: **103,867 packages**, 54,825 script objects resolved, 322,300
identifiers read out of the shipped executable.

Note the unit change from earlier tables in this document: these are **export
occurrences**, not packages containing one.

| | share | count |
|---|---|---|
| runtime native-class exports | — | **1,153,834** |
| property block decodes | **99.9944 %** | 1,153,769 (65 fail) |
| every byte accounted for | **99.96 %** | 1,153,425 |
| never attempted | — | 0 |

That byte figure was 99.43 % at the start of this session; the layout fixes
listed below account for the 6,189-export difference. Measured in *bytes* rather
than exports the change is far larger — the unread total fell from roughly
1.02 GB to **156 MB** — of which 151 MB is a single class.

Plus **89,762** Blueprint-class exports, which have no native schema and decode
via their class package, and **153** editor-only exports the shipped runtime
cannot construct (`BlamFrameEventListTagDataAsset` 130, `PerfToolTextBox` 17,
two `HLODBuilder*` settings, two more `PerformanceOverlayTool` shims). These are
correctly excluded from the runtime denominator — they are cooker leftovers,
written by an editor that had those plugins loaded, and their names appear in no
shipped binary in either encoding.

### Classes this document previously called blocked

| Class | Exports | Decoded | Bytes complete |
|---|---|---|---|
| `Material` | 1,397 | 1,397 | **all** |
| `MaterialInstanceConstant` | 7,852 | 7,852 | **all** |
| `Texture2D` | 14,237 | 14,237 | **all** |
| `AnimSequence` | 14,130 | 14,130 | **all** |
| `StaticMesh` | 15,231 | 15,231 | **all** |
| `BodySetup` | 17,754 | 17,754 | **all** |
| `World` / `Level` | 14,240 each | all | **all** |
| `PhysicsAsset` | 96 | 96 | **all** |
| `SkeletalMesh` | 415 | 415 | **all** |
| `Skeleton` | 128 | 128 | **all** |

The `array_dim` bug and the native-struct sizing that blocked `StaticMesh`
(`MeshUVChannelInfo: present schema index 3 beyond 3 props`) and `SkeletalMesh`
(`implausible array count 1017370378`) are both resolved. `StaticMesh` is
complete through `FStaticMeshRenderData` including Nanite, and `SkeletalMesh`
is now complete through `FSkeletalMeshRenderData` — every byte of all 415.

**`MaterialInstanceConstant` authoring surface** (unchanged, still the point):
`Parent`, `ScalarParameterValues`, `TextureParameterValues`,
`VectorParameterValues`, `BasePropertyOverrides`, `StaticParametersRuntime`.
The refs are **all hard imports and zero soft**, so **retexturing a material
requires import-map surgery** — unlike a mesh swap on a mesh-sync component,
which is soft paths only. Two different authoring mechanisms.

### Closed this session — the `UStruct` chain and six native tails

All verified against UE `5.5.4-release` source, then corpus-gated: every class
below went to **zero** remaining tails with **zero** new decode failures.

| Class | Tails closed | What it actually was |
|---|---|---|
| `Function` | 897 | see below |
| `NiagaraSpriteRendererProperties` | 918 | `FSubUVDerivedData` = one `TArray<FVector2f>` of cutout geometry |
| `SkeletalMesh` | 415 | the whole `FSkeletalMeshRenderData` — see below |
| `TextureCube` / `VolumeTexture` / `Texture2DArray` | 58 | share `SerializeCookedPlatformData`; **only `UTexture2D` writes the `bSerializeMipData` flag**, so the existing `SkipOffset` skip transferred once parameterised — 179 MB |
| `StaticMesh` | 1 | not a layout bug: `FRawStaticIndexBuffer` stores indices as **single-byte** elements, so a 1024×1024 plane's count of 25,165,824 tripped a flat 10 M plausibility ceiling. Bounding array counts by the bytes actually remaining is both correct here and tighter everywhere else — 75 MB |
| `PhysicsAsset` | 49 | `CollisionDisableTable`, a `TMap<FRigidBodyIndexPair, bool>` at 12 bytes per entry |
| `StringTable` | 29 | `FStringTable::Serialize` — namespace, key/source `FString` pairs, then a per-key meta-data map |
| `SkyAtmosphereComponent` | 17 | `bStaticLightingBuiltGUID`, 16 bytes |
| `AkStateValue` / `AkSwitchValue` / `AkRtpc` / `AkAuxBus` / `AkInitBank` | 178 | each appends its Wwise cooked-data struct as a property block — **no plugin source needed, the usmap describes them all** |
| `RecastNavMesh` | 13 | a version and a byte count the loader seeks past |
| `ModelComponent` | 17 | `Model`, the elements, then `ComponentIndex` + node list |
| `PCGMetadata` | 63 of 123 | typed attribute table; see below |
| `DNAAsset` | 21 | two embedded RigLogic DNA streams — **91 MB**, see below |
| `NiagaraScript` | 17,674 | the cooked GPU shader maps — see below |
| `InstancedFoliageActor` | 238 | `FoliageInfos` map: key ref, `uint8 EFoliageImplType`, and for `StaticMesh` one component ref |
| `Skeleton` | 128 | `FReferenceSkeleton` + retarget sources + `Guid` + empty smart-name map + `FStripDataFlags` |
| `WorldPartition` | 72 | cooked flag, then the streaming-policy ref |
| `UserDefinedEnum` | 64 | `UEnum`'s `Names` (`FName` + `int64` each) then a `uint8 CppForm` |
| `FontFace` | 37 | cooked flag + inline-data flag (CE ships every face out of line) |
| `BlueprintGeneratedClass` etc. | 67 | rode along on the `UStruct` fix |

**`Function` was three separate `FProperty` layout bugs, not a padding
problem.** The `Struct` arm used to *probe* several word offsets for the field
count, which silently accepted a wrong parse whenever the real one failed — so
the reader reported a bogus decoded prefix instead of a tail, and hid all three:

- `FEnumProperty::Serialize` writes `Enum` **before** the underlying property.
  We read them the other way round, so the nested field's type-name `FName`
  landed on a negative package index.
- `FClassProperty` adds a `MetaClass` after `FObjectPropertyBase`'s
  `PropertyClass` — two references, not one. Same for `FClassPtrProperty`.
- `FSoftClassProperty` likewise adds `MetaClass` on top of `FSoftObjectProperty`.

With those fixed, `UStruct::Serialize` reads straight through with no padding
anywhere: `SuperStruct`, `TArray<UField*> ChildArray`, `SerializeProperties`
(an `int32` count then that many `FField`s), then the Kismet script.

The probing is now gone, so a layout error surfaces as a tail instead of a
plausible-looking wrong answer. **Do not reintroduce offset probing here** — it
cost roughly a thousand exports of silent mis-parse.

### `SkeletalMesh` — the deepest walk in the reader

`USkeletalMesh::Serialize` is strip flags, a 56-byte `FBoxSphereBounds`, the
`FSkeletalMaterial` list, the `FReferenceSkeleton`, a cooked flag, and then the
entire `FSkeletalMeshRenderData`: every LOD's sections, then either the inlined
buffer set (`SerializeStreamedData`) or a `.ubulk` header plus
`SerializeAvailabilityInfo`, then `FNaniteResources`, then two `uint8` counts.
Four details cost real time and are worth keeping:

- **`FStripDataFlags` bit 0 is `EditorOnly`, bit 1 is `AudioVisual`.** Every
  client cook sets bit 0, so testing it as "audio-visual stripped" silently
  skips all render data. The observed value is `5`.
- **Two array conventions sit side by side.** Vertex payloads reached through
  `TStaticMeshVertexData::Serialize` use `BulkSerialize` — an element size *and*
  a count. Members reached through a plain `Ar <<` (`SourceRayTracingGeometry
  .RawData`, `MorphData`, both half-edge buffers, the skin-weight profile
  arrays) are a bare count. Reading one as the other drifts by four bytes per
  array and shows up hundreds of bytes later.
- **`NumInlinedLODs`/`NumNonOptionalLODs` are `uint8`,** not `int32`.
- **`bHasVertexColors` and `bEnablePerPolyCollision` are reflected properties
  that gate native layout** — the tail reader takes them from the decoded
  property block rather than probing.

### `NiagaraScript` — the "deep one" was walkable after all

This was recorded across two earlier sessions as the one part of the corpus with
no tractable path: `FNiagaraShaderScript::SerializeShaderMap` →
`FShaderMapBase::Serialize` → `FMemoryImageResult::LoadFromArchive`, ending in a
**frozen memory image** — a raw dump of C++ objects whose layout depends on
target-platform pointer size and alignment. The earlier note also established
that Niagara does *not* use `FMaterialResourceProxyReader`, so the `NumBytes`
skip that works for materials does not transfer.

All true, and all beside the point. **Nothing in it needs interpreting.** The
frozen image is length-prefixed (`FrozenSize`), the shader bytecode is two
length-prefixed `FSharedBuffer`s, and every table between them is a plain count.
The full walk:

```
int32 ResourceCount, then per resource:
  bCooked, NumPermutations, BaseCompileHash (count + bytes)
  bValid                                    -- a cooked resource may have none
  FPlatformTypeLayoutParameters             -- 8 bytes
  FrozenSize + that many opaque bytes
  pointer table: type dependencies (32 B each: FName + size + FSHAHash),
                 shader + vertex-factory type names (FHashedName, 8 B each),
                 then Niagara's own data-interface class names (FStrings)
  NumVTables / NumScriptNames / NumMemoryImageNames, counted up front,
                 then their three patch tables
  bShareCode, ShaderPlatformName
  bShareCode ? FSHAHash : FShaderMapResourceCode
                 (ResourceHash, shader hashes, then per shader two
                  uint64-prefixed buffers: header and code)
```

One thing is genuinely Niagara-specific and easy to miss: it subclasses the
pointer table as `FNiagaraShaderMapPointerTable`, which appends its
**data-interface class names as `FString`s** after the base class's hashed type
names. Without that the walk desyncs a few hundred bytes later, deep inside the
patch tables, where the failure looks nothing like its cause.

All 17,674 exports now account for every byte. The lesson is worth keeping: *"the
payload is opaque" and "the structure is opaque" are different claims*, and only
the second one blocks a byte-accounting reader.

### `DNAAsset` — a foreign, big-endian container with no length

`UDNAAsset::Serialize` embeds **two** RigLogic DNA streams back to back
(behavior, then geometry). The DNA container is not UE serialization at all:
a three-byte `DNA` signature, a generation/version pair, a section table, then
the sections — and **every scalar in it is big-endian**. Nothing records a
stream's total length.

Two header shapes ship in CE, and the mix is what makes this work:

- **generation 2 version 5** — a `uint32` section count then 16-byte entries
  (`desc`, `defn`, `bhvr`, `geom`, `mlbh`, `rbfb`, `rbfe`, `jbmd`, `twsw`), each
  with an offset *and a size*. The stream's end is the furthest section end.
  Verified: the index ends at exactly the first section's offset.
- **generation 2 version 1** — eight bare `uint32` offsets, **no sizes**. Its own
  bytes cannot give its length.

The version-1 boundary is still derivable rather than guessed. `UDNAAsset` is the
last class in its chain, so the *second* stream must end exactly at the end of
the export — and in every version-1 asset that second stream is a version-5
geometry stub. So the split point is the unique later `DNA` signature whose own
sized index lands precisely on the export end. Searching for a magic byte pattern
would be a heuristic; requiring it to close the export exactly is a constraint.

All 21 assets resolve. These are the MetaHuman head rigs, so this reader is worth
more than its coverage share — it is the entry point to the DNA data behind the
existing CE head extraction, not just a way to skip 91 MB.

**`SK_Manny` — a missing trailing field, 400 KB downstream.** The last mesh to
resolve failed *before* any of the above ran: its 768 KB `SamplingInfo` property
block ended in the wrong place, so the `UObject` `hasGuid` trailer read `10` and
the class chain stopped before `USkeletalMesh::Serialize`. The cause was one
omitted array. `FSkeletalMeshSamplingRegionBuiltData` sets `WithSerializer` and
its hand-written serializer writes:

```
Ar << TriangleIndices;  Ar << BoneIndices;  AreaWeightedSampler.Serialize(Ar);
if (CustomVer(FNiagaraObjectVersion) >= SkeletalMeshVertexSampling) Ar << Vertices;
```

`Vertices` comes **last**, after the sampler — not in declaration order, where it
sits between the triangle and bone arrays — and it is version-gated, so it reads
as optional. The reader stopped at the sampler, leaving every region element
short by `4 + 4 * NumVertices`. All 415 skeletal meshes now account for every
byte.

`BLAM_TAIL_WHY=1` narrates this walk stage by stage and hexdumps around any
bail — the tail is far too deep to diagnose by reasoning about the schema.

### What is left — 409 exports (0.035 %), 156 MB

**344 unmodeled native tails:**

| Class | Tails | Bytes | Note |
|---|---|---|---|
| `UserDefinedStruct` | 117 | 4.7 KB | its default instance via `SerializeItem` — needs the struct's own field chain as a schema |
| `DataTable` | 77 | 82 KB | each row is a property block in the table's row struct |
| `PCGMetadata` | 60 | 2.1 MB | 63 of 123 now decode; the rest leave a trailing run of `-1` entry keys not yet accounted for |
| `SoundWave` | 27 | 4 KB | |
| `GeometryCollection` | 14 | **151 MB** | the Chaos port — see above |
| `MorphTarget` / `CompositeDataTable` | 11 each | small | |
| ~20 more classes | 1–6 each | | `VectorFieldStatic`, `Model`, `OptimusComputeGraph`, `DynamicMesh`, `RigVMMemoryStorageGeneratorClass`, sound nodes … |

**`GeometryCollection` is 97 % of the remaining bytes and everything else
together is under 5 MB.** By exports the work is now dominated by three
schema-driven cases — `UserDefinedStruct`, `DataTable` and `CompositeDataTable`
(205 exports) all need the *same* missing capability: walking a property block
against a struct schema that comes from the package rather than the usmap. That
is one piece of machinery, not three.

**65 decode failures,** the complete list:

| Class | Failed / seen |
|---|---|
| `PCGGraphInstance` | 28 / 43 |
| `LandscapeGrassType` | 10 / 15 |
| `PCGGraph` | 7 / 19 |
| `Font` | 6 / 6 |
| `OptimusKernelSource` | 3 / 3 |
| `StaticMeshComponent` | 3 / 126,158 |
| `BlamMeshSynchronizationComponent` | 3 / 358 |
| `NiagaraComponent` | 2 / 16,403 |
| `RigVM` | 1 / 1 |
| `MovieSceneSkeletalAnimationSection` | 1 / 5 |
| `HaloUINumericTextBlock` | 1 / 43 |

`PCGGraphInstance`/`PCGGraph` are one cause (`InstancedPropertyBag`), `Font` is
`FontData`, and the three `StaticMeshComponent` failures are a `FColorVertexBuffer`
in a component LOD. The `BlamMeshSynchronizationComponent` and `NiagaraComponent`
outliers are long-standing and verified *not* truncated.

A further 19 failures exist in `PerformanceOverlayTool` classes and are **not**
counted in the 65 — those classes are editor-only, no schema for them exists in
the usmap or in any public source, and the shipped game cannot construct them.
They are permanently unreachable and that is correct.

---

## Explicitly out of scope

Named so they are decisions rather than oversights. None block tag authoring.

- **New Unreal assets** — Blueprints, skeletal meshes, materials, textures.
  Textures and material-instance parameters are tractable with the same
  machinery and are the obvious follow-on; Blueprints and meshes realistically
  need the UE 5.5.4 editor plus `retoc to-zen`.
- **Creating new scenarios** — different flags, level-pak resident, and every
  scenario is imported by nothing. Editing existing ones is in scope.
- **New audio** — the Wwise path, separately established as feasible, unbuilt.
- **Whether CE's engine-class layouts diverge from stock UE 5.5.4** — only
  matters for the excluded editor-cooking path. Checkable by diffing against a
  stock 5.5.4 usmap.

---

## Dependency order

```
Phase 0  spike                        (no dependencies, no new code)
   │
Phase 1  1.1 encoder ──► 1.2 builder  (gates 1.1, 1.2a, 1.2b)
   │
Phase 2  2.1 tables
         2.2 CookedAssets  ──┐
         2.3 AssetReference ─┼──► 2.6 writer rewiring
         2.4 RuntimeVariants ┤
         2.5 region tables ──┘
         2.7 robustness
   │
Phase 3  in-game matrix rows 2, 4–8
   │
Phase 4  4.1 persistence ──► 4.2 panel ──► 4.3 New Tag ──► 4.4 lint ──► 4.5 preview
   │
Phase 5  loose ends
```

Phase 0 gates everything: a negative result there changes Phase 2.6 and adds a
registration step, but leaves Phases 1, 2.1–2.5 and 4 untouched.

### Remaining class sweep (measured)

| Class | Sampled | Decoded | Finding |
|---|---|---|---|
| `AkAudioEvent` | 3,000 | 3,000 | **zero reflected properties** — pure native serialization; already handled by `iostore/wwise_event.rs` |
| `World` (`.umap`) | 400 | 400 | the `UWorld` export is 18–22 bytes, 1 property. The level's content is the *other* exports (median 30, max 2,232 per package) — actors and components. Covering levels means covering that class long tail, not `UWorld`. |
| `LevelSequence` | 121 | 52 | hard refs `MovieScene`, `CompiledData`, `Signature`; 69 fail |
| `NiagaraSystem` | 400 | 0 | native serialization throughout |

### Classes requiring native-format work (not reflected properties)

Each is a separate reverse-engineering effort, listed so the scope is explicit:

- `FTexturePlatformData` — pixel format, mip descriptors, inline/bulk mips
- `FStaticMeshRenderData` / `FSkeletalMeshRenderData` — vertex/index buffers
  (partially covered by `iostore/static_mesh.rs`, `skeletal_mesh.rs`,
  `nanite.rs`)
- Niagara system/emitter/script serialization
- `MovieScene` evaluation data
- Wwise `.bnk`/`.wem` (already covered by the audio module)

Plus the reflected-property blockers: the `native_struct_size` additions needed
for `StaticMesh` (`MeshUVChannelInfo`) and `SkeletalMesh`.
