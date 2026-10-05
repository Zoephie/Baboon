#!/usr/bin/env python3
"""Generate Baboon persisted-format compatibility fixtures.

Every value is synthetic. The shapes are transcribed from the readers and
writers Baboon ships (first at main@3f91f73); README.md beside this script
says which reader takes each file. Re-run to regenerate, then run
`cargo test compat_` to check them against the real readers.
"""
import hashlib, json, os, shutil, sqlite3

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "samples")

def gt(fourcc: str) -> int:
    return int.from_bytes(fourcc.encode("latin-1"), "big")

def fmt_group(fourcc: str) -> str:  # blam_tags::fields::format_group_tag
    return fourcc.rstrip("\0 ")

GAMES = ["haloce_mcc", "halo2_mcc", "halo2amp_mcc", "halo3_mcc", "halo3odst_mcc",
         "haloreach_mcc", "halo4_mcc", "haloce_evolved"]
FUTURE_GAME = "halo5_mcc"

# --- Roots -----------------------------------------------------------------
H3_WIN = r"C:\Program Files (x86)\Steam\steamapps\common\H3EK\tags"
HR_MIXED_ROOT = "C:/Kits/HREK/tags"  # user-typed forward slashes; walk adds '\'
H2_UNC = r"\\fileserver\share\H2EK\tags"
H4_POSIX = "/Users/me/Kits/H4EK/tags"
CE_INSTALL = r"D:\XboxGames\Halo Campaign Evolved\Content"
CE_PAKS = CE_INSTALL + r"\Meteorite\Content\Paks"
H3_MAPS = r"C:\Program Files (x86)\Steam\steamapps\common\Halo The Master Chief Collection\halo3\maps"

# --- Every TagKey kind -------------------------------------------------------
K = {
    "file_windows": "file:" + H3_WIN + r"\objects\weapons\rifle\assault_rifle\assault_rifle.weapon",
    "file_mixed": "file:" + HR_MIXED_ROOT + r"\objects\characters\elite\elite.biped",
    "file_unc": "file:" + H2_UNC + r"\objects\characters\masterchief\masterchief.biped",
    "file_posix": "file:" + H4_POSIX + "/objects/weapons/rifle/storm_rifle/storm_rifle.weapon",
    "file_verbatim": "file:" + r"\\?\C:\Kits\H3ODSTEK\tags\objects\vehicles\warthog\warthog.vehicle",
    "cache_bitm": "cache:bitm:" + r"objects\weapons\rifle\assault_rifle\bitmaps\assault_rifle_diffuse",
    "cache_rm": "cache:" + fmt_group("rm  ") + ":" + r"shaders\default",
    "cache_snd": "cache:snd!:" + r"sound\weapons\assault_rifle\fire",
    "ublock_base": "ublock:pakchunk0-WinGDK:Meteorite/Content/Tags/objects/characters/marine/marine-biped.ubulk",
    "ublock_level": "ublock:pakchunk240-WinGDK:Meteorite/Content/Tags/levels/halo1/solo/a30/_Generated_/a30-scenario.ubulk",
    "ublock_mod": "ublock:mymod_P:Meteorite/Content/Tags/objects/characters/marine/marine_copy-biped.ubulk",
    "newtag_ce": "newtag:/Game/Tags/objects/foo/bar-camera_track",
    "legacy_bare": "objects/weapons/rifle/new_rifle/new_rifle.weapon",  # pre-34c18f5 New Tag / Blam Import
}

