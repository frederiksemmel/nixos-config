PRAGMA foreign_keys = OFF;
-- #[toasty::breakpoint]
CREATE TABLE "_toasty_new_capabilities" (
    "profile_id" BIGINT NOT NULL,
    "capability" TEXT NOT NULL CHECK ("capability" IN ('control_tv', 'headphones', 'download')),
    PRIMARY KEY ("profile_id", "capability")
);
-- #[toasty::breakpoint]
INSERT INTO "_toasty_new_capabilities" ("profile_id", "capability") SELECT "profile_id", "capability" FROM "capabilities";
-- #[toasty::breakpoint]
DROP TABLE "capabilities";
-- #[toasty::breakpoint]
ALTER TABLE "_toasty_new_capabilities" RENAME TO "capabilities";
-- #[toasty::breakpoint]
PRAGMA foreign_keys = ON;
