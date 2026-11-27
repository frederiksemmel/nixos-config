ALTER TABLE "watches" ADD COLUMN "person_id" BIGINT;
-- #[toasty::breakpoint]
ALTER TABLE "subscriptions" ADD COLUMN "person_id" BIGINT;
-- #[toasty::breakpoint]
ALTER TABLE "invites" ADD COLUMN "person_id" BIGINT;
-- #[toasty::breakpoint]
CREATE TABLE "people" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "name" TEXT NOT NULL,
    "admin" BOOLEAN NOT NULL,
    "created_at" BIGINT NOT NULL
);
-- #[toasty::breakpoint]
CREATE TABLE "devices" (
    "token_hash" TEXT NOT NULL,
    "person_id" BIGINT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "seen_at" BIGINT NOT NULL,
    "kind" TEXT NOT NULL,
    PRIMARY KEY ("token_hash")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_devices_by_person_id" ON "devices" ("person_id");
-- #[toasty::breakpoint]
CREATE TABLE "asks" (
    "token" TEXT NOT NULL,
    "person_id" BIGINT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "until" TEXT NOT NULL,
    "options" TEXT NOT NULL,
    "picks" TEXT NOT NULL,
    PRIMARY KEY ("token")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_asks_by_person_id" ON "asks" ("person_id");
