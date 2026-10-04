BEGIN TRANSACTION;
CREATE TABLE folders (
    path TEXT PRIMARY KEY
);
INSERT INTO "folders" VALUES('objects/my_new_folder');
INSERT INTO "folders" VALUES('levels/custom');
CREATE TABLE history (
    identity TEXT NOT NULL,
    stack TEXT NOT NULL,
    position INTEGER NOT NULL,
    label TEXT NOT NULL,
    bytes BLOB NOT NULL,
    PRIMARY KEY (identity, stack, position)
);
CREATE TABLE history_steps (
    identity TEXT NOT NULL,
    step INTEGER NOT NULL,
    stack TEXT NOT NULL,
    position INTEGER NOT NULL,
    label TEXT NOT NULL,
    bytes BLOB NOT NULL,
    PRIMARY KEY (identity, step)
);
INSERT INTO "history_steps" VALUES('62697064:objects/characters/marine/marine',41,'undo',0,'Edit health',X'424C414D2D5441472D42595445532D504C414345484F4C44455220286E6F74206120706172736561626C652074616729');
INSERT INTO "history_steps" VALUES('62697064:objects/characters/marine/marine',42,'undo',1,'Edit shield',X'424C414D2D5441472D42595445532D504C414345484F4C44455220286E6F74206120706172736561626C652074616729');
INSERT INTO "history_steps" VALUES('62697064:objects/characters/marine/marine',43,'redo',0,'Edit speed',X'424C414D2D5441472D42595445532D504C414345484F4C44455220286E6F74206120706172736561626C652074616729');
INSERT INTO "history_steps" VALUES('62697064:objects/characters/marine/marine',44,'sideways',0,'unknown stack',X'424C414D2D5441472D42595445532D504C414345484F4C44455220286E6F74206120706172736561626C652074616729');
CREATE TABLE overlays (
    identity TEXT PRIMARY KEY,
    group_tag INTEGER NOT NULL,
    logical_path TEXT NOT NULL,
    kind TEXT NOT NULL,
    package TEXT,
    bytes BLOB NOT NULL
);
INSERT INTO "overlays" VALUES('62697064:objects/characters/marine/marine',1651077220,'objects/characters/marine/marine','existing',NULL,X'424C414D2D5441472D42595445532D504C414345484F4C44455220286E6F74206120706172736561626C652074616729');
INSERT INTO "overlays" VALUES('7472616b:objects/foo/bar',1953653099,'objects/foo/bar','new','/Game/Tags/objects/foo/bar-camera_track',X'424C414D2D5441472D42595445532D504C414345484F4C44455220286E6F74206120706172736561626C652074616729');
CREATE TABLE project (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL,
    game TEXT NOT NULL,
    source_path TEXT NOT NULL,
    selected_identity TEXT
);
INSERT INTO "project" VALUES(1,1,'haloce_evolved','D:\XboxGames\Halo Campaign Evolved\Content\Meteorite\Content\Paks','62697064:objects/characters/marine/marine');
CREATE TABLE tabs (
    position INTEGER PRIMARY KEY,
    identity TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    group_tag INTEGER NOT NULL,
    logical_path TEXT NOT NULL,
    kind TEXT NOT NULL,
    package TEXT,
    floating INTEGER NOT NULL
);
INSERT INTO "tabs" VALUES(0,'62697064:objects/characters/marine/marine','objects/characters/marine/marine.biped',1651077220,'objects/characters/marine/marine','existing',NULL,0);
INSERT INTO "tabs" VALUES(1,'62697064:objects/characters/marine/marine_copy','objects/characters/marine/marine_copy.biped',1651077220,'objects/characters/marine/marine_copy','new','/Game/Tags/objects/characters/marine/marine_copy-biped',0);
INSERT INTO "tabs" VALUES(2,'7472616b:objects/foo/bar','objects/foo/bar.camera_track',1953653099,'objects/foo/bar','new','/Game/Tags/objects/foo/bar-camera_track',0);
INSERT INTO "tabs" VALUES(3,'73636e72:levels/halo1/solo/a30/_generated_/a30','levels/halo1/solo/a30/_generated_/a30.scenario',1935896178,'levels/halo1/solo/a30/_generated_/a30','existing',NULL,0);
COMMIT;
