CREATE TABLE "usage" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "at" BIGINT NOT NULL,
    "activity" TEXT NOT NULL CHECK ("activity" IN ('sign_in', 'visit', 'play')),
    "profile_id" BIGINT NOT NULL,
    "device" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "play" TEXT NOT NULL,
    "seconds" BIGINT NOT NULL
);
-- #[toasty::breakpoint]
CREATE INDEX "index_usage_by_at" ON "usage" ("at");
-- #[toasty::breakpoint]
CREATE INDEX "index_usage_by_play" ON "usage" ("play");
