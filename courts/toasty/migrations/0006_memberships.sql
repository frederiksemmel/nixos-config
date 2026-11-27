CREATE TABLE "memberships" (
    "person_id" BIGINT NOT NULL,
    "club_id" BIGINT NOT NULL,
    PRIMARY KEY ("person_id", "club_id")
);
