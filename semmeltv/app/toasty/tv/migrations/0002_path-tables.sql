ALTER TABLE "files" RENAME TO "legacy_files";
-- #[toasty::breakpoint]
ALTER TABLE "file_places" RENAME TO "legacy_file_places";
-- #[toasty::breakpoint]
ALTER TABLE "show_places" RENAME TO "legacy_show_places";
-- #[toasty::breakpoint]
ALTER TABLE "watching" RENAME TO "legacy_watching";
-- #[toasty::breakpoint]
ALTER TABLE "dismissed" RENAME TO "legacy_dismissed";
