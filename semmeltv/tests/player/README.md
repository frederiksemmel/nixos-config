# Player harness

Plays a real library file in a browser the way a phone or laptop plays it from
the TV, with the file's own codecs, and checks that playback moves after seeks.
The VM test can't do this: its films are tiny H.264 clips, and Playwright's
Chromium and WebKit decode no HEVC. Playwright's Firefox does decode HEVC (and
H.264 and AAC) through Media Source Extensions once it finds the system's
ffmpeg libraries, which `shell.nix` puts on `LD_LIBRARY_PATH`. That is the
same hls.js path Firefox and Chrome take on a laptop or an Android phone.

    cd semmeltv/tests/player
    rclone copy "pcloud:SemmelTV/Movies/Ali G꞉ Who Iz I (2026)/" ~/semmeltv-debug/media --include "*.mkv"
    nix-shell --run "python3 -I probe.py firefox"          # which types it plays
    nix-shell --run "python3 -I seek.py FILE"              # every scenario
    nix-shell --run "python3 -I seek.py FILE --only seek-from-start --headed"

`shell.nix` takes nixpkgs from the flake's lock, so ffmpeg is the TV's version.

- `seek.py` serves the stream the way `app/src/stream` does: ffmpeg with the
  TV's arguments (`remux.py`), the offset from ffprobe, the master playlist
  with the codecs, the media playlist with `EXT-X-START`, the init and media
  segments. The page runs hls.js from the URL in `phone.js` and the player's
  own timeline code: everything `phone.js` marks with `// timeline: begin` …
  `// timeline: end` (Player methods) and `// timeline functions: begin` …
  `end` (the functions they call). It prints the player's position, the
  video's clock, its buffered and seekable ranges once every three seconds,
  and exits 1 when a scenario's playback did not move.
- `remux.py` builds ffmpeg's arguments as `remux.rs` does for a browser that
  takes HEVC and 5.1 AAC; keep the two in step. `READY_SEGMENTS=n` in the
  environment overrides how many segments an open waits for.
- `inspect_segments.py DIR` lists each segment's per-track start and end from
  its `tfdt` and `trun` boxes, next to the playlist's times.
- `probe.py` prints which video and audio types each Playwright browser plays
  through MSE.

Safari plays HLS natively, not through hls.js, so this harness doesn't cover
it; that needs an iPhone or a Mac.
