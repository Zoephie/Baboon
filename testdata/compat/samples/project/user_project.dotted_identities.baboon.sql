BEGIN TRANSACTION;
CREATE TABLE folders (
    path TEXT PRIMARY KEY
);
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
CREATE TABLE overlays (
    identity TEXT PRIMARY KEY,
    group_tag INTEGER NOT NULL,
    logical_path TEXT NOT NULL,
    kind TEXT NOT NULL,
    package TEXT,
    bytes BLOB NOT NULL
);
CREATE TABLE project (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL,
    game TEXT NOT NULL,
    source_path TEXT NOT NULL,
    selected_identity TEXT
);
INSERT INTO "project" VALUES(1,1,'haloce_evolved','D:\XboxGames\Halo Campaign Evolved\Content\Meteorite\Content\Paks','6269746d:levels/v1.2/bitmaps/rock');
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
INSERT INTO "tabs" VALUES(0,'6269746d:levels/v1.2/bitmaps/rock','levels/v1.2/bitmaps/rock.bitmap',1651078253,'levels/v1.2/bitmaps/rock','existing',NULL,0);
INSERT INTO "tabs" VALUES(1,'6269746d:levels/v1','levels/v1.bitmap',1651078253,'levels/v1','existing',NULL,0);
INSERT INTO "tabs" VALUES(2,'736e6421:sound/machines/piston_close2.l','sound/machines/piston_close2.l.sound',1936614433,'sound/machines/piston_close2.l','existing',NULL,0);
COMMIT;
