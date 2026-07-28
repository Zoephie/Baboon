# Campaign Evolved tag packages — complete specification

Every rule here was measured against all **12,291** shipped tag packages in
`Meteorite/Content/Paks` (build `2026.06.26.1097863-Release`, UE 5.5.4), the
UHT header dump, and the shipped `.usmap`. Percentages are exact counts, not
estimates. Where a rule is not 100%, the exceptions are enumerated and
explained.

Ground-truth extractor: `blam-tags/examples/ce_tag_ground_truth.rs` (one JSON
record per tag). All other `ce_*` probes referenced below live alongside it.

---

## 1. What a tag is

A CE tag is **two chunks of one UE IoStore package**:

| Chunk | Contents |
|---|---|
| `.uasset` (`ExportBundleData`, index 0) | a Zen package header + one export: a `UBlam<Group>TagDataAsset` |
| `.ubulk` (`BulkData`, index 0) | the byte-complete Reach-format tag blob |

The simulation runs off the blob. The `.uasset` exists to give the blob a UE
identity, to hold hard references so its dependencies are preloaded, and (for
22 groups) to bind the tag to an Unreal asset.

---

## 2. Derivation rules — all 100% over 12,291 tags

Given a Halo tag path `objects\characters\spartans\spartans` and group `biped`:

| Field | Rule | Verified |
|---|---|---|
| package name | `/Game/Tags/objects/characters/spartans/spartans-biped` | — |
| `FPackageId` | `cityhash64(lowercase UTF-16LE package name)` | 12291/12291 |
| export object name | `spartans-biped` (the leaf) | 12291/12291 |
| `public_export_hash` | `cityhash64(lowercase UTF-16LE object name)` | 12291/12291 |
| `class_index` | ScriptImport hash of `/Script/BlamSynchronization.Blam<PascalCase(group)>TagDataAsset` | 12291/12291 |
| `template_index` | same, `Default__` prefixed | 12291/12291 |
| `.uasset` chunk id | `(package_id, 0, ExportBundleData=1)` | — |
| `.ubulk` chunk id | `(package_id, 0, BulkData=2)` | — |

`PascalCase` splits on `_` and upper-cases each initial: `frame_event_list` →
`BlamFrameEventListTagDataAsset`. This holds for all 101 shipped groups.

> `BlamFrameEventListTagDataAsset` exists in the game but is **absent from both
> the UHT dump and the `.usmap`**. Decode/encode it as `BlamTagDataAssetBase`.
> Do not derive the group→class table from the usmap.

### Structural constants (no exceptions in 12,291)

- exactly **1 export**; `outer_index`, `super_index` both Null
- exactly **1 bulk-data map entry**, `flags = 66817` (`0x10501`),
  `serial_offset = 0`, `duplicate_serial_offset = -1`,
  `serial_size = len(.ubulk)`
- `is_unversioned = true`
- export bundle entries: `[Create(0), Serialize(0)]`
- exactly **1** dependency-bundle header, always
  `(0, 0, N, 0)` — every dependency is `create_before_serialize` — and
  `N == len(dependency_bundle_entries)`
- 4-byte trailer after the export body: `c1 83 2a 9e` (`PACKAGE_FILE_TAG`)
- 0 shader map hashes, 0 cell imports/exports

### Flags — three variants, not one

| `object_flags` | `package_flags` | Count | Applies to |
|---|---|---|---|
| `0xb` | `0x80002200` | 9,416 | the default for every group |
| `0x3` | `0x80002200` | 2,563 | 2,562 `sound` + 1 `ai_mission_dialogue` |
| `0x1` | `0x88002200` | 312 | **all** `scenario`, `scenario_structure_bsp`, `scenario_structure_lighting_info`, `structure_design`, `structure_seams` |

`0x80002200` = `PKG_FilterEditorOnly | PKG_UnversionedProperties | PKG_Cooked`.
The extra `0x08000000` is `PKG_CookGenerated` — those 312 are the `_Generated_`
level tags and they live in per-level paks, never `pakchunk0`.
`0xb` = `RF_Public|RF_Standalone|RF_Transactional`; `0x3` drops
`RF_Transactional`, which is inert in a cooked build. Only `sound` and
`ai_mission_dialogue` are mixed — **use `0xb`/`0x80002200` for everything
except the five `_Generated_` groups.**

