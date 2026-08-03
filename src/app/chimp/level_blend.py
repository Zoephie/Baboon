"""Build .blend files from a Baboon level export.

Run this inside Blender: Scripting tab, open this file, press Run. Or from a
terminal:

    blender --background --python build_blend.py

It reads the .baboonlevel file sitting next to it and writes:

    meshes/<name>.blend        one file per mesh, holding only that geometry
    <name>_master_NN.blend     one per region, linking those meshes and placing
                               them in world space

The masters *link* rather than append, so a mesh placed a thousand times is
stored once and every placement of it is a linked duplicate. That is the whole
point of the split, and it is why the meshes folder has to stay beside the
masters.

Geometry arrives in Blender's own conventions - triangles wound
counter-clockwise, V running up the image - so nothing here converts anything.
The one exception is the placement matrix, which is stored the way Unreal holds
it (row-major, translation last) and is transposed below, because Blender takes
column vectors.
"""

import os
import struct
import sys

import bpy  # type: ignore
import numpy as np  # type: ignore
from mathutils import Matrix  # type: ignore

MAGIC = b"BABOONLV"
VERSION = 1
# mesh index, segment index, then a 4x4 of doubles.
PLACEMENT_SIZE = 4 + 4 + 16 * 8


def script_directory():
    """Where this script lives, whether run from the text editor or the CLI."""
    if bpy.context.space_data is not None and getattr(bpy.context.space_data, "text", None):
        path = bpy.context.space_data.text.filepath
        if path:
            return os.path.dirname(os.path.abspath(path))
    if "__file__" in globals():
        return os.path.dirname(os.path.abspath(__file__))
    return os.getcwd()


def find_export(directory):
    for entry in sorted(os.listdir(directory)):
        if entry.endswith(".baboonlevel"):
            return os.path.join(directory, entry)
    raise RuntimeError(
        "no .baboonlevel file next to this script - keep them in the same folder"
    )


class Export:
    """The sidecar, read into memory as flat arrays."""

    def __init__(self, path):
        with open(path, "rb") as handle:
            data = handle.read()
        if data[:8] != MAGIC:
            raise RuntimeError(f"{path} is not a Baboon level export")
        version, mesh_count, placement_count, segment_count = struct.unpack_from(
            "<IIII", data, 8
        )
        if version != VERSION:
            raise RuntimeError(
                f"{path} is version {version}; this script reads version {VERSION}. "
                "Re-export from the Baboon that wrote this script, or use the "
                "script that came with the file."
            )
        self.name = os.path.splitext(os.path.basename(path))[0]
        self.segment_count = segment_count
        self.meshes = []

        at = 24
        for _ in range(mesh_count):
            (name_length,) = struct.unpack_from("<I", data, at)
            at += 4
            name = data[at : at + name_length].decode("utf-8")
            at += name_length
            vertices, triangles = struct.unpack_from("<II", data, at)
            at += 8

            positions = np.frombuffer(data, dtype=np.float32, count=vertices * 3, offset=at)
            at += vertices * 3 * 4
            normals = np.frombuffer(data, dtype=np.float32, count=vertices * 3, offset=at)
            at += vertices * 3 * 4
            uvs = np.frombuffer(data, dtype=np.float32, count=vertices * 2, offset=at)
            at += vertices * 2 * 4
            indices = np.frombuffer(data, dtype=np.uint32, count=triangles * 3, offset=at)
            at += triangles * 3 * 4

            self.meshes.append(
                {
                    "name": name,
                    "vertices": vertices,
                    "triangles": triangles,
                    "positions": positions,
                    "normals": normals,
                    "uvs": uvs,
                    "indices": indices,
                }
            )

        self.placements = []
        for _ in range(placement_count):
            mesh, segment = struct.unpack_from("<II", data, at)
            values = struct.unpack_from("<16d", data, at + 8)
            at += PLACEMENT_SIZE
            self.placements.append((mesh, segment, values))