def write_json(rel, value, *, sort=True):
    path = os.path.join(OUT, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", newline="\n") as f:
        json.dump(value, f, indent=2, sort_keys=sort, ensure_ascii=False)
    return path

# =========================================================================
# prefs.json
# =========================================================================
def profile(i, name, game, root, **extra):
    v = {"id": f"bab00000-0000-4000-8000-{i:012x}" if i is not None else "3f6f1c4e-8d0e-4e7b-9a52-2f8f4f9b6c11",
         "read_only": False, "git_tracked": False, "name": name, "game": game,
         "root": root, "icon": None}
    v.update(extra)
    return v

prefs_current = {
    "browser_mode": "folders", "browser_sort": "natural", "nested_default": "schema",
    "show_browser_prefixes": False, "folders_before_tags": False, "double_click_to_open_tags": False,
    "session_restore": "ask", "update_channel": "stable", "check_updates_on_startup": True,
    "show_block_sizes": False, "angles_in_degrees": True, "scroll_to_cycle_dropdowns": True,
    "confirm_container_overwrite": True, "confirm_runtime_poke": True, "enable_chimp": True,
    "chimp_output_dir": r"D:\ChimpOut", "chimp_usmap_path": r"D:\Mappings\CE_5.5.4.usmap",
    "expert_mode": True, "dark_mode": True, "ui_scale": 1.0, "scroll_speed": 1.0, "zoom_speed": 1.0,
    "model_preview_size": 320.0, "model_preview_perspective": True,
    "bitmap_preview_background": "dark_gray", "bitmap_preview_checkerboard": True, "bitmap_preview_border": True,
    "blender_path": r"C:\Program Files\Blender Foundation\Blender 4.2\blender.exe",
    "ek_folder_aliases": [
        {"folder_name": "h2rek", "game": "halo2_mcc"},
        {"folder_name": "MyReachKit", "game": "haloreach_mcc"},
    ],
    # Standard kits get stable ids bab00000-0000-4000-8000-00000000000N, N = EDITING_KIT_SHORTCUTS index.
    "editing_kit_profiles": [
        profile(0, "HCEEK", "haloce_mcc", r"C:\Program Files (x86)\Steam\steamapps\common\HCEEK"),
        profile(1, "H2EK", "halo2_mcc", r"\\fileserver\share\H2EK"),
        profile(2, "H3EK", "halo3_mcc", r"C:\Program Files (x86)\Steam\steamapps\common\H3EK"),
        profile(3, "H3ODSTEK", "halo3odst_mcc", r"C:\Kits\H3ODSTEK"),
        profile(4, "HREK", "haloreach_mcc", "C:/Kits/HREK"),
        profile(5, "H4EK", "halo4_mcc", "/Users/me/Kits/H4EK"),
        profile(6, "H2AMPEK", "halo2amp_mcc", r"C:\Kits\H2AMPEK"),
        profile(7, "Campaign Evolved", "haloce_evolved", CE_INSTALL),
        profile(None, "H2 (moda tags)", "halo2_mcc", r"C:\Kits\H2EK",
                read_only=True, git_tracked=True,
                icon=r"editing kit icons\H2-moda-3f6f1c4e\icon-0123456789ab.png",
                tags_folder=r"C:\Kits\H2EK\tags_moda", data_folder=r"C:\Kits\H2EK\data_moda"),
        # tags_folder on a game whose folders are not choosable: ignored on load, dropped on save.
        profile(None, "Reach ignored folder", "haloreach_mcc", r"C:\Kits\HREK2",
                tags_folder=r"C:\Kits\HREK2\tags_moda") | {"id": "5b0c8a8e-1111-4222-8333-444455556666"},
    ],
    "tool_commands_window_pos": [120.0, 80.0], "tool_commands_window_size": [900.0, 600.0],
    "tool_commands_left_width": 280.0, "tool_commands_collapsed_categories": ["Lightmaps"],
    "recent_folders": [CE_INSTALL, H3_WIN, r"\\fileserver\share\H2EK", "/Users/me/Kits/H4EK"],
    "editing_kit_favorites": [
        {"tags_root": H3_WIN, "tags": [r"objects\weapons\rifle\assault_rifle\assault_rifle.weapon"],
         "folders": [r"objects\weapons"]},
        {"tags_root": H4_POSIX, "tags": ["objects/weapons/rifle/storm_rifle/storm_rifle.weapon"], "folders": []},
    ],
    "custom_color_swatches": ["#FF0000FF", {"rgba": "#336699FF", "name": "Spartan blue"}] + [None] * 62,
    "palette_last_dir": r"C:\Users\me\Palettes",
    "storage_mode": "installed",
    "first_run_complete": True,
    "terminal_open_games": sorted(["halo3_mcc", "haloce_evolved"]),
}
write_json("prefs/prefs.current.json", prefs_current)

# Pre-unified editing kits: editing_kit_paths (game-id keyed) + boolean session flag + 16 legacy swatches.
write_json("prefs/prefs.legacy_editing_kit_paths.json", {
    "browser_mode": "groups", "browser_sort": "type",
    "auto_restore_last_session": True,
    "editing_kit_paths": {g: rf"C:\Kits\{g}" for g in GAMES} | {FUTURE_GAME: r"C:\Kits\H5"},
    "custom_color_swatches": ["#FF0000FF", None, "#33669980", "not-a-color"],
    "terminal_open_games": ["halo3_mcc"],
})
write_json("prefs/prefs.legacy_custom_profiles.json", {
    "custom_editing_kit_profiles": [profile(None, "Old custom", "HALO3_MCC", r"C:\Kits\H3EK")],
    "editing_kit_paths": {"haloreach_mcc": r"C:\Kits\HREK"},
})
# Forward-compat probe: profiles and aliases naming a game this build does not
# support (unknown or empty) are never offered as kits but are written back
# unchanged, every field included (63b257d; before it they were dropped).
write_json("prefs/prefs.unknown_game.json", {
    "editing_kit_profiles": [
        profile(None, "Future kit", FUTURE_GAME, r"C:\Kits\H5EK", some_newer_setting=[1, 2, 3]),
        profile(2, "H3EK", "halo3_mcc", r"C:\Kits\H3EK"),
        profile(None, "No game", "", r"C:\Kits\Unknown") | {"id": "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f"},
    ],
    "ek_folder_aliases": [{"folder_name": "h5ek", "game": FUTURE_GAME}],
    "terminal_open_games": [FUTURE_GAME, "halo3_mcc"],
})
with open(os.path.join(OUT, "prefs/prefs.malformed.json"), "w") as f:
    f.write('{"first_run_complete": true, "browser_mode": ')

# =========================================================================
# last_session.json  v1 .. v6 (+ the v2-with-kits shape 8f30d04 wrote, and v99)
# =========================================================================
def tag(key, label, fourcc, path=None):
    return {"key": key, "label": label, "group_tag": gt(fourcc), "path": path}

v1 = {"version": 1,
      "source": {"kind": "loose_folder", "path": H3_WIN, "game": "halo3_mcc"},
      "tags": [tag(K["file_windows"], "objects/weapons/rifle/assault_rifle/assault_rifle.weapon - weapon", "weap",
                   r"\\?\C:\Program Files (x86)\Steam\steamapps\common\H3EK\tags\objects\weapons\rifle\assault_rifle\assault_rifle.weapon")]}
write_json("last_session/v1.json", v1)

v2_single = {"version": 2,
             "source": {"kind": "iostore_container_set", "path": CE_PAKS, "game": "haloce_evolved",
                        "project_path": r"C:\Users\me\AppData\Roaming\Baboon\campaign_evolved_recovery.baboon"},
             "tags": [tag(K["ublock_base"], "objects/characters/marine/marine.biped - biped", "bipd")]}
write_json("last_session/v2_single_source.json", v2_single)

v2_kits = {"version": 2, "kits": [
    {"source": {"kind": "loose_folder", "path": H3_WIN, "game": "halo3_mcc"},
     "tags": [tag(K["file_windows"], "assault_rifle.weapon - weapon", "weap")]}]}
write_json("last_session/v2_kits_8f30d04.json", v2_kits)

def kit_v3(source, tags, **extra):
    v = {"source": source, "tags": tags}
    v.update(extra)
    return v

v3 = {"version": 3, "kits": [
    kit_v3({"kind": "loose_folder", "path": H3_WIN, "game": "halo3_mcc", "project_path": None},
           [tag(K["file_windows"], "a - weapon", "weap")], browser_mode="groups", browser_sort="name"),
    kit_v3({"kind": "monolithic_cache", "path": H3_MAPS, "game": None, "project_path": None},
           [tag(K["cache_bitm"], "b - bitmap", "bitm"), tag(K["cache_rm"], "shaders/default.shader - render_method", "rm  ")]),
    # recovery path recorded as project_path by builds before a3b7d05; no has_project
    kit_v3({"kind": "iostore_container_set", "path": CE_INSTALL, "game": "haloce_evolved",
            "project_path": r"C:\Users\me\AppData\Roaming\Baboon\campaign_evolved_recovery-a1b2c3d4e5f6.baboon"}, []),
]}
write_json("last_session/v3.json", v3)

v4 = {"version": 4, "kits": [
    kit_v3({"kind": "iostore_container_set", "path": CE_INSTALL, "game": "haloce_evolved",
            "profile_id": "bab00000-0000-4000-8000-000000000007", "project_path": None, "has_project": True},
           [tag(K["ublock_mod"], "marine_copy - biped", "bipd")],
           browser_mode="folders", browser_sort="natural",
           chimp_packages=["/Game/Maps/a30/a30_Persistent", "/Game/Tags/objects/characters/marine/marine-biped"],
           active_chimp_package="/Game/Maps/a30/a30_Persistent", active=True),
]}
write_json("last_session/v4.json", v4)

v5 = {"version": 5, "kits": [
    kit_v3({"kind": "loose_folder", "path": "C:/Kits/HREK", "game": "haloreach_mcc",
            "profile_id": "bab00000-0000-4000-8000-000000000004", "project_path": None, "has_project": False},
           [tag(K["file_mixed"], "elite - biped", "bipd",
                r"\\?\C:\Kits\HREK\tags\objects\characters\elite\elite.biped")],
           browser_mode="folders", browser_sort="natural", chimp_packages=[], active_chimp_package=None,
           bitmap_library=True, active=False),
]}
write_json("last_session/v5.json", v5)

def kit_v6(kind, path, game, profile_id, tags, *, folders=(), project_path=None, has_project=False,
           chimp=(), active_chimp=None, bitmap=False, model=False, active=False, mode="folders", sort="natural"):
    return {"source": {"kind": kind, "path": path, "game": game, "profile_id": profile_id,
                       "project_path": project_path, "has_project": has_project},
            "browser_mode": mode, "browser_sort": sort, "tags": list(tags),
            "folders": [{"path": p, "label": l} for p, l in folders],
            "chimp_packages": list(chimp), "active_chimp_package": active_chimp,
            "bitmap_library": bitmap, "model_library": model, "active": active}

v6 = {"version": 6, "kits": [
    kit_v6("loose_folder", r"C:\Program Files (x86)\Steam\steamapps\common\H3EK", "halo3_mcc",
           "bab00000-0000-4000-8000-000000000002",
           [tag(K["file_windows"], "objects/weapons/rifle/assault_rifle/assault_rifle.weapon - weapon", "weap",
                r"\\?\C:\Program Files (x86)\Steam\steamapps\common\H3EK\tags\objects\weapons\rifle\assault_rifle\assault_rifle.weapon"),
            # Pre-34c18f5 bare key: no prefix, only `path` can recover it.
            tag(K["legacy_bare"], "objects/weapons/rifle/new_rifle/new_rifle.weapon - weapon", "weap",
                r"\\?\C:\Program Files (x86)\Steam\steamapps\common\H3EK\tags\objects\weapons\rifle\new_rifle\new_rifle.weapon")],
           folders=[("objects/weapons", "weapons"), ("levels/solo/010_jungle", "010_jungle")],
           bitmap=True, model=True, active=True, mode="groups", sort="type"),
    kit_v6("loose_folder", "C:/Kits/HREK", "haloreach_mcc", "bab00000-0000-4000-8000-000000000004",
           [tag(K["file_mixed"], "objects/characters/elite/elite.biped - biped", "bipd",
                r"\\?\C:\Kits\HREK\tags\objects\characters\elite\elite.biped")]),
    kit_v6("loose_folder", r"\\fileserver\share\H2EK", "halo2_mcc", "bab00000-0000-4000-8000-000000000001",
           [tag(K["file_unc"], "objects/characters/masterchief/masterchief.biped - biped", "bipd",
                r"\\?\UNC\fileserver\share\H2EK\tags\objects\characters\masterchief\masterchief.biped")]),
    kit_v6("loose_folder", "/Users/me/Kits/H4EK", "halo4_mcc", None,
           [tag(K["file_posix"], "objects/weapons/rifle/storm_rifle/storm_rifle.weapon - weapon", "weap",
                "/Users/me/Kits/H4EK/tags/objects/weapons/rifle/storm_rifle/storm_rifle.weapon")]),
    kit_v6("loose_folder", r"C:\Kits\H3ODSTEK", "halo3odst_mcc", "bab00000-0000-4000-8000-000000000003",
           [tag(K["file_verbatim"], "objects/vehicles/warthog/warthog.vehicle - vehicle", "vehi",
                r"\\?\C:\Kits\H3ODSTEK\tags\objects\vehicles\warthog\warthog.vehicle")]),
    kit_v6("loose_folder", r"C:\Kits\HCEEK", "haloce_mcc", "bab00000-0000-4000-8000-000000000000", []),
    kit_v6("loose_folder", r"C:\Kits\H2AMPEK", "halo2amp_mcc", "bab00000-0000-4000-8000-000000000006", []),
    kit_v6("monolithic_cache", H3_MAPS, None, None,
           [tag(K["cache_bitm"], "objects/weapons/rifle/assault_rifle/bitmaps/assault_rifle_diffuse.bitmap - bitmap", "bitm"),
            tag(K["cache_rm"], "shaders/default.shader - render_method", "rm  "),
            tag(K["cache_snd"], "sound/weapons/assault_rifle/fire.sound - sound", "snd!")]),
    kit_v6("single_file", r"C:\Downloads\masterchief.render_model", None, None, []),
    kit_v6("iostore_container_set", CE_INSTALL, "haloce_evolved", "bab00000-0000-4000-8000-000000000007",
           [tag(K["ublock_base"], "objects/characters/marine/marine.biped - biped", "bipd"),
            tag(K["ublock_level"], "levels/halo1/solo/a30/_generated_/a30.scenario - scenario", "scnr"),
            tag(K["ublock_mod"], "objects/characters/marine/marine_copy.biped - biped", "bipd"),
            tag(K["newtag_ce"], "objects/foo/bar.camera_track - camera_track", "trak")],
           folders=[("objects/characters", "characters")],
           project_path=r"C:\Users\me\Documents\Baboon Projects\Campaign Overhaul.baboon", has_project=True,
           chimp=["/Game/Maps/a30/a30_Persistent"], active_chimp="/Game/Maps/a30/a30_Persistent"),
    # Unknown/future game id in a session: displayed only, restore goes by kind+path.
    kit_v6("loose_folder", r"C:\Kits\H5EK", FUTURE_GAME, "3f6f1c4e-8d0e-4e7b-9a52-000000000099", []),
]}
write_json("last_session/v6.json", v6)
write_json("last_session/v99_unknown_version.json", {"version": 99, "kits": []})

# =========================================================================
# window-state.json (no identifiers; schema_version must equal 1)
# =========================================================================
write_json("window_state/window-state.json", {
    "schema_version": 1, "mode": "maximized",
    "normal": {"inner_position_px": {"x": 100.0, "y": 80.0}, "outer_position_px": {"x": 92.0, "y": 49.0},
               "inner_size_logical": {"width": 1280.0, "height": 800.0},
               "outer_size_logical": {"width": 1296.0, "height": 839.0},
               "native_scale_factor": 1.5, "monitor_name": r"\\.\DISPLAY1",
               "monitor_bounds_px": {"x": 0.0, "y": 0.0, "width": 3840.0, "height": 2160.0}}})

# =========================================================================
# Legacy JSON indexes ({game}_index.json, {game}_reverse_dependencies.json)
# =========================================================================
def entry_item(key, display, fourcc, group_name, rel=None, fp=True):
    v = {"key": key, "display_path": display, "group_tag": gt(fourcc), "group_name": group_name}
    if rel is not None:
        v["rel_path"] = rel
    if fp:
        v.update({"size": 123456, "modified_secs": 1759500000, "modified_nanos": 123000000})
    return v

write_json("legacy_index/halo3_mcc_index.json", {
    "root": H3_WIN,
    "entries": [
        entry_item(K["file_windows"], "objects/weapons/rifle/assault_rifle/assault_rifle.weapon", "weap", "weapon",
                   rel="objects/weapons/rifle/assault_rifle/assault_rifle.weapon"),
        entry_item("file:" + H3_WIN + r"\objects\characters\masterchief\masterchief.model", "objects/characters/masterchief/masterchief.model", "hlmt", None, fp=False),
    ]})
# A legacy JSON item whose key lacks "file:" makes entry_from_index_item return None and the
# whole parse_entry_index return None (the `?` on line ~263) -- the entire index is discarded.
write_json("legacy_index/haloreach_mcc_index.json", {
    "root": HR_MIXED_ROOT,
    "entries": [entry_item(K["file_mixed"], "objects/characters/elite/elite.biped", "bipd", "biped"),
                entry_item(K["legacy_bare"], "objects/weapons/rifle/new_rifle/new_rifle.weapon", "weap", "weapon")]})
write_json("legacy_index/halo3_mcc_reverse_dependencies.json", {
    "version": 1, "root": H3_WIN,
    "tags": [{"key": K["file_windows"], "dependencies": [
        {"group_tag": gt("hlmt"), "rel_path": r"objects\weapons\rifle\assault_rifle\assault_rifle"},
        {"group_tag": gt("jpt!"), "rel_path": r"objects\weapons\rifle\assault_rifle\damage_effects\assault_rifle_bullet"}]},
        {"key": "file:" + H3_WIN + r"\objects\characters\masterchief\masterchief.model", "dependencies": []}]})

# =========================================================================
# indexes.sqlite3
# =========================================================================
INDEX_DDL = """
CREATE TABLE IF NOT EXISTS sources (
    id INTEGER PRIMARY KEY,
    game TEXT NOT NULL,
    root_key TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE(game, root_key)
);
CREATE TABLE IF NOT EXISTS entries (
    source_id INTEGER NOT NULL,
    key TEXT NOT NULL,
    rel_path TEXT NOT NULL,
    display_path TEXT NOT NULL,
    group_tag INTEGER NOT NULL,
    group_name TEXT,
    size INTEGER,
    modified_secs INTEGER,
    modified_nanos INTEGER,
    PRIMARY KEY(source_id, key),
    FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS dependencies (
    source_id INTEGER NOT NULL,
    tag_key TEXT NOT NULL,
    dep_group_tag INTEGER NOT NULL,
    dep_rel_path TEXT NOT NULL,
    PRIMARY KEY(source_id, tag_key, dep_group_tag, dep_rel_path),
    FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS indexed_tags (
    source_id INTEGER NOT NULL,
    tag_key TEXT NOT NULL,
    PRIMARY KEY(source_id, tag_key),
    FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS entries_by_source_rel ON entries(source_id, rel_path);
CREATE INDEX IF NOT EXISTS deps_by_target ON dependencies(source_id, dep_group_tag, dep_rel_path);
CREATE INDEX IF NOT EXISTS deps_by_tag ON dependencies(source_id, tag_key);
"""

def win_root_key(p):  # cache_root_key on Windows: canonicalize (\\?\ prefix) + '\' + lowercase
    p = p.replace("/", "\\")
    if p.startswith("\\\\") and not p.startswith("\\\\?\\"):
        p = "\\\\?\\UNC\\" + p[2:]
    elif not p.startswith("\\\\?\\"):
        p = "\\\\?\\" + p
    return p.lower()

def win_root_key_uncanonical(p):  # cache_root_key on Windows when canonicalize fails
    return p.replace("/", "\\").lower()

def make_db(path, sources):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    if os.path.exists(path):
        os.remove(path)
    c = sqlite3.connect(path)
    c.executescript(INDEX_DDL)
    for sid, (game, root_key, entries, deps, indexed) in enumerate(sources, start=1):
        c.execute("INSERT INTO sources (id, game, root_key, schema_version, updated_at) VALUES (?,?,?,1,1759500000)",
                  (sid, game, root_key))
        for e in entries:
            c.execute("INSERT INTO entries VALUES (?,?,?,?,?,?,?,?,?)", (sid, *e))
        for t in indexed:
            c.execute("INSERT INTO indexed_tags VALUES (?,?)", (sid, t))
        for d in deps:
            c.execute("INSERT INTO dependencies VALUES (?,?,?,?)", (sid, *d))
    c.commit()
    sql = "\n".join(c.iterdump())
    c.close()
    with open(path + ".sql", "w") as f:
        f.write(sql + "\n")

ar_key = K["file_windows"]
make_db(os.path.join(OUT, "index/indexes.sqlite3"), [
    ("halo3_mcc", win_root_key(H3_WIN),
     [(ar_key, "objects/weapons/rifle/assault_rifle/assault_rifle.weapon",
       "objects/weapons/rifle/assault_rifle/assault_rifle.weapon", gt("weap"), "weapon", 123456, 1759500000, 5),
      # row with no file: prefix (pre-34c18f5) -- skipped on load, not fatal
      (K["legacy_bare"], "", "objects/weapons/rifle/new_rifle/new_rifle.weapon", gt("weap"), "weapon", None, None, None)],
     [(ar_key, gt("hlmt"), r"objects\weapons\rifle\assault_rifle\assault_rifle"),
      (ar_key, gt("bitm"), r"objects\weapons\rifle\assault_rifle\bitmaps\assault_rifle_diffuse")],
     [ar_key]),
    # Same physical root spelled with '/': same root_key, so its old-spelling keys are reused.
    # A root that did not exist when it was indexed (an unplugged drive): no
    # canonical form, so the key is the spelling lowered with '\' separators.
    ("haloreach_mcc", win_root_key_uncanonical(HR_MIXED_ROOT),
     [(K["file_mixed"], "objects/characters/elite/elite.biped", "objects/characters/elite/elite.biped",
       gt("bipd"), "biped", 2048, 1759500000, 0)], [], [K["file_mixed"]]),
    ("halo2_mcc", win_root_key(H2_UNC),
     [(K["file_unc"], "objects/characters/masterchief/masterchief.biped", "objects/characters/masterchief/masterchief.biped",
       gt("bipd"), "biped", 4096, 1759500000, 0)], [], []),
    ("halo4_mcc", H4_POSIX,  # non-Windows: canonical path, case preserved
     [(K["file_posix"], "objects/weapons/rifle/storm_rifle/storm_rifle.weapon", "objects/weapons/rifle/storm_rifle/storm_rifle.weapon",
       gt("weap"), "weapon", 1, 1759500000, 0),
      # The same pre-34c18f5 bare key, reachable from a non-Windows host.
      (K["legacy_bare"], "", "objects/weapons/rifle/new_rifle/new_rifle.weapon", gt("weap"), "weapon", None, None, None)],
     [], []),
    ("halo3odst_mcc", win_root_key(r"C:\Kits\H3ODSTEK\tags"), [], [], []),
    ("haloce_mcc", win_root_key(r"C:\Kits\HCEEK\tags"), [], [], []),
    ("halo2amp_mcc", win_root_key(r"C:\Kits\H2AMPEK\tags"), [], [], []),
    (FUTURE_GAME, win_root_key(r"C:\Kits\H5EK\tags"), [], [], []),
])

# =========================================================================
# {game}_keywords.json -- BTreeMap<String, Vec<String>> (entry key -> sorted lowercase keywords)
# =========================================================================
write_json("keywords/halo3_mcc_keywords.json", {K["file_windows"]: ["favorite", "rifle"], K["legacy_bare"]: ["wip"]})
write_json("keywords/haloce_evolved_keywords.json", {K["ublock_base"]: ["ai"], K["ublock_mod"]: ["copy"], K["newtag_ce"]: ["cinematic"]})
write_json("keywords/haloreach_mcc_keywords.json", {K["file_mixed"]: ["elite"]})
# Cut short mid-write. Reads as empty, and the next save moves it aside to
# `<name>.unreadable` byte for byte before starting a new one (41bd542).
with open(os.path.join(OUT, "keywords/halo4_mcc_keywords.corrupt.json"), "w") as f:
    f.write('{"file:/Users/me/Kits/H4EK/tags/a.weapon": ["x"')

# =========================================================================
# campaign_duplicates.json (CreatedTagLedger)
# =========================================================================
UTOC_MOD = CE_PAKS + r"\~mods\mymod_P.utoc"
def ledger_rec(**over):
    r = {"utoc_path": UTOC_MOD, "chunk_label": "mymod_P",
         "package_path": "/Game/Tags/objects/characters/marine/marine_copy-biped",
         "package_id": 0x1234_5678_9abc_def0,
         "uasset_path": "Meteorite/Content/Tags/objects/characters/marine/marine_copy-biped.uasset",
         "ubulk_path": "Meteorite/Content/Tags/objects/characters/marine/marine_copy-biped.ubulk",
         "display_path": "objects/characters/marine/marine_copy.biped", "group_tag": gt("bipd"),
         "source_display": "objects/characters/marine/marine.biped",
         "container_entry_count_before": 4, "origin": "Authored", "created_unix_secs": 1759500000}
    r.update(over)
    return r
write_json("ledger/campaign_duplicates.json", {"version": 1, "tags": [
    ledger_rec(),
    ledger_rec(ubulk_path="Meteorite/Content/Tags/objects/weapons/rifle/ar_renamed-weapon.ubulk",
               uasset_path="Meteorite/Content/Tags/objects/weapons/rifle/ar_renamed-weapon.uasset",
               package_path="/Game/Tags/objects/weapons/rifle/ar_renamed-weapon", display_path="objects/weapons/rifle/ar_renamed.weapon",
               group_tag=gt("weap"), origin="RenamedFromShipped", chunk_label="pakchunk0-WinGDK",
               utoc_path=CE_PAKS + r"\pakchunk0-WinGDK.utoc")]})
old = ledger_rec(); del old["origin"]; del old["container_entry_count_before"]
write_json("ledger/campaign_duplicates.v0_no_version_no_origin.json", {"tags": [old]})
# A newer build's rows: an unknown origin loads as Unrecognized and is kept
# (and refused for delete); a row of a shape this build cannot read is kept
# raw. Both are written back on save (8c6da07; before it the whole file failed
# to parse and the next save erased every record).
write_json("ledger/campaign_duplicates.future_origin.json", {"version": 2, "tags": [
    ledger_rec(origin="ImportedFromMod"),
    ledger_rec(ubulk_path="x.ubulk"),
    {"utoc_path": UTOC_MOD, "ubulk_path": "y.ubulk", "provenance": {"kind": "future"}},
]})
# Not a ledger at all: loads empty, and save refuses so the file is left as is.
with open(os.path.join(OUT, "ledger/campaign_duplicates.truncated.json"), "w") as f:
    f.write('{"version": 1, "tags": [{"utoc_path": ')

# =========================================================================
# .baboon projects (campaign_evolved_recovery-<sha256(root)[..6]>.baboon, user projects, mod sidecars)
# =========================================================================
PROJECT_DDL = """
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS project (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL,
    game TEXT NOT NULL,
    source_path TEXT NOT NULL,
    selected_identity TEXT
);
CREATE TABLE IF NOT EXISTS tabs (
    position INTEGER PRIMARY KEY,
    identity TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    group_tag INTEGER NOT NULL,
    logical_path TEXT NOT NULL,
    kind TEXT NOT NULL,
    package TEXT,
    floating INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS overlays (
    identity TEXT PRIMARY KEY,
    group_tag INTEGER NOT NULL,
    logical_path TEXT NOT NULL,
    kind TEXT NOT NULL,
    package TEXT,
    bytes BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS history (
    identity TEXT NOT NULL,
    stack TEXT NOT NULL,
    position INTEGER NOT NULL,
    label TEXT NOT NULL,
    bytes BLOB NOT NULL,
    PRIMARY KEY (identity, stack, position)
);
CREATE TABLE IF NOT EXISTS history_steps (
    identity TEXT NOT NULL,
    step INTEGER NOT NULL,
    stack TEXT NOT NULL,
    position INTEGER NOT NULL,
    label TEXT NOT NULL,
    bytes BLOB NOT NULL,
    PRIMARY KEY (identity, step)
);
CREATE TABLE IF NOT EXISTS folders (
    path TEXT PRIMARY KEY
);
"""
# Pre-history/pre-folders project (version 1 too -- version was never bumped).
PROJECT_DDL_V1_ORIGINAL = PROJECT_DDL.split("CREATE TABLE IF NOT EXISTS history (")[0]

def ident(fourcc, logical):
    return f"{gt(fourcc):08x}:{logical}"

MARINE = ident("bipd", "objects/characters/marine/marine")
COPY = ident("bipd", "objects/characters/marine/marine_copy")
NEW = ident("trak", "objects/foo/bar")
A30 = ident("scnr", "levels/halo1/solo/a30/_generated_/a30")

def make_project(path, ddl, *, version=1, game="haloce_evolved", source_path=CE_PAKS, history=True,
                 legacy_history=False, folders=True, kind_override=None):
    if os.path.exists(path):
        os.remove(path)
    c = sqlite3.connect(path)
    c.executescript(ddl)
    c.execute("INSERT INTO project VALUES (1,?,?,?,?)", (version, game, source_path, MARINE))
    tabs = [(0, MARINE, "objects/characters/marine/marine.biped", gt("bipd"), "objects/characters/marine/marine", "existing", None, 0),
            (1, COPY, "objects/characters/marine/marine_copy.biped", gt("bipd"), "objects/characters/marine/marine_copy", "new",
             "/Game/Tags/objects/characters/marine/marine_copy-biped", 0),
            (2, NEW, "objects/foo/bar.camera_track", gt("trak"), "objects/foo/bar", kind_override or "new", "/Game/Tags/objects/foo/bar-camera_track", 0),
            (3, A30, "levels/halo1/solo/a30/_generated_/a30.scenario", gt("scnr"), "levels/halo1/solo/a30/_generated_/a30", "existing", None, 0)]
    c.executemany("INSERT INTO tabs VALUES (?,?,?,?,?,?,?,?)", tabs)
    blob = b"BLAM-TAG-BYTES-PLACEHOLDER (not a parseable tag)"
    c.executemany("INSERT INTO overlays VALUES (?,?,?,?,?,?)", [
        (MARINE, gt("bipd"), "objects/characters/marine/marine", "existing", None, blob),
        (NEW, gt("trak"), "objects/foo/bar", "new", "/Game/Tags/objects/foo/bar-camera_track", blob)])
    if history:
        c.executemany("INSERT INTO history_steps VALUES (?,?,?,?,?,?)", [
            (MARINE, 41, "undo", 0, "Edit health", blob), (MARINE, 42, "undo", 1, "Edit shield", blob),
            (MARINE, 43, "redo", 0, "Edit speed", blob), (MARINE, 44, "sideways", 0, "unknown stack", blob)])
    if legacy_history:
        c.executemany("INSERT INTO history VALUES (?,?,?,?,?)", [
            (MARINE, "undo", 0, "Edit health", blob), (MARINE, "redo", 0, "Edit speed", blob)])
    if folders:
        c.executemany("INSERT INTO folders VALUES (?)", [("objects/my_new_folder",), ("levels/custom",)])
    c.commit()
    sql = "\n".join(c.iterdump())
    c.close()
    with open(path + ".sql", "w") as f:
        f.write(sql + "\n")

recovery_tag = hashlib.sha256(CE_PAKS.encode()).hexdigest()[:12]
os.makedirs(os.path.join(OUT, "project"), exist_ok=True)
make_project(os.path.join(OUT, f"project/campaign_evolved_recovery-{recovery_tag}.baboon"), PROJECT_DDL)
make_project(os.path.join(OUT, "project/user_project.history_table_only.baboon"), PROJECT_DDL, history=False, legacy_history=True, folders=False)
make_project(os.path.join(OUT, "project/user_project.original_v1_schema.baboon"), PROJECT_DDL_V1_ORIGINAL, history=False, folders=False)
make_project(os.path.join(OUT, "project/rejected.version2.baboon"), PROJECT_DDL, version=2)
make_project(os.path.join(OUT, "project/rejected.game_halo3_mcc.baboon"), PROJECT_DDL, game="halo3_mcc")
make_project(os.path.join(OUT, "project/rejected.unknown_kind.baboon"), PROJECT_DDL, kind_override="renamed")

# Container tags with a dot in their name (bb6315b). The identity is the group
# and the display path without its extension; before bb6315b the display path
# was cut at the last dot, so `levels/v1.2/bitmaps/rock` was filed as
# `levels/v1`. A file holds either; the old one still resolves when unique.
DOTTED = ident("bitm", "levels/v1.2/bitmaps/rock")
DOTTED_LEGACY = ident("bitm", "levels/v1")
path = os.path.join(OUT, "project/user_project.dotted_identities.baboon")
if os.path.exists(path):
    os.remove(path)
c = sqlite3.connect(path)
c.executescript(PROJECT_DDL)
c.execute("INSERT INTO project VALUES (1,1,'haloce_evolved',?,?)", (CE_PAKS, DOTTED))
c.executemany("INSERT INTO tabs VALUES (?,?,?,?,?,?,?,?)", [
    (0, DOTTED, "levels/v1.2/bitmaps/rock.bitmap", gt("bitm"), "levels/v1.2/bitmaps/rock", "existing", None, 0),
    (1, DOTTED_LEGACY, "levels/v1.bitmap", gt("bitm"), "levels/v1", "existing", None, 0),
    (2, ident("snd!", "sound/machines/piston_close2.l"), "sound/machines/piston_close2.l.sound", gt("snd!"),
     "sound/machines/piston_close2.l", "existing", None, 0)])
c.commit()
sql = "\n".join(c.iterdump())
c.close()
with open(path + ".sql", "w") as f:
    f.write(sql + "\n")

# =========================================================================
# chimp-recovery-<sha256(root)[..12]>/manifest.json + payloads
# =========================================================================
chimp_dir = os.path.join(OUT, f"chimp/chimp-recovery-{hashlib.sha256(CE_PAKS.encode()).hexdigest()[:24]}")
os.makedirs(chimp_dir, exist_ok=True)
pk = "/Game/Maps/a30/a30_Persistent"
fn = hashlib.sha256(pk.encode()).hexdigest() + ".uasset"
with open(os.path.join(chimp_dir, fn), "wb") as f:
    f.write(b"\xc1\x83\x2a\x9e placeholder uasset bytes")
write_json(os.path.relpath(os.path.join(chimp_dir, "manifest.json"), OUT), {"source": CE_PAKS, "packages": {pk: fn}})

# =========================================================================
# Container backup manifest (write-only; never deserialized)
# =========================================================================
with open(os.path.join(OUT, "backups_pakchunk7-WinGDK.utoc.baboon-duplicate-backup.manifest.json"), "w") as f:
    f.write(json.dumps({"version": 1, "original_utoc_filename": "pakchunk7-WinGDK.utoc", "original_ucas_length": 37},
                       separators=(",", ":")))

# Every key kind in one list, for a TagKey round-trip table test.
write_json("tag_keys.json", K, sort=False)
print("recovery file tag:", recovery_tag)
print("wrote", OUT)