### Import map

Every tag has exactly **3 script imports**: the class, its `Default__` CDO, and
the module package `/Script/BlamSynchronization` (hash `24D5BCDF3D9D342`).
Verified 12,290/12,291 — the sole exception is
`player_model_customization_globals`, which has 5.

Beyond that, per imported package the cooker emits **one `Null` slot** (the
`UPackage` itself) plus one `PackageImport` slot per imported export.
`sum(null slots) == sum(len(imported_package_names))` = **25,476/25,476**.

Ordering rules, all **8733/8733** over tags that have imports:

- `imported_packages` is sorted **ascending by `FPackageId`**, and
  `imported_package_names[i]` is the name hashing to `imported_packages[i]`
- `imported_public_export_hashes` has **one entry per unique
  `(imported_package_index, hash)` pair**, in import-map first-use order.
  Deduping by hash alone is wrong and costs 8 bytes per collision.

The **import-map slot order itself is the cooker's and is not reproducible** —
it interleaves script/package/null slots in linker order. It does not need to
be: the array is only indexed from inside the package (export properties and
dependency-bundle entries), so any self-consistent order loads identically.
See §7 for what this means for the test gates.

Of 92,006 total import slots, 25,266 are named by the export's properties,
25,476 are the Null package slots, 36,875 are the 3 script imports, and **4,389
are vestigial** — package imports that no property and no dependency entry
references (BP sub-objects, all in the 12 object-ish groups). They are cook
residue and are safe to drop.

### Dependency bundle

The entries are **exactly the imports the export's properties reference** —
25,266 == 25,266 corpus-wide, and per-tag **12,291/12,291**.

Their **order** is the *reversed* usmap property order — base-class properties
first, then derived. So `CookedAssetsReferencedByTag` entries come first, then
`AssetReference` / `ModelRegionStringTable`. Verified **1089/1089** on tags with
two or more object-valued properties (on single-property tags both orders
coincide).

### Name map

`names = sort_case_insensitive(unique FName base names used by the export's
properties) + [object name, package name]` — **12,288/12,291**.

The 3 exceptions are `FName` *numbers*: `default` and `default_0` are one
name-map entry with `number` 0 and 1. A writer must fold a trailing `_<n>` back
into `(base, number+1)` when `base` is already present, which is ordinary UE
`FName` semantics. `FSoftObjectPath` package/asset/sub strings are FNames too
and participate in the map (this is what makes
`player_model_customization_globals` a 71-name outlier).

---

## 3. The wrapper's property surface — all of it

Of the 178 declared `Blam*TagDataAsset` classes, **only 9 add any property at
all**. Everything else is bare `BlamTagDataAssetBase`.

| Class | Properties |
|---|---|
| `BlamTagDataAssetBase` (universal) | `CookedAssetsReferencedByTag: TArray<UBlamTagDataAssetBase*>`, `BinaryBlobSize: uint32` (Transient) |
| `BlamObjectTagDataAsset` | `AssetReference: TSubclassOf<ABlamObjectActor>`, `DefaultAssetReference` (Transient) |
| `BlamBaseEffectTagDataAsset` | `AssetReference`, `bSpawnPerInstance: bool`, `DefaultAssetReference` (Transient) |
| `BlamBaseSoundDefinitionTagDataAsset` | `AssetReference: UObject*`, `DefaultAssetReference` (Transient) |
| `BlamBaseLoopingSoundTagDataAsset` | same |
| `BlamBaseSoundCombinerTagDataAsset` | same |
| `BlamCinematicTagDataAsset` | same |
| `BlamDamageResponseDefinitionTagDataAsset` | same |
| `BlamModelTagDataAsset` | `ModelRegionStringTable: UBlamModelRegionStringTable*`, `RegionTable: TArray<FName>`, `Permutations_EMPTY` (Transient), `Variants: TArray<FBlamVariant>`, `RuntimeVariants: TArray<FBlamVariant>`, `ObjectTagDataAsset` (Transient) |
| `BlamPlayerModelCustomizationGlobalsTagDataAsset` | `CustomizationDataTables: TMap<SoftObject,SoftObject>`, `CustomizationIndicesLookup: TMap<GameplayTag,FBlamCustomizationGlobalsTagDataIndices>` |

