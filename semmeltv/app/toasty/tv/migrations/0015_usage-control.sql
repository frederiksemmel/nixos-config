PRAGMA foreign_keys = OFF;
-- #[toasty::breakpoint]
CREATE TABLE "_toasty_new_usage" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "at" BIGINT NOT NULL,
    "activity" TEXT NOT NULL CHECK ("activity" IN ('sign_in', 'visit', 'play', 'control')),
    "profile_id" BIGINT NOT NULL,
    "device" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "play" TEXT NOT NULL,
    "seconds" BIGINT NOT NULL
);
-- #[toasty::breakpoint]
INSERT INTO "_toasty_new_usage" ("id", "at", "activity", "profile_id", "device", "title", "play", "seconds") SELECT "id", "at", "activity", "profile_id", "device", "title", "play", "seconds" FROM "usage";
-- #[toasty::breakpoint]
DROP TABLE "usage";
-- #[toasty::breakpoint]
ALTER TABLE "_toasty_new_usage" RENAME TO "usage";
-- #[toasty::breakpoint]
CREATE INDEX "index_usage_by_at" ON "usage" ("at");
-- #[toasty::breakpoint]
CREATE INDEX "index_usage_by_play" ON "usage" ("play");
-- #[toasty::breakpoint]
PRAGMA foreign_keys = ON;
