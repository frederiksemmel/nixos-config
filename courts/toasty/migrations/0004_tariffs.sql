CREATE TABLE "tariffs" (
    "court_id" BIGINT NOT NULL,
    "club_id" BIGINT NOT NULL,
    "checked_at" BIGINT NOT NULL,
    "week" TEXT NOT NULL,
    PRIMARY KEY ("court_id")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_tariffs_by_club_id" ON "tariffs" ("club_id");