**Never serialized** anywhere in the corpus — a builder must not emit them:
`BinaryBlobSize`, `DefaultAssetReference`, `Permutations_EMPTY`,
`ObjectTagDataAsset`, and on `model` both `Variants` (0/451) and `RegionTable`
(0/451). Only `RuntimeVariants` ships.

### Per-group breakdown

**47 bare groups** (wrapper carries nothing at all):

`achievements`, `ai_dialogue_globals`, `camera_fx_settings`, `camera_shake`,
`camera_track`, `camo`, `chud_animation_definition`,
`chud_widget_placement_data_template`, `chud_widget_render_data_template`,
`chud_widget_state_data_template`, `cinematic_transition`, `collision_damage`,
`collision_model`, `color_table`, `coop_spawning_globals_definition`,
`encounter_remix`, `formation`, `game_engine_settings_definition`,
`game_medal_globals`, `game_performance_throttle`, `grounded_friction`,
`havok_collision_filter`, `incident_globals_definition`,
`megalo_string_id_table`, `megalogamengine_sounds`,
`multilingual_unicode_string_list`, `multiplayer_object_type_list`,
`physics_model`, `rain_definition`, `rumble`,
`scenario_structure_lighting_info`, `shader`, `shield_impact`,
`simulated_input`, `simulation_interpolation`, `skeleton_model`,
`sound_classes`, `sound_dialogue_constants`, `sound_global_propagation`,
`sound_mix`, `sound_radio_settings`, `spring_acceleration`, `structure_design`,
`structure_seams`, `style`, `text_value_pair_definition`,
`water_physics_drag_properties`

**54 wrapper-bearing groups.** 32 carry only `CookedAssetsReferencedByTag`.
The 22 that carry more:

| Group | Tags | Wrapper fields present |
|---|---|---|
| `biped` | 32 | AssetReference 27, CookedAssets 31 |
| `crate` | 292 | AssetReference 267, CookedAssets 290 |
| `creature` | 2 | AssetReference 2, CookedAssets 1 |
| `device_control` | 20 | AssetReference 19, CookedAssets 19 |
| `device_machine` | 63 | AssetReference 63, CookedAssets 61 |
| `device_terminal` | 2 | AssetReference 2, CookedAssets 1 |
| `effect_scenery` | 4 | AssetReference 1, CookedAssets 3 |
| `equipment` | 20 | AssetReference 15, CookedAssets 19 |
| `giant` | 1 | AssetReference 1 |
| `projectile` | 61 | AssetReference 34, CookedAssets 60 |
| `scenery` | 30 | AssetReference 27, CookedAssets 20 |
| `sound_scenery` | 2 | AssetReference 2 |
| `vehicle` | 25 | AssetReference 22, CookedAssets 24 |
| `weapon` | 75 | AssetReference 49, CookedAssets 74 |
| `effect` | 884 | AssetReference 100, CookedAssets 459, **bSpawnPerInstance 5** |
| `sound` | 5895 | AssetReference 5595 |
| `sound_looping` | 163 | AssetReference 123, CookedAssets 142 |
| `sound_combiner` | 2 | AssetReference 1, CookedAssets 1 |
| `cinematic` | 45 | AssetReference 45, CookedAssets 44 |
| `damage_response_definition` | 155 | AssetReference 9, CookedAssets 153 |
| `model` | 451 | CookedAssets 451, **RuntimeVariants 408, ModelRegionStringTable 364** |
| `player_model_customization_globals` | 1 | CustomizationDataTables, CustomizationIndicesLookup |

An absent property means "class default" (null / empty), which is legal
everywhere — e.g. 784 of 884 `effect` tags and 300 of 5,895 `sound` tags have
no `AssetReference` at all.

---

