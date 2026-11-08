# SemmelTV

Apple TV style interface for a personal video library. It reads `library.toml`
at the root of a library folder, draws menus with Slint, and plays video in a
separate mpv window.

    semmeltv [ui|list|artwork|import|understand TEXT] [--library DIR] [--cache DIR]
             [--vfs-cache DIR] [--tmdb-token-file FILE]
             [--manifest FILE] [--listing FILE] [--refresh]

The library is `pcloud:SemmelTV`: `library.toml`, `Movies/`, `Series/` and
`Artwork/`. `--library` defaults to `$SEMMELTV_LIBRARY`, then `~/pcloud/SemmelTV`.
`--cache` defaults to `$SEMMELTV_CACHE`, then `$XDG_CACHE_HOME/semmeltv`.
`--vfs-cache` defaults to `$SEMMELTV_VFS_CACHE`. Nothing in the app writes to the
library: import and artwork work on a local copy of `library.toml`, which the
`/semmeltv-incoming` skill uploads.

## library.toml

The only source of truth, written by `semmeltv import` and fine to edit by hand.
The app reads nothing else: no file names, no folders. An entry whose file is
missing is skipped. Paths are relative to the library root, and `added` is a
date.

    [[show]]
    title = "Dark"
    year = 2017
    tmdb = 70523
    episodes = [
      { season = 1, episode = 1, title = "Secrets", path = "Series/Dark/S01/Dark.S01E01.1080p.x265.DD5.1.mkv", added = 2024-05-01, runtime = 51, video = "HEVC", resolution = "1080p", audio = "5.1" },
    ]

    [[movie]]
    title = "Wind"
    path = "Movies/Wind/WIND.avi"
    added = 2024-05-01
    review = true

`year`, `tmdb` and an episode's `title` are optional. So are `video`, `resolution`
and `audio`, which import reads from the release tags in the file name, then from
the folders above it, and `runtime` in minutes, which the artwork step takes from
TMDB along with a missing episode `title`. `review = true`, on a show, an episode or a film, marks
something a person should check; it changes nothing in the app. Unknown keys are
an error, so typos surface. Import rewrites the file sorted by title, which drops
comments.

Home and the rows sort by `added`, shows by their newest episode. Newly Added is
the 12 newest titles.

## Importing

    semmeltv import --manifest library.toml [--library DIR | --listing FILE]

Compares the videos under `Series/` and `Movies/` with the manifest, parsing file
names as it goes, and adds what is new. Existing entries stay as they are, except
that a missing `video`, `resolution` or `audio` is filled from the path; a value
already there is never replaced. With no manifest yet it is a first import: `added` is each file's date. Later,
`added` is today, because a film sorted today may have an old file date.

`--listing` reads `rclone lsf -R --files-only --format tsp` output instead of the
tree. A new title takes its TMDB id from a search if a token is set. A title with no id, a file with no
episode marker, or a second copy of a title is marked for review. The report also
names entries whose file is missing and videos that were left out.

New videos reach the library through `pcloud:Incoming`: a person starts a Claude
session on beast and runs the `/semmeltv-incoming` skill, which sorts them into
place, runs the import and the artwork step against a local copy, and uploads
the result.

## Title details

A quiet line at the bottom of Home and of a show describes the focused card:

    HEVC 10-bit · 1080p · 5.1 · 2h 11m · Stopped at 1h 04m · Downloaded

Encoding and audio come from `library.toml`. The duration is the one mpv reported
once a file has played, and until then the `runtime` in `library.toml`. The stopping
point comes from `progress.json`, and a finished title has none, nor does one left
in its first minute. A show on Home describes its resume episode, named first, as
in `S2 E3`. Anything unknown is left out. No video file is read for any of this.

## Refreshing

The app checks `library.toml` every 30 seconds and reloads it when it changed.
Focus stays on the same title and an open show stays open. During playback the
reload waits until the video ends. Art for cards that had none is looked for
again on every reload.

## Streaming and offline

On semmeltv5, `/var/lib/semmeltv/library` is a read-only rclone mount of pCloud
with a full VFS cache on local disk. The cache size is
`services.semmeltv.offlineCacheGiB` in `semmeltv/hosts/semmeltv5/default.nix`, and
the least recently watched titles are evicted first.

rclone only keeps what was read, so when a title starts the app reads the rest of
the file through the mount in the background, slowly, followed by the next
episode of a show. A file larger than the cache is skipped. `SEMMELTV_VFS_CACHE`
is the cache's file tree (`.../vfs/pcloud/SemmelTV`), `SEMMELTV_VFS_CACHE_BYTES` its size,
and `SEMMELTV_REMOTE` a `host:port` probed every poll to tell whether pCloud can
be reached. Without `SEMMELTV_VFS_CACHE` everything counts as streaming and cards
show no state.

Cards show what is on the device: Downloaded, Downloading or Not downloaded, and
for a show a count such as 3 of 10 downloaded. A file counts as downloaded when
its allocated blocks in the cache cover its size, since cache files are sparse.
Newly added titles are not downloaded until watched.

