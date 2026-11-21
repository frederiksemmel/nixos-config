CREATE TABLE "push_notices" (
    "profile_id" BIGINT NOT NULL,
    "about" TEXT NOT NULL,
    "sent_at" BIGINT NOT NULL,
    PRIMARY KEY ("profile_id", "about")
);