### The complete type surface a property writer must cover

Every value in every tag export across the corpus, exhaustively:

| Kind | Instances | Appears in |
|---|---|---|
| `Object` (`FPackageIndex` i32) | 25,266 | `AssetReference`, `CookedAssetsReferencedByTag`, `ModelRegionStringTable` |
| `Name` (`FName`) | 4,445 | `RuntimeVariants`, `CustomizationIndicesLookup` |
| `Array` | 3,457 | `CookedAssetsReferencedByTag`, `RuntimeVariants` |
| `Struct` | 754 | `FBlamVariant`, `FBlamCustomizationGlobalsTagDataIndices` |
| `Map` | 682 | `FName→FName`, `FGameplayTag→struct` |
| `Int` | 74 | `CustomizationIndicesLookup` |
| `SoftObjectPath` | 16 | `CustomizationDataTables` |
| `Bool` | 5 | `bSpawnPerInstance` (all `true`) |

That is the whole surface — seven kinds. No float, no string, no enum, no
native struct.

`FUnversionedHeader` facts, measured with the correct fragment layout
(`skip` = bits 0–6, `has_zeroes` = bit 7, `is_last` = bit 8, `value_num` =
bits 9+):

- **no tag export uses a has-zeroes fragment** — the zero mask is never needed.
  (Exactly one `.uasset` under `/Content/Tags/` does, and it is a Blueprint,
  `BP_PulseConduit_C45`, not a tag.) The 5 `bSpawnPerInstance` bools serialize
  as a real 1-byte value, not as a mask bit.
- 3.12 fragments per export on average, **5 maximum**
- **every tag export body begins with 4 zero bytes and ends with 4 zero bytes**,
  both *inside* `cooked_serial_size` — 12,291/12,291 for the leading pair and
  12,329/12,329 for the trailing. The trailing pair is distinct from the package
  trailer `c1 83 2a 9e`, which follows `cooked_serial_size`.

  The leading 4 bytes parse as two no-op fragments (`skip=0, value_num=0,
  is_last=0`), so a reader consumes them harmlessly — but a *writer* that emits
  only the minimal fragments produces a body 4 bytes short and fails
  byte-identity. **Emit the 4-zero prefix.** The prefix is constant regardless of
  how many properties the export carries, so it is a fixed frame, not a
  position-dependent value.

So the body layout is:
`[00 00 00 00] [fragments…] [values…] [00 00 00 00]`

Flattened schemas also contain `DefaultAssetReference`, `BinaryBlobSize` and
`NativeClass` (inherited from `UDataAsset`); none is ever emitted, so the
encoder reaches them only as skip counts.

### Things that do not exist and need no handling

- **no localized tag variants** — 2,316 `/Game/L10N` packages, none under
  `/Tags/`, so the container header's localized-package machinery is unused
- **no optional-segment chunks** — zero `.uptnl` and zero `.m.ubulk` anywhere in
  the game; one bulk chunk per package, always index 0

### One shipped tag is broken

`/Game/Tags/sound/characters/masterchief/FOL_Move_ET/FOL_MC_GearMove_Land-sound`
declares `serial_size = 4700` but **has no `.ubulk` chunk at all** — its five
siblings in the same folder all do. Tooling must degrade gracefully on a missing
bulk chunk rather than erroring.

---

## 4. `AssetReference` — computable from the target path alone

**6404/6404, zero misses.** No donor tag and no Blueprint parsing needed:

- target is a Blueprint class (`TSubclassOf`): hash =
  `cityhash64(lowercase UTF-16 "<PackageLeaf>_C")` — 631 cases, exactly the 14
  object groups plus `effect`
- target is a plain asset (`UObject*`): hash =
  `cityhash64(lowercase UTF-16 "<PackageLeaf>")` — 5,773 cases, exactly
  `sound`, `sound_looping`, `sound_combiner`, `cinematic`,
  `damage_response_definition`

No target package is ever referenced with two different hashes (0 of 6,278).

Which rule applies is decided by the declaring class, so it is known statically
from the group.

