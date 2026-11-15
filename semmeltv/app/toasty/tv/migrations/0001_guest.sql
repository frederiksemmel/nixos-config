PRAGMA foreign_keys = OFF;
-- #[toasty::breakpoint]
CREATE TABLE "_toasty_new_profiles" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "name" TEXT NOT NULL,
    "role" TEXT NOT NULL CHECK ("role" IN ('admin', 'viewer', 'guest')),
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
PRAGMA foreign_keys = ON;
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_profiles_by_name" ON "profiles" ("name");
-- #[toasty::breakpoint]
INSERT INTO "profiles" ("name", "role", "created_at") SELECT 'Guest', 'guest', unixepoch() WHERE NOT EXISTS (SELECT 1 FROM "profiles" WHERE "name" = 'Guest');
