CREATE TABLE "video_facts" (
    "kind" TEXT NOT NULL CHECK ("kind" IN ('episode', 'movie')),
    "video_id" BIGINT NOT NULL,
    "duration" REAL,
    "credits" REAL,
    "subtitle_delay" REAL NOT NULL,
    "embedded_subtitles" BIGINT,
    PRIMARY KEY ("kind", "video_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "series_places" (
    "profile_id" BIGINT NOT NULL,
    "show_id" BIGINT NOT NULL,
    "season" BIGINT NOT NULL,
    "episode" BIGINT NOT NULL,
    "finished" BOOLEAN NOT NULL,
    PRIMARY KEY ("profile_id", "show_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "dismissed_videos" (
    "profile_id" BIGINT NOT NULL,
    "kind" TEXT NOT NULL CHECK ("kind" IN ('episode', 'movie')),
    "video_id" BIGINT NOT NULL,
    "at" BIGINT NOT NULL,
    PRIMARY KEY ("profile_id", "kind", "video_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "video_places" (
    "profile_id" BIGINT NOT NULL,
    "kind" TEXT NOT NULL CHECK ("kind" IN ('episode', 'movie')),
    "video_id" BIGINT NOT NULL,
    "stopped_at" REAL,
    "last_played" BIGINT,
    "finished" BOOLEAN NOT NULL,
    "audio_track" BIGINT,
    "subtitle_track" BIGINT,
    "subtitles_shown" BOOLEAN,
    PRIMARY KEY ("profile_id", "kind", "video_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "started" (
    "profile_id" BIGINT NOT NULL,
    "kind" TEXT NOT NULL CHECK ("kind" IN ('series', 'movie')),
    "title_id" BIGINT NOT NULL,
    "last_played" BIGINT NOT NULL,
    PRIMARY KEY ("profile_id", "kind", "title_id")
);
