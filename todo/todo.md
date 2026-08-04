# Baboon — todo

Work that is known, understood and deliberately not done yet. Each entry says
what is missing, how much it costs, and what implementing it would involve, so
that picking one up does not mean rediscovering it first.

---

## Blueprint-inherited meshes in level export

**Status:** not implemented. Counted, not silently dropped.

### What is missing

A level export reads a `StaticMeshComponent`'s mesh from its own `StaticMesh`
property. Some components do not have one: they are Blueprint subobjects that
inherit the mesh from their class default object, which lives in the Blueprint's
own package rather than in the cell being read. Those placements are skipped.

`src/app/chimp/level.rs` counts them as `LevelSkips::inherited_mesh`, in the arm
that distinguishes "the property is there and did not resolve"
(`unresolved_mesh`) from "there is no property at all" (`inherited_mesh`):

```rust
Err(()) => {
    if property(block, "StaticMesh").is_some() {
        skips.unresolved_mesh += 1;
    } else {
        skips.inherited_mesh += 1;
    }
    continue;
}
```

### What it costs

Measured over all 2,334 cells of Campaign Evolved's C10:

| | placements | share |
| --- | --- | --- |
| `inherited_mesh` | 1,675 | 0.56% |
| `unresolved_mesh` | 225 | 0.08% |
| **total skipped** | **1,900** | **0.64%** |

Against 296,399 placements exported successfully. So this is a long tail, not a
hole — but it is 1,675 props that are in the game and not in the export, and
which ones is not currently reported.

### What implementing it involves

1. Resolve the component's owning class. The export's class is already resolved
   (`world.class_key`), but a Blueprint-generated class needs following to the
   package that defines it.
2. Load that package and find its class default object.
3. Read the CDO's `StaticMesh` property, and use it for any component of that
   class that has none of its own.
4. Cache per class. A Blueprint used a thousand times must not be loaded a
   thousand times — the level walk is 0.7s and this could easily undo that.

The reading side is the whole job; once a package name comes out, the rest of
the export path already handles it like any other mesh.

### Why it was left

It is a different problem from reading a cell: it means resolving and loading
*other* packages while walking, which is the first thing in the level path that
reaches outside the cell it is reading. At 0.56% it was never the thing standing
between the exporter and a usable level.

---

## Single full-level `.blend` master

**Status:** builds and loads headless; not verified interactively.

`c10_single_master_00.blend` places all 296,399 of C10's placements in one file
— 20 MB, since a master holds placements and links rather than geometry. It
opens headless with all 745 meshes library-linked and every object intact.

What is unknown is whether it is *workable*: 296,399 objects in the depsgraph
and draw manager is a different question from parsing them, and only opening it
in the UI answers it.

This decides a default. The Blender export currently does not split, on the
grounds that a master is cheap; if the viewport cannot cope, the placement
budget in `src/app/chimp/level_segment.rs` wants lowering and the format's
default in the export prompt should change with it.

Needs its own session — it is a measurement task, not a code change.