Offline, the app uses the last good copy of the manifest,
`library.last-good.toml` in the cache, and plays downloaded files straight from
the rclone cache, which also works when the mount is down or was started without
a network. Selecting anything else says it can't be reached. rclone's own view of
a cached file only survives offline until `--dir-cache-time` (1h) runs out, which
is why the app does not depend on it. Files played straight from the cache do not
refresh their place in the eviction order, and art read through the mount is
only there offline if rclone kept it.

## Remote

The TV's own remote works over HDMI-CEC through the kernel's `/dev/cec*` devices,
which needs membership of the `video` group. The app appears on the TV as a
playback device named SemmelTV. Machines without CEC simply skip this.

Arrows move, OK selects, Back or Exit goes back, and Play and Pause toggle
playback. During a video the arrows seek, and Stop quits.

The app wakes the TV and switches it to its input when the machine cold boots
with HDMI connected, or when HDMI is plugged in after being out for at least 10
seconds. Restarting the app never wakes it, and neither does a shorter hot-plug
pulse. Once the TV sends Standby, plugging in leaves it off until the TV shows it
is on again. A cold boot wakes it regardless.

## Playing a video

The remote and the keyboard do the same thing in mpv. Left and Right skip 10
seconds, and repeating or holding them speeds up: 10, 10, 20, 30, 60, then 120
seconds a press. Changing direction or leaving them alone for 1.2 seconds starts
over. Up and Down skip a minute. Seeks stop at the start and 5 seconds before the
end, so a long fast-forward does not end the episode.

Starting, seeking, pausing and resuming show the title, a progress bar, and the
elapsed and remaining time at the bottom of the screen, for 4 seconds or for as
long as the video is paused. A seek also shows where it lands and the size of the
last jump. It is drawn with mpv's `osd-overlay` because mpv shows nothing of its
own for commands sent over its IPC socket, which is how the remote reaches it.

Stopping a video saves its position and playing it again resumes there. So does
a restart, and a crash or power cut loses at most the last 30 seconds. A video
that played to its end starts over. `progress.json` in the cache keeps the last
episode started per show under `shows`, and each file's stopping point and real
duration under `files`. mpv's own saved positions live in `watch-later/` in the
cache, one folder per library file, so a position saved while streaming also
applies to the same file played from the rclone cache, and two files with the
same name stay apart.

## Skipping intros and recaps

During an episode's recap or intro a Skip button shows in mpv, and OK presses it.
Otherwise OK pauses, as Play and Pause always do. Skipping is never automatic,
because the times do not fit every release: a file with a recap the database's
release lacks lands partway through the titles.

Times come from [TheIntroDB](https://theintrodb.org) when an episode starts, and
are not stored. An episode needs its show's `tmdb` in `library.toml`, and without
one or without a network it plays with no button. When
several ranges exist the earliest counts, since later ones have been wrong. Their
terms allow personal, non-commercial use and forbid passing the data on, so
timings must not be exported or committed.

## Voice commands

`semmeltv understand TEXT` shows what a spoken sentence would do, in English,
German or Spanish:

    continue playing Outlander
    spiele Outlander Staffel 2 Folge 3
    descarga la siguiente temporada de Outlander

Continuing plays the next episode once the last one ended. Titles match the
library loosely, so transcription slips are forgiven; translated titles are not
known. Download requests are understood but not carried out yet.

## Artwork

    semmeltv artwork --manifest DIR/library.toml [--refresh]

Artwork is fetched when titles are added, not on the TVs, which need no TMDB
token. The step reads the manifest and, for every title with a `tmdb` id, saves
images into `DIR/Artwork`, beside the manifest:

    Artwork/movie/<tmdb id>/backdrop.jpg
    Artwork/tv/<tmdb id>/backdrop.jpg
    Artwork/tv/<tmdb id>/s01e02.jpg

They are 640x360 JPEGs: a backdrop for shows and films, a still for episodes.
It also writes a missing `runtime` and episode `title` into the manifest. It only
asks TMDB for what is missing, so a run over a copy of `pcloud:SemmelTV/Artwork`
is quick. It refuses a manifest inside `--library`, so it never writes through
the mount. Titles without a `tmdb` id are listed and keep the gradient.

The app shows `<library>/Artwork/...` when the file is there and a gradient with
the title otherwise. The TMDB notice on Home appears once any title has an id.

TMDB's terms allow keeping images for six months, so refreshing is a manual
step: `--refresh` downloads again every image older than five months and deletes
one it cannot replace. The `/semmeltv-incoming` skill describes the upload. The
step needs a TMDB API Read Access Token, from `TMDB_TOKEN_FILE`,
`--tmdb-token-file` or `TMDB_TOKEN`.

## Development

    cd semmeltv/app
    devenv shell
    cargo test
    cargo run -- ui

`semmeltv/devenv.nix` holds the tooling: the Rust toolchain, `slint-lsp`, `mpv`,
`wlr-randr` and `bacon`, with the build environment the package uses
(`app/build-env.nix`). It imports the light root `devenv.nix` of the repo, and
`devenv shell` picks it up anywhere under `semmeltv/`. The shell sets
`SEMMELTV_LIBRARY` to `~/pcloud/SemmelTV`, where the desktop's rclone mount of
the whole pCloud account has the library, unless it is already set.
