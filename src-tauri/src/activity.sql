CREATE TABLE IF NOT EXISTS activity (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    stamp INTEGER NOT NULL,
    digest TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS activity_by_time ON activity(stamp DESC);
CREATE INDEX IF NOT EXISTS activity_by_source ON activity(source, stamp DESC);
CREATE TABLE IF NOT EXISTS activity_previews (
    event TEXT PRIMARY KEY REFERENCES activity(id) ON DELETE CASCADE,
    jpeg TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS activity_habits (
    id TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    corrected INTEGER NOT NULL DEFAULT 0,
    updated INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS activity_support (
    habit TEXT REFERENCES activity_habits(id) ON DELETE CASCADE,
    event TEXT REFERENCES activity(id) ON DELETE CASCADE,
    PRIMARY KEY(habit, event)
);
CREATE VIRTUAL TABLE IF NOT EXISTS activity_search USING fts5(
    title, body, source, content='activity', content_rowid='rowid',
    tokenize='porter unicode61'
);
CREATE VIRTUAL TABLE IF NOT EXISTS activity_vectors USING vec0(
    event_id TEXT PRIMARY KEY, space TEXT PARTITION KEY, feature float[192]
);
CREATE TRIGGER IF NOT EXISTS activity_unembed AFTER DELETE ON activity BEGIN
    DELETE FROM activity_vectors WHERE event_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS activity_index AFTER INSERT ON activity BEGIN
    INSERT INTO activity_search(rowid,title,body,source)
    VALUES(NEW.rowid,NEW.title,NEW.body,NEW.source);
END;
CREATE TRIGGER IF NOT EXISTS activity_unindex AFTER DELETE ON activity BEGIN
    INSERT INTO activity_search(activity_search,rowid,title,body,source)
    VALUES('delete',OLD.rowid,OLD.title,OLD.body,OLD.source);
END;
CREATE TRIGGER IF NOT EXISTS activity_remove_learning BEFORE DELETE ON activity BEGIN
    DELETE FROM activity_habits WHERE id IN (
        SELECT habit FROM activity_support WHERE event=OLD.id
    );
END;
CREATE TABLE IF NOT EXISTS activity_dirty (day TEXT PRIMARY KEY);
CREATE TRIGGER IF NOT EXISTS activity_dirty_insert AFTER INSERT ON activity BEGIN
    INSERT OR IGNORE INTO activity_dirty VALUES(date(NEW.stamp,'unixepoch'));
END;
CREATE TRIGGER IF NOT EXISTS activity_dirty_delete AFTER DELETE ON activity BEGIN
    INSERT OR IGNORE INTO activity_dirty VALUES(date(OLD.stamp,'unixepoch'));
END;

CREATE TRIGGER IF NOT EXISTS activity_forget_chats AFTER DELETE ON activity BEGIN
    DELETE FROM docs WHERE kind='floatingChat' AND EXISTS (
        SELECT 1 FROM json_each(docs.v,'$.turns') AS turn, json_each(turn.value,'$.sources') AS source
        WHERE source.type='object' AND json_extract(source.value,'$.id')=OLD.id
    );
END;

CREATE VIRTUAL TABLE IF NOT EXISTS imported_passages USING fts5(
    id UNINDEXED, title, body, kind UNINDEXED, tokenize='porter unicode61'
);

CREATE TABLE IF NOT EXISTS prepared_baggage (
    booking TEXT PRIMARY KEY REFERENCES activity(id) ON DELETE CASCADE,
    note TEXT
);

CREATE TABLE IF NOT EXISTS proactive_dismissals (
    digest TEXT PRIMARY KEY,
    expires INTEGER NOT NULL
);
