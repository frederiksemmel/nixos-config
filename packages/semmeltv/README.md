# SemmelTV

Apple TV style interface for a personal video library. It reads `Series/` and
`Movies/` under a library folder, draws menus with Slint, and plays video in a
separate mpv window.

    semmeltv [ui|list|artwork] [--library DIR] [--cache DIR]
                               [--no-frames] [--tmdb-token-file FILE]

`--library` defaults to `$SEMMELTV_LIBRARY`, then `~/pcloud`. `--cache` defaults
to `$SEMMELTV_CACHE`, then `$XDG_CACHE_HOME/semmeltv`. The cache must be outside
the library, and nothing ever writes to the library.

## Artwork

`semmeltv artwork` runs apart from the UI, after a sync or on demand. A card uses
the first of:

1. `poster.jpg` or `poster.png` in the title's folder
2. a TMDB image
3. a frame extracted with ffmpeg

TMDB needs an API Read Access Token, from `TMDB_TOKEN_FILE`, `--tmdb-token-file`
or `TMDB_TOKEN`. Images older than six months are refreshed, as TMDB's terms
require. Titles it could not match safely are printed and saved to
`tmdb-report.txt` in the cache.

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