**Useful defaults**, from the target census:
`/Game/_Prototypes/SynchronizationTestContent/TestActor/BP_EmptyActor` is the
game's own generic actor, used by `biped`, `crate`, `creature`, `device_control`,
`device_machine`, `effect_scenery`, `equipment`, `giant`, `projectile`.
`damage_response_definition` has only ever pointed at two assets
(`DA_Default-PlayerEffect`, `DA_AreaOfEffect-PlayerEffect`).

---

## 5. `CookedAssetsReferencedByTag` — derived from the blob

Reconciled against every tag's own `tag_reference` set:

| Case | Count |
|---|---|
| array absent, blob has 0 resolvable refs (correct) | 9,118 |
| array present and **exactly equal** to the blob refs | 2,968 |
| array present, a **strict subset** (cook-culled) | 47 |
| array present, a **strict superset** | 34 |
| array absent but the blob has refs | 124 |
| differs in both directions | **0** |

**98.7% is exact or an explained subset.** The exceptions:

- **47 subsets** — deliberate cook-time culling on `character` (36), `biped`
  (4), `model_animation_graph` (3), `frame_event_list` (2), `skull_globals`,
  `multiplayer_globals`. These are tags that would otherwise hard-preload half
  the game (an AI `character` naming every weapon and vehicle it can use).
- **124 absent-with-refs** — 122 `squad_template` plus
  `player_model_customization_globals` and `multiplayer_object_type_list`. The
  other 220 squad_templates of identical shape *do* carry the array; no folder,
  ref-count or content discriminator exists. This is cook non-determinism, and
  it proves the array is a preload optimisation rather than a correctness
  requirement *when something else already loads the target*.
- **34 supersets** — fully explained: 13 `scenario` tags each add
  `globals-globals`; 11 `model_animation_graph` add their sibling
  `-frame_event_list`; 10 `frame_event_list` add refs that are covered by their
  sibling `model_animation_graph`'s reference set (jmad and fel are cooked as a
  unit and share a union).

**Authoring rule:** emit every blob `tag_reference` that resolves to a package
we ship or that exists in the base game. That is a superset of shipped
behaviour, and supersets are demonstrably safe (they only preload more).

### Dangling references are normal

1,157 reference instances point at tag paths that are not shipped packages, plus
224 whose group has no definition at all (`ligh` 124, `lens` 53, `gldf` 27,
`ltvl` 18, `mode` 2 — groups whose rendering role moved to Unreal). Top dangling
targets: `sound` 637, `scenario_structure_bsp` 122,
`scenario_structure_lighting_info` 122, `sound_looping` 53, `effect` 48.
A validator must **warn, not error**.

---

## 6. Inferable from tag data, or needs UI?

### Fully inferable — never ask the user

- everything in §2 (identity, hashes, flags, orderings, name map, dependency
  bundle)
- `CookedAssetsReferencedByTag` — §5
- `RuntimeVariants` on `model`:
  `{VariantName: variants[i].name, Permutations: {region.name →
  region.permutations[0] or "None"}}` — **407/408 exact** (43 model tags have no
  variants and correctly omit the property). The single outlier drops a region
  the render_model does not have.
- `AssetReference`'s *hash*, given the target path — §4

### Genuinely Unreal-only — needs UI

- **`AssetReference`'s target package.** Not inferable: `sound` leaf-names match
  the tag leaf only 1119/5719, `cinematic` 1/45. This has always been the real
  tag→Unreal bridge; the actor's meshes, components and firing sounds are
  compiled into the Blueprint and unreachable from any tag.
- **`ModelRegionStringTable`'s target.** 118 distinct packages, inconsistent
  naming (`RT_*` / `DA_*` / `rst_*`) scattered across `/Game`, and 87 of 451
  model tags have none. **214 of 364 share
  `/Game/Tags/objects/shared/RT_default_object_regions`** — the obvious default.
- **`bSpawnPerInstance`** — a checkbox; 5 tags, all `true`, absent = false.
- **`CustomizationDataTables` / `CustomizationIndicesLookup`** — one tag in the
  game; a map editor, lowest priority.

### Half-inferable — offer a generate action, not an inference

