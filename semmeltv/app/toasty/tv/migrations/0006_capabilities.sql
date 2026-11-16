CREATE TABLE "capabilities" (
    "profile_id" BIGINT NOT NULL,
    "capability" TEXT NOT NULL CHECK ("capability" IN ('control_tv')),
    PRIMARY KEY ("profile_id", "capability")
);
-- #[toasty::breakpoint]
INSERT INTO "capabilities" ("profile_id", "capability") SELECT "id", 'control_tv' FROM "profiles" WHERE "name" = 'Chris' AND "role" = 'viewer';
