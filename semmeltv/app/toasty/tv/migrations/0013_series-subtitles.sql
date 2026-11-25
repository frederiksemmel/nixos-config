ALTER TABLE "series_places" ADD COLUMN "subtitles_shown" BOOLEAN;
-- #[toasty::breakpoint]
ALTER TABLE "series_places" ADD COLUMN "subtitle_language" TEXT;