The *contents* of a `BlamModelRegionStringTable`: `Regions` matches the model
tag's variant regions **278/364**, but `Permutations` is the **mesh-side**
vocabulary (the union over the mesh-sync data), not the tag's designated subset
— e.g. `DA_Marine_Regions` lists 88 permutations shared across marine/johnson.
The asset itself is a 370-byte `UDataAsset` with **no bulk data** and 3 script
imports, so it is trivially synthesizable: take `Regions` from the tag and
`Permutations` from the mesh-sync data blam-tags already reads.

---

## 7. How a tag is found at runtime

There is **no asset registry and no tag manifest** in the paks.

From the shipped exe, the resolver builds a full object path with
`FString::Printf` and resolves it as an `FSoftObjectPath`:

```
"%s/%s%s-%s.%s-%s"                  with "/Game/Tags"   ->  /Game/Tags/<dir>/<name>-<group>.<name>-<group>
"%s/Default/default-%s.default-%s"  with "/Game/Tags"   ->  /Game/Tags/Default/default-<group>.default-<group>
```

The group string comes from a runtime-built table indexed by group index (it
lives in the uninitialized tail of `.data`, so it cannot be dumped statically).

So resolution is **by name, with a per-group `Default/default-<group>`
fallback** — which is why an unresolvable reference degrades to the group
default instead of crashing, and why the 22 `/Game/Tags/Default/default-*` tags
are imported by nothing yet obviously load.

Reachability across all 103,867 cooked packages:

- 18,740 of 18,858 import edges into `/Game/Tags` come from **other tags**
- only 118 come from non-tag packages (levels 40, characters 26, vehicles 26,
  weapons 22, blueprints 20, …); only 20 tags are reachable *only* that way
- **334 tags are imported by nothing** — 312 of them are exactly the
  `_Generated_` level groups (every `scenario` among them), the rest are the
  `Default/` fallbacks

A single scenario's package-import closure reaches 37,475 packages and 74.8% of
all tags.

### There is a runtime tag registry

The function that builds those paths (`0x140000000 + 0x075f7790`) is not a
per-lookup resolver — it is a **registry builder**. It walks a per-group array
at `+0x90` of a subsystem object, formats the object path, constructs an
`FSoftObjectPath`, and inserts it into a `TMap` (visible as the standard UE
bucket machinery: `-1` sentinels, `hash & (size-1)`, `[map+0x48]` bucket count).
Its two callers are `0x76f030b` and `0x7712a40`; the latter references the
string `'BlamEngine'`, so the registry is populated at BlamEngine subsystem
init. The sibling at `0x075f8810` builds the `Default/default-<group>` fallback
path and is called from `0x76fc1ad`.

So tag identity → `FSoftObjectPath` → UE's soft-object-path machinery, which has
both find-only (`ResolveObject`) and load-on-demand (`TryLoad`) modes.

> **The one open question that gates the design.** Is that registry populated by
> **enumerating packages**, or from a **fixed list**?
>
> - Enumerated → new tags are discovered automatically,
>   `CookedAssetsReferencedByTag` is purely a preload optimisation, and the
>   authoring plan is complete as written.
> - Fixed list → a new tag must also be registered in that list, which is an
>   entire additional requirement.
>
> `scenario_required_resource` is **not** that list (it holds 35 `character` +
> 20 `weapon` refs for skulls). Static analysis has hit its limit here: symbols
> are absent, dispatch is virtual (the call graph from the resolver reaches only
> 57 functions statically), and the group-name table lives in the uninitialised
> tail of `.data`, so it exists only at runtime. Settling this needs IDA's
> decompiler on `0x075f7790` and its callers, or a live breakpoint — minutes
> with either, hours without.

---

## 8. The two defects this exposes

In `blam-tags/src/iostore/writer.rs`:

1. **`add_override_to_writer`** re-emits the base game's `.uasset` verbatim,
   patching only `serial_size`, and only when the size changed. Repointing a
   reference at a **new** tag therefore never adds the import, so the new tag is
   never preloaded. (Repointing at an *existing* tag often works by luck —
   74.8% of tags are already in the closure — which is exactly the kind of
   inconsistency that is hard to diagnose from symptoms.)
