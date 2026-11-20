CREATE TABLE "push_subscriptions" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "profile_id" BIGINT NOT NULL,
    "endpoint" TEXT NOT NULL,
    "p256dh" TEXT NOT NULL,
    "auth" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL
);
-- #[toasty::breakpoint]
CREATE INDEX "index_push_subscriptions_by_profile_id" ON "push_subscriptions" ("profile_id");
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_push_subscriptions_by_endpoint" ON "push_subscriptions" ("endpoint");
