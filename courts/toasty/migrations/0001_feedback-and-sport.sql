ALTER TABLE "watches" ADD COLUMN "sport" TEXT;
-- #[toasty::breakpoint]
CREATE TABLE "feedback" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "created_at" BIGINT NOT NULL,
    "page" TEXT NOT NULL,
    "batch" TEXT NOT NULL,
    "state" TEXT NOT NULL,
    "outcome" TEXT,
    "answer" TEXT,
    "commit" TEXT,
    "marked_at" BIGINT NOT NULL,
    "thread" TEXT NOT NULL
);
-- #[toasty::breakpoint]
CREATE INDEX "index_feedback_by_state" ON "feedback" ("state");
