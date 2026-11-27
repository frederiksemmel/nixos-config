CREATE TABLE "watches" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "club_id" BIGINT,
    "date" TEXT NOT NULL,
    "earliest" BIGINT NOT NULL,
    "latest" BIGINT NOT NULL,
    "indoor" BOOLEAN NOT NULL,
    "created_at" BIGINT NOT NULL
);
-- #[toasty::breakpoint]
CREATE INDEX "index_watches_by_date" ON "watches" ("date");
-- #[toasty::breakpoint]
CREATE TABLE "told" (
    "watch_id" BIGINT NOT NULL,
    "court_id" BIGINT NOT NULL,
    "start" BIGINT NOT NULL,
    "date" TEXT NOT NULL,
    PRIMARY KEY ("watch_id", "court_id", "start")
);
-- #[toasty::breakpoint]
CREATE TABLE "settings" (
    "name" TEXT NOT NULL,
    "value" TEXT NOT NULL,
    PRIMARY KEY ("name")
);
-- #[toasty::breakpoint]
CREATE TABLE "subscriptions" (
    "endpoint" TEXT NOT NULL,
    "p256dh" TEXT NOT NULL,
    "auth" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    PRIMARY KEY ("endpoint")
);
-- #[toasty::breakpoint]
CREATE TABLE "days" (
    "club_id" BIGINT NOT NULL,
    "date" TEXT NOT NULL,
    "fetched_at" BIGINT NOT NULL,
    "courts" TEXT NOT NULL,
    "busy" TEXT NOT NULL,
    PRIMARY KEY ("club_id", "date")
);
-- #[toasty::breakpoint]
CREATE TABLE "clubs" (
    "id" BIGINT NOT NULL,
    "name" TEXT NOT NULL,
    "canonical" TEXT NOT NULL,
    "address" TEXT NOT NULL,
    "zip" TEXT NOT NULL,
    "city" TEXT NOT NULL,
    "latitude" REAL NOT NULL,
    "longitude" REAL NOT NULL,
    "phone" TEXT NOT NULL,
    "web" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "courts" TEXT NOT NULL,
    "checked_at" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE TABLE "closed_clubs" (
    "club_id" BIGINT NOT NULL,
    "why" TEXT NOT NULL,
    "at" BIGINT NOT NULL,
    PRIMARY KEY ("club_id")
);
