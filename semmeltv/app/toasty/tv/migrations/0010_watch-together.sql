CREATE TABLE "watch_together" (
    "profile_id" BIGINT NOT NULL,
    "show_id" BIGINT NOT NULL,
    "partner_id" BIGINT NOT NULL,
    PRIMARY KEY ("profile_id", "show_id", "partner_id")
);
