BEGIN TRANSACTION;
CREATE TABLE dependencies (
    source_id INTEGER NOT NULL,
    tag_key TEXT NOT NULL,
    dep_group_tag INTEGER NOT NULL,
    dep_rel_path TEXT NOT NULL,
    PRIMARY KEY(source_id, tag_key, dep_group_tag, dep_rel_path),
    FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
);
INSERT INTO "dependencies" VALUES(1,'file:C:\Program Files (x86)\Steam\steamapps\common\H3EK\tags\objects\weapons\rifle\assault_rifle\assault_rifle.weapon',1751936372,'objects\weapons\rifle\assault_rifle\assault_rifle');
INSERT INTO "dependencies" VALUES(1,'file:C:\Program Files (x86)\Steam\steamapps\common\H3EK\tags\objects\weapons\rifle\assault_rifle\assault_rifle.weapon',1651078253,'objects\weapons\rifle\assault_rifle\bitmaps\assault_rifle_diffuse');
CREATE TABLE entries (
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
INSERT INTO "entries" VALUES(1,'file:C:\Program Files (x86)\Steam\steamapps\common\H3EK\tags\objects\weapons\rifle\assault_rifle\assault_rifle.weapon','objects/weapons/rifle/assault_rifle/assault_rifle.weapon','objects/weapons/rifle/assault_rifle/assault_rifle.weapon',2003132784,'weapon',123456,1759500000,5);
INSERT INTO "entries" VALUES(1,'objects/weapons/rifle/new_rifle/new_rifle.weapon','','objects/weapons/rifle/new_rifle/new_rifle.weapon',2003132784,'weapon',NULL,NULL,NULL);
INSERT INTO "entries" VALUES(2,'file:C:/Kits/HREK/tags\objects\characters\elite\elite.biped','objects/characters/elite/elite.biped','objects/characters/elite/elite.biped',1651077220,'biped',2048,1759500000,0);
INSERT INTO "entries" VALUES(3,'file:\\fileserver\share\H2EK\tags\objects\characters\masterchief\masterchief.biped','objects/characters/masterchief/masterchief.biped','objects/characters/masterchief/masterchief.biped',1651077220,'biped',4096,1759500000,0);
INSERT INTO "entries" VALUES(4,'file:/Users/me/Kits/H4EK/tags/objects/weapons/rifle/storm_rifle/storm_rifle.weapon','objects/weapons/rifle/storm_rifle/storm_rifle.weapon','objects/weapons/rifle/storm_rifle/storm_rifle.weapon',2003132784,'weapon',1,1759500000,0);
INSERT INTO "entries" VALUES(4,'objects/weapons/rifle/new_rifle/new_rifle.weapon','','objects/weapons/rifle/new_rifle/new_rifle.weapon',2003132784,'weapon',NULL,NULL,NULL);
CREATE TABLE indexed_tags (
    source_id INTEGER NOT NULL,
    tag_key TEXT NOT NULL,
    PRIMARY KEY(source_id, tag_key),
    FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
);
INSERT INTO "indexed_tags" VALUES(1,'file:C:\Program Files (x86)\Steam\steamapps\common\H3EK\tags\objects\weapons\rifle\assault_rifle\assault_rifle.weapon');
INSERT INTO "indexed_tags" VALUES(2,'file:C:/Kits/HREK/tags\objects\characters\elite\elite.biped');
CREATE TABLE sources (
    id INTEGER PRIMARY KEY,
    game TEXT NOT NULL,
    root_key TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE(game, root_key)
);
INSERT INTO "sources" VALUES(1,'halo3_mcc','\\?\c:\program files (x86)\steam\steamapps\common\h3ek\tags',1,1759500000);
INSERT INTO "sources" VALUES(2,'haloreach_mcc','c:\kits\hrek\tags',1,1759500000);
INSERT INTO "sources" VALUES(3,'halo2_mcc','\\?\unc\fileserver\share\h2ek\tags',1,1759500000);
INSERT INTO "sources" VALUES(4,'halo4_mcc','/Users/me/Kits/H4EK/tags',1,1759500000);
INSERT INTO "sources" VALUES(5,'halo3odst_mcc','\\?\c:\kits\h3odstek\tags',1,1759500000);
INSERT INTO "sources" VALUES(6,'haloce_mcc','\\?\c:\kits\hceek\tags',1,1759500000);
INSERT INTO "sources" VALUES(7,'halo2amp_mcc','\\?\c:\kits\h2ampek\tags',1,1759500000);
INSERT INTO "sources" VALUES(8,'halo5_mcc','\\?\c:\kits\h5ek\tags',1,1759500000);
CREATE INDEX entries_by_source_rel ON entries(source_id, rel_path);
CREATE INDEX deps_by_target ON dependencies(source_id, dep_group_tag, dep_rel_path);
CREATE INDEX deps_by_tag ON dependencies(source_id, tag_key);
COMMIT;