2. **`add_new_package_to_writer`** clones a same-group template and copies its
   export body and import map verbatim, so a new tag inherits the template's
   `AssetReference` and dependency list.

For the 47 bare groups, cloning happens to be correct. For the other 54 it is
not.

---

## 9. What is actually proven in-game, and the debug surface

**The container-header registration data is correct.** For 351 shipped tag
packages, the `StoreEntry` our writer derives (via
`FZenPackageHeader::serialize`) is **identical to the game's own** —
`export_bundles_size`, `imported_packages` and `shader_map_hashes` all match.
So what we would register for a new package is byte-correct; only whether the
engine *honours* a mod-supplied header is untested.

**The in-game-proven surface is narrower than it looks.** The mod that was
confirmed loading (`mymod-WinGDK_P`) contains **one chunk**: a `BulkData`
override of package `09bec3c47f33c65e` =
`/Game/Tags/objects/characters/marine/marine-biped`, same size as the original
(45,270 bytes). No `.uasset`, no `ContainerHeader`, no new package. So
*same-size tag edits* are proven; **size-changing edits (which patch the
`.uasset`), new packages, and container-header registration are all unproven**.

**The retail simulation DLL retains the full Blam debug command set.**
`HaloSimulation_tag_release.dll` exports only `CreateBlamEngineShell` (one
vtable), but its string tables carry the Blam command and debug-global pools,
including:

| Name | Kind | Why it matters |
|---|---|---|
| `tag_load_force` | command | force-load a tag **by name** |
| `tag_unload_force`, `tag_reload_force` | command | the other half of the cycle |
| `tag_is_active` | command | **query whether a given tag is loaded** |
| `dump_loaded_tags` | command | dump the whole loaded set |
| `scenario_load_all_tags` | debug global | toggle loading every tag for a scenario |
| `debug_tag_dependencies`, `debug_single_tag`, `debug_tags` | debug global | dependency tracing |
| `enable_tag_edit_sync`, `enable_tag_resource_xsync` | debug global | the live-edit sync path |

`tag_is_active` + `dump_loaded_tags` + `tag_load_force` are precisely the three
primitives needed to settle the load model empirically, without a debugger.

**The retail build also ships ImGui and a Blam debug menu.** `/Script/ImGui`,
`/Script/HaloImGuiUtils`, `SBlamDebugMenuWidget` and `UBlamDebugMenuWidget` are
all present in the shipping exe, gated by:

```
UCLASS(Blueprintable, DefaultConfig, Config=Game)
class UDebugMenuSettings : public UDeveloperSettings {
    bool bEnableDebugMenuBetaShipping    = false;
    bool bEnableDebugMenuReleaseShipping = false;
    bool bEnableDebugMenuDefaultShipping = false;
    // …NonShipping variants all default true
};
```

Because these are `Config=Game` properties, a `Game.ini` override is the
obvious thing to try:

```ini
[/Script/Meteorite.DebugMenuSettings]
bEnableDebugMenuDefaultShipping=True
bEnableDebugMenuReleaseShipping=True
bEnableDebugMenuBetaShipping=True
```

Untested. `Meteorite/Content/Config` ships only an `Achievements` folder, so the
project config is baked in; whether the user-layer config is still consulted for
a `DefaultConfig` class in this build is the open part.

---

## 10. Test gates

Two distinct gates, because the import-map slot order is not reproducible (§2):

- **Round-trip gate (byte-identity).** Parse a shipped package, rebuild it with
  the builder using the parsed import order, re-serialize: must be
  byte-identical for all 12,291. This proves the serializer, the property
  encoder, and every derivation in §2–§6. Already demonstrated for 9 of 10
  sampled groups by `ce_tag_pkg_synth.rs`; the tenth was off by exactly 8 bytes
  from deduping `imported_public_export_hashes` by hash instead of by
  `(package, hash)` pair.
- **From-scratch gate (semantic).** Build from a canonical order, parse back,
  and assert: identical resolved property targets, identical dependency set,
  identical blob-ref set, identical identity hashes and flags.
