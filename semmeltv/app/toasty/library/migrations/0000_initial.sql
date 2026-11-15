CREATE TABLE "shows" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "title" TEXT NOT NULL,
    "year" BIGINT,
    "last_aired" BIGINT,
    "running" BOOLEAN NOT NULL,
    "tmdb" BIGINT,
    "imdb" TEXT,
    "ratings_imdb" BIGINT,
    "ratings_rt" BIGINT,
    "ratings_popcorn" BIGINT,
    "ratings_metacritic" BIGINT,
    "ratings_letterboxd" BIGINT,
    "ratings_ebert" BIGINT,
    "rated" TEXT,
    "review" BOOLEAN NOT NULL,
    "about" TEXT NOT NULL
);
-- #[toasty::breakpoint]
CREATE TABLE "episodes" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "show_id" BIGINT NOT NULL,
    "season" BIGINT NOT NULL,
    "number" BIGINT NOT NULL,
    "title" TEXT,
    "path" TEXT NOT NULL,
    "added" TEXT NOT NULL,
    "runtime" BIGINT,
    "ratings_imdb" BIGINT,
    "ratings_rt" BIGINT,
    "ratings_popcorn" BIGINT,
    "ratings_metacritic" BIGINT,
    "ratings_letterboxd" BIGINT,
    "ratings_ebert" BIGINT,
    "video" TEXT,
    "resolution" TEXT,
    "audio" TEXT,
    "review" BOOLEAN NOT NULL
);
-- #[toasty::breakpoint]
CREATE INDEX "index_episodes_by_show_id" ON "episodes" ("show_id");
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_episodes_by_path" ON "episodes" ("path");
-- #[toasty::breakpoint]
CREATE TABLE "movies" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "title" TEXT NOT NULL,
    "year" BIGINT,
    "tmdb" BIGINT,
    "imdb" TEXT,
    "ratings_imdb" BIGINT,
    "ratings_rt" BIGINT,
    "ratings_popcorn" BIGINT,
    "ratings_metacritic" BIGINT,
    "ratings_letterboxd" BIGINT,
    "ratings_ebert" BIGINT,
    "rated" TEXT,
    "path" TEXT NOT NULL,
    "added" TEXT NOT NULL,
    "runtime" BIGINT,
    "video" TEXT,
    "resolution" TEXT,
    "audio" TEXT,
    "review" BOOLEAN NOT NULL,
    "about" TEXT NOT NULL
);
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_movies_by_path" ON "movies" ("path");
