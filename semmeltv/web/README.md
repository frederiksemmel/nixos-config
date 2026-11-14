# SemmelTV web app

A phone-first web app for the TV: browse and search the library, open a title,
play it or pick an episode, control playback, and talk to the TV through the
phone's microphone. The Controls page turns the TV on or off and switches it to
SemmelTV over HDMI-CEC, sets the TV's volume, and the Subtitles page sets
up subtitles. It is a [Topcoat](https://docs.rs/topcoat) app, pinned to
0.11.0, which needs Rust 1.99. It reads and changes everything through the TV's
[HTTP API](../app/API.md).

    SEMMELTV_API_URL=http://127.0.0.1:8090 PORT=3000 cargo run

Everyone signs in. A layer in front of every page asks the TV's API who the
browser's cookie belongs to and sends a stranger to `/profiles`, the "Who's
watching?" picker, which signs in with a passkey or a password, or switches
at once to a profile already signed in on this browser. The server forwards
each visitor's cookie on every API call, so the API answers with that
person's own state and refuses what their role doesn't allow; the web app
also hides admin pages and controls from viewers. `/invite/{token}` greets a
new person, `/account` adds passkeys and sets the password, and `/people` is
the admin's list of people and invite links. The server follows the TV's
event stream with the token in `SEMMELTV_SERVICE_TOKEN_FILE`.

Before deploying a change to the page scripts or styles, run the browser test.
It serves the app against a fake TV API with a film playing, signs in, then taps, scrolls,
opens titles, goes Back and opens the remote in WebKit (iPhone) and Chromium
(Android), and fails if the page stops answering:

    nix build .#checks.x86_64-linux.semmeltv-web-browser -L

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
