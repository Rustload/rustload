-- Rustload initial schema. Timestamps are unix seconds, sizes are bytes.

CREATE TABLE users (
    id            INTEGER PRIMARY KEY,
    username      TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT    NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
    token_hash TEXT    PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

-- Small key/value store: settings document, global pause flag...
CREATE TABLE kv (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE categories (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    path       TEXT    NOT NULL,
    extensions TEXT    NOT NULL DEFAULT '',  -- comma separated, lowercase, no dot
    is_default INTEGER NOT NULL DEFAULT 0,
    position   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE accounts (
    id            INTEGER PRIMARY KEY,
    provider      TEXT    NOT NULL,
    label         TEXT    NOT NULL,
    credentials   TEXT    NOT NULL,          -- JSON, provider specific
    enabled       INTEGER NOT NULL DEFAULT 1,
    position      INTEGER NOT NULL DEFAULT 0,
    username      TEXT,
    premium       INTEGER NOT NULL DEFAULT 0,
    premium_until INTEGER,
    traffic_left  INTEGER,
    last_check    INTEGER,
    last_error    TEXT,
    created_at    INTEGER NOT NULL
);

CREATE TABLE packages (
    id                INTEGER PRIMARY KEY,
    name              TEXT    NOT NULL,
    category_id       INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    dest_dir          TEXT,
    create_subfolder  INTEGER NOT NULL,
    extract           INTEGER NOT NULL,
    delete_archives   INTEGER NOT NULL,
    archive_password  TEXT,
    source_kind       TEXT    NOT NULL DEFAULT 'links',    -- links | magnet | torrent
    source            TEXT,                                -- magnet URI or torrent file name
    -- 'remote' (debrid processes a magnet), 'download', 'extract', 'done'
    stage             TEXT    NOT NULL DEFAULT 'download',
    remote_account_id INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    remote_id         TEXT,
    remote_state      TEXT,                                -- JSON DebridProgress
    name_locked       INTEGER NOT NULL DEFAULT 0,          -- user chose the name explicitly
    paused            INTEGER NOT NULL DEFAULT 0,
    error             TEXT,
    position          INTEGER NOT NULL,
    created_at        INTEGER NOT NULL,
    finished_at       INTEGER
);

CREATE INDEX packages_finished_at ON packages(finished_at);

CREATE TABLE downloads (
    id                INTEGER PRIMARY KEY,
    package_id        INTEGER NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    url               TEXT    NOT NULL,
    rel_dir           TEXT    NOT NULL DEFAULT '',         -- sub-folder inside the package folder
    filename          TEXT,
    size              INTEGER,
    downloaded        INTEGER NOT NULL DEFAULT 0,
    status            TEXT    NOT NULL DEFAULT 'queued',
    error             TEXT,
    via               TEXT,
    host              TEXT,
    account_id        INTEGER REFERENCES accounts(id) ON DELETE SET NULL, -- must be unlocked by this account
    path              TEXT,                                -- final path once completed
    temp_path         TEXT,
    resume            TEXT,                                -- JSON resume state
    checked           INTEGER NOT NULL DEFAULT 0,          -- availability check done
    attempts          INTEGER NOT NULL DEFAULT 0,
    retry_at          INTEGER,
    position          INTEGER NOT NULL,
    created_at        INTEGER NOT NULL,
    started_at        INTEGER,
    finished_at       INTEGER
);

CREATE INDEX downloads_package ON downloads(package_id);
CREATE INDEX downloads_status ON downloads(status);
