# SemmelTV

Apple TV style interface for a personal video library. It reads `Series/` and
`Movies/` under a library folder, draws menus with Slint, and plays video in a
separate mpv window.

    semmeltv [ui|list|artwork|understand TEXT] [--library DIR] [--cache DIR]
                                               [--tmdb-token-file FILE]

`--library` defaults to `$SEMMELTV_LIBRARY`, then `~/pcloud`. `--cache` defaults
to `$SEMMELTV_CACHE`, then `$XDG_CACHE_HOME/semmeltv`. The cache must be outside
the library, and nothing ever writes to the library.

## Remote

The TV's own remote works over HDMI-CEC through the kernel's `/dev/cec*` devices,
which needs membership of the `video` group. The app appears on the TV as a
playback device named SemmelTV. Machines without CEC simply skip this.

Arrows move, OK selects, Back or Exit goes back, and Play and Pause toggle
playback. During a video the arrows seek, and Stop quits.

Stopping a video saves its position and playing it again resumes there. So does
a restart, and a crash or power cut loses at most the last 30 seconds. A video
that played to its end starts over. The last episode started per show is kept in
`progress.json` in the cache.

## Skipping intros and recaps

During an episode's recap or intro a Skip button shows in mpv, and OK presses it.
Otherwise OK pauses, as Play and Pause always do. Skipping is never automatic,
because the times do not fit every release: a file with a recap the database's
release lacks lands partway through the titles.

Times come from [TheIntroDB](https://theintrodb.org) when an episode starts, and
are not stored. An episode needs a TMDB id, from `semmeltv artwork` or `tmdb` in
`meta.toml`, and without one or without a network it plays with no button. When
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

`semmeltv artwork` runs apart from the UI, after a sync or on demand. A card uses
the first of:

1. `poster.jpg` or `poster.png` in the title's folder
2. a TMDB image: a backdrop for shows and films, a still for episodes
3. a gradient with the title

The step needs a TMDB API Read Access Token, from `TMDB_TOKEN_FILE`,
`--tmdb-token-file` or `TMDB_TOKEN`. Images older than six months are refreshed,
or deleted if that fails, as TMDB's terms require. Titles it could not match
safely keep the gradient, and are printed and saved to `tmdb-report.txt` in the
cache.

## meta.toml

Optional, in a title's folder, all fields optional:

    title = "Star Wars: Maul – Shadow Lord"
    year = 2026
    tmdb = 12345

## Development

    nix develop .#semmeltv
    cd packages/semmeltv
    cargo test
    cargo run -- ui --library /path/to/library
