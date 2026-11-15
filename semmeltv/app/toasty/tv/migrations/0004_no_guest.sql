PRAGMA foreign_keys = OFF;
-- #[toasty::breakpoint]
DELETE FROM "sessions" WHERE "profile_id" IN (SELECT "id" FROM "profiles" WHERE "role" = 'guest');
-- #[toasty::breakpoint]
DELETE FROM "passkeys" WHERE "profile_id" IN (SELECT "id" FROM "profiles" WHERE "role" = 'guest');
-- #[toasty::breakpoint]
DELETE FROM "invites" WHERE "profile_id" IN (SELECT "id" FROM "profiles" WHERE "role" = 'guest');
-- #[toasty::breakpoint]
UPDATE "invites" SET "created_by" = NULL WHERE "created_by" IN (SELECT "id" FROM "profiles" WHERE "role" = 'guest');
-- #[toasty::breakpoint]
DELETE FROM "audio_prefs" WHERE "profile_id" IN (SELECT "id" FROM "profiles" WHERE "role" = 'guest');
-- #[toasty::breakpoint]
DELETE FROM "subtitle_settings" WHERE "profile_id" IN (SELECT "id" FROM "profiles" WHERE "role" = 'guest');
-- #[toasty::breakpoint]
DELETE FROM "started" WHERE "profile_id" IN (SELECT "id" FROM "profiles" WHERE "role" = 'guest');
-- #[toasty::breakpoint]
DELETE FROM "video_places" WHERE "profile_id" IN (SELECT "id" FROM "profiles" WHERE "role" = 'guest');
-- #[toasty::breakpoint]
DELETE FROM "series_places" WHERE "profile_id" IN (SELECT "id" FROM "profiles" WHERE "role" = 'guest');
-- #[toasty::breakpoint]
DELETE FROM "dismissed_videos" WHERE "profile_id" IN (SELECT "id" FROM "profiles" WHERE "role" = 'guest');
-- #[toasty::breakpoint]
DELETE FROM "settings" WHERE "name" = 'tv_profile' AND "value" IN (SELECT CAST("id" AS TEXT) FROM "profiles" WHERE "role" = 'guest');
-- #[toasty::breakpoint]
DELETE FROM "profiles" WHERE "role" = 'guest';
-- #[toasty::breakpoint]
CREATE TABLE "_toasty_new_profiles" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "name" TEXT NOT NULL,
    "role" TEXT NOT NULL CHECK ("role" IN ('admin', 'viewer')),
    "password_hash" TEXT,
    "color" TEXT,
    "created_at" BIGINT NOT NULL
);
-- #[toasty::breakpoint]
INSERT INTO "_toasty_new_profiles" ("id", "name", "role", "password_hash", "color", "created_at") SELECT "id", "name", "role", "password_hash", "color", "created_at" FROM "profiles";
-- #[toasty::breakpoint]
DROP TABLE "profiles";
-- #[toasty::breakpoint]
ALTER TABLE "_toasty_new_profiles" RENAME TO "profiles";
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_profiles_by_name" ON "profiles" ("name");
-- #[toasty::breakpoint]
PRAGMA foreign_keys = ON;