def build_mesh(entry):
    """One Blender mesh datablock, filled from flat buffers.

    `foreach_set` takes the arrays as they are, so no per-vertex Python runs
    here - which is the difference between seconds and hours on a mesh with a
    hundred thousand vertices.
    """
    mesh = bpy.data.meshes.new(entry["name"])
    vertices = entry["vertices"]
    triangles = entry["triangles"]

    mesh.vertices.add(vertices)
    mesh.vertices.foreach_set("co", entry["positions"])

    # Every face is a triangle, so the loop layout is known without building
    # per-polygon lists.
    mesh.loops.add(triangles * 3)
    mesh.polygons.add(triangles)
    mesh.loops.foreach_set("vertex_index", entry["indices"])
    mesh.polygons.foreach_set("loop_start", np.arange(0, triangles * 3, 3, dtype=np.int32))
    mesh.polygons.foreach_set("loop_total", np.full(triangles, 3, dtype=np.int32))

    uv_layer = mesh.uv_layers.new(name="UVMap")
    # UVs are per vertex in the export and per loop in Blender, so they are
    # expanded through the index buffer.
    per_loop = entry["uvs"].reshape(-1, 2)[entry["indices"].astype(np.int64)]
    uv_layer.data.foreach_set("uv", per_loop.ravel())

    mesh.update()
    mesh.validate()

    # Custom split normals, so the shading is the one the game had rather than
    # one recomputed from the faces. Set after validate(), which discards them.
    normals = entry["normals"].reshape(-1, 3)
    try:
        mesh.normals_split_custom_set_from_vertices(normals)
    except AttributeError:
        # Blender 4.0 and earlier want them per loop.
        mesh.normals_split_custom_set(normals[entry["indices"].astype(np.int64)])
    return mesh


def write_mesh_libraries(export, directory):
    """One .blend per mesh, each holding only its own geometry."""
    folder = os.path.join(directory, "meshes")
    os.makedirs(folder, exist_ok=True)
    paths = []
    for index, entry in enumerate(export.meshes):
        mesh = build_mesh(entry)
        path = os.path.join(folder, f"{entry['name']}.blend")
        # `libraries.write` saves datablocks to another file without disturbing
        # this session, which is what lets one run produce hundreds of files.
        bpy.data.libraries.write(path, {mesh}, fake_user=True, compress=True)
        paths.append((path, mesh.name))
        # The session keeps nothing: the geometry is in the file now, and
        # holding every mesh at once is the memory ceiling this avoids.
        bpy.data.meshes.remove(mesh)
        if (index + 1) % 25 == 0:
            print(f"  {index + 1}/{len(export.meshes)} meshes written")
    return paths


def build_master(export, directory, segment, mesh_paths):
    """One master scene: link each mesh used here, then place it."""
    bpy.ops.wm.read_homefile(use_empty=True)
    collection = bpy.context.scene.collection

    placements = [p for p in export.placements if p[1] == segment]
    used = sorted({mesh for mesh, _, _ in placements})
    linked = {}
    for index in used:
        path, name = mesh_paths[index]
        # Link, not append: the geometry stays in its own file and every
        # placement of it here is a reference to that one copy.
        with bpy.data.libraries.load(path, link=True) as (source, target):
            target.meshes = [n for n in source.meshes if n == name]
        linked[index] = bpy.data.meshes[name, path] if (name, path) in bpy.data.meshes else bpy.data.meshes[name]

    for number, (mesh_index, _, values) in enumerate(placements):
        mesh = linked.get(mesh_index)
        if mesh is None:
            continue
        obj = bpy.data.objects.new(f"inst_{number}", mesh)
        # The export stores Unreal's row-major matrix with the translation last;
        # Blender takes column vectors, so it transposes.
        obj.matrix_world = Matrix([values[0:4], values[4:8], values[8:12], values[12:16]]).transposed()
        collection.objects.link(obj)

    path = os.path.join(directory, f"{export.name}_master_{segment:02d}.blend")
    bpy.ops.wm.save_as_mainfile(filepath=path, compress=True)
    return path, len(placements)


def main():
    directory = script_directory()
    export = Export(find_export(directory))
    print(
        f"{export.name}: {len(export.meshes)} meshes, "
        f"{len(export.placements)} placements, {export.segment_count} segment(s)"
    )

    print("writing mesh libraries...")
    mesh_paths = write_mesh_libraries(export, directory)

    print("building masters...")
    for segment in range(export.segment_count):
        path, count = build_master(export, directory, segment, mesh_paths)
        print(f"  {os.path.basename(path)}: {count} placements")

    print("done. Open a master to see that region of the level.")
    if export.segment_count > 1:
        print(
            f"The level is split across {export.segment_count} masters - the whole "
            "of it does not open at once. They share a coordinate system, so any "
            "two line up exactly."
        )


if __name__ == "__main__":
    try:
        main()
    except Exception as error:  # noqa: BLE001 - the message is the whole point
        print(f"build_blend.py failed: {error}", file=sys.stderr)
        raise
