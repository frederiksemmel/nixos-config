CREATE TABLE profiles (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL CHECK (role IN ('admin', 'viewer')),
    password_hash TEXT,
    color TEXT,
    created_at INTEGER NOT NULL
);

INSERT INTO profiles (id, name, role, created_at) VALUES
    (1, 'Frederik', 'admin', unixepoch()),
    (2, 'Mami', 'viewer', unixepoch()),
    (3, 'Miriam', 'viewer', unixepoch()),
    (4, 'Ramu', 'viewer', unixepoch());

CREATE TABLE passkeys (
    id INTEGER PRIMARY KEY,
    profile_id INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    credential_id BLOB NOT NULL UNIQUE,
    rp_id TEXT NOT NULL,
    name TEXT NOT NULL,
    passkey TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE INDEX passkeys_profile ON passkeys (profile_id);

CREATE TABLE sessions (
    token_hash BLOB PRIMARY KEY,
    profile_id INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    label TEXT
);

CREATE INDEX sessions_profile ON sessions (profile_id);

CREATE TABLE invites (
    token_hash BLOB PRIMARY KEY,
    profile_id INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    created_by INTEGER REFERENCES profiles (id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at INTEGER
);

CREATE TABLE files (
    path TEXT PRIMARY KEY,
    duration REAL,
    credits REAL,
    subtitle_delay REAL NOT NULL DEFAULT 0,
    embedded_subtitles INTEGER
);

CREATE TABLE file_places (
    profile_id INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    stopped_at REAL,
    last_played INTEGER,
    finished INTEGER NOT NULL DEFAULT 0 CHECK (finished IN (0, 1)),
    audio_track INTEGER,
    subtitle_track INTEGER,
    subtitles_shown INTEGER CHECK (subtitles_shown IN (0, 1)),
    PRIMARY KEY (profile_id, path)
) WITHOUT ROWID;

CREATE TABLE show_places (
    profile_id INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    show_key TEXT NOT NULL,
    season INTEGER NOT NULL,
    episode INTEGER NOT NULL,
    finished INTEGER NOT NULL CHECK (finished IN (0, 1)),
    PRIMARY KEY (profile_id, show_key)
) WITHOUT ROWID;

CREATE TABLE watching (
    profile_id INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('series', 'movie')),
    key TEXT NOT NULL,
    last_played INTEGER NOT NULL,
    PRIMARY KEY (profile_id, kind, key)
) WITHOUT ROWID;

CREATE TABLE dismissed (
    profile_id INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (profile_id, path)
) WITHOUT ROWID;

CREATE TABLE subtitle_settings (
    profile_id INTEGER PRIMARY KEY REFERENCES profiles (id) ON DELETE CASCADE,
    settings TEXT NOT NULL
);

CREATE TABLE audio_prefs (
    profile_id INTEGER PRIMARY KEY REFERENCES profiles (id) ON DELETE CASCADE,
    output TEXT NOT NULL CHECK (output IN ('tv', 'headphones')),
    headphones_address TEXT
);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
