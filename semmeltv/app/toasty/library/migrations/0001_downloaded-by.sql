ALTER TABLE "episodes" ADD COLUMN "downloaded_by" TEXT;
-- #[toasty::breakpoint]
ALTER TABLE "movies" ADD COLUMN "downloaded_by" TEXT;
