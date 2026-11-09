# SemmelTV web app

A phone-first web app for the TV: browse and search the library, open a title,
play it or pick an episode, control playback, and talk to the TV through the
phone's microphone. The Controls page turns the TV on or off, switches it to
SemmelTV and presses its volume keys over HDMI-CEC, and the Subtitles page sets
up subtitles. It is a [Topcoat](https://docs.rs/topcoat) app, pinned to
0.11.0, which needs Rust 1.99. It reads and changes everything through the TV's
[HTTP API](../app/API.md).

    SEMMELTV_API_URL=http://127.0.0.1:8090 PORT=3000 cargo run

`SEMMELTV_API_URL` is where the server reaches the API. Browsers load artwork
and stream the microphone from `/api` on the page's own origin, which is how
`tailscale serve` maps it on a TV. Without that mapping, as with `cargo run`,
set `SEMMELTV_API_PUBLIC_URL=http://127.0.0.1:8090` as well.

Topcoat serves its browser runtime and this app's CSS and script from an asset
bundle. `semmeltv-web bundle DIR` writes the bundle from the binary itself, so
there is no separate CLI. The Nix package does this at build time and sets
`SEMMELTV_WEB_ASSETS`. Without that variable the server bundles into a
temporary directory at startup.

Pages render on the server. Search, season tabs and buttons are Topcoat runtime
expressions, and play buttons call procedures. The now-playing bar and the voice
feedback are live regions that the server updates over Topcoat's WebSocket.
The microphone is plain JavaScript (`src/microphone.js`), because Topcoat's
runtime cannot reach `getUserMedia` or audio worklets.
