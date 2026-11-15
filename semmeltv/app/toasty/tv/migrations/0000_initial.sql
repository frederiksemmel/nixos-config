CREATE TABLE "settings" (
    "name" TEXT NOT NULL,
    "value" TEXT NOT NULL,
    PRIMARY KEY ("name")
);
-- #[toasty::breakpoint]
CREATE TABLE "audio_prefs" (
    "profile_id" BIGINT NOT NULL,
    "output" TEXT NOT NULL CHECK ("output" IN ('tv', 'headphones')),
    "headphones_address" TEXT,
    PRIMARY KEY ("profile_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "passkeys" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "profile_id" BIGINT NOT NULL,
    "credential_id" BLOB NOT NULL,
    "rp_id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "passkey" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "last_used_at" BIGINT
);
-- #[toasty::breakpoint]
CREATE INDEX "index_passkeys_by_profile_id" ON "passkeys" ("profile_id");
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_passkeys_by_credential_id" ON "passkeys" ("credential_id");
-- #[toasty::breakpoint]
CREATE TABLE "invites" (
    "token_hash" BLOB NOT NULL,
    "profile_id" BIGINT NOT NULL,
    "created_by" BIGINT,
    "created_at" BIGINT NOT NULL,
    "expires_at" BIGINT NOT NULL,
    "used_at" BIGINT,
    PRIMARY KEY ("token_hash")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_invites_by_profile_id" ON "invites" ("profile_id");
-- #[toasty::breakpoint]
CREATE INDEX "index_invites_by_created_by" ON "invites" ("created_by");
-- #[toasty::breakpoint]
CREATE TABLE "started" (
    "profile_id" BIGINT NOT NULL,
    "kind" TEXT NOT NULL CHECK ("kind" IN ('series', 'movie')),
    "title_id" BIGINT NOT NULL,
    "last_played" BIGINT NOT NULL,
    PRIMARY KEY ("profile_id", "kind", "title_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "video_facts" (
    "kind" TEXT NOT NULL CHECK ("kind" IN ('episode', 'movie')),
    "video_id" BIGINT NOT NULL,
    "duration" REAL,
    "credits" REAL,
    "subtitle_delay" REAL NOT NULL,
    "embedded_subtitles" BIGINT,
    PRIMARY KEY ("kind", "video_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "subtitle_settings" (
    "profile_id" BIGINT NOT NULL,
    "settings" TEXT NOT NULL,
    PRIMARY KEY ("profile_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "video_places" (
    "profile_id" BIGINT NOT NULL,
    "kind" TEXT NOT NULL CHECK ("kind" IN ('episode', 'movie')),
    "video_id" BIGINT NOT NULL,
    "viewing" TEXT NOT NULL CHECK ("viewing" IN ('not_started', 'in_progress', 'finished')),
    "viewing_at" REAL,
    "viewing_audio_track" BIGINT,
    "viewing_subtitle_track" BIGINT,
    "viewing_subtitles_shown" BOOLEAN,
    "last_played" BIGINT,
    PRIMARY KEY ("profile_id", "kind", "video_id"),
    CHECK (("viewing" = 'in_progress') = ("viewing_at" IS NOT NULL)),
    CHECK ("viewing" = 'in_progress' OR ("viewing_audio_track" IS NULL AND "viewing_subtitle_track" IS NULL AND "viewing_subtitles_shown" IS NULL))
);
-- #[toasty::breakpoint]
CREATE TABLE "sessions" (
    "token_hash" BLOB NOT NULL,
    "profile_id" BIGINT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "last_seen_at" BIGINT NOT NULL,
    "expires_at" BIGINT NOT NULL,
    "label" TEXT NOT NULL,
    PRIMARY KEY ("token_hash")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_sessions_by_profile_id" ON "sessions" ("profile_id");
-- #[toasty::breakpoint]
CREATE TABLE "profiles" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "name" TEXT NOT NULL,
    "role" TEXT NOT NULL CHECK ("role" IN ('admin', 'viewer')),
    "password_hash" TEXT,
    "color" TEXT,
    "created_at" BIGINT NOT NULL
);
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_profiles_by_name" ON "profiles" ("name");
-- #[toasty::breakpoint]
CREATE TABLE "series_places" (
    "profile_id" BIGINT NOT NULL,
    "show_id" BIGINT NOT NULL,
    "season" BIGINT NOT NULL,
    "episode" BIGINT NOT NULL,
    "finished" BOOLEAN NOT NULL,
    PRIMARY KEY ("profile_id", "show_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "dismissed_videos" (
    "profile_id" BIGINT NOT NULL,
    "kind" TEXT NOT NULL CHECK ("kind" IN ('episode', 'movie')),
    "video_id" BIGINT NOT NULL,
    "at" BIGINT NOT NULL,
    PRIMARY KEY ("profile_id", "kind", "video_id")
);
-- #[toasty::breakpoint]
INSERT INTO "profiles" ("id", "name", "role", "created_at") VALUES
    (1, 'Frederik', 'admin', unixepoch()),
    (2, 'Mami', 'viewer', unixepoch()),
    (3, 'Miriam', 'viewer', unixepoch()),
    (4, 'Ramu', 'viewer', unixepoch());
