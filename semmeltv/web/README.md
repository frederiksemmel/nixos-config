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

Before deploying a change to the page scripts or styles, run the VM test
([../tests/vm](../tests/vm/README.md)). It runs the TV app, the API and this
app in a NixOS VM and drives the pages in WebKit (iPhone) and Chromium (Android
and a laptop): signing in, browsing, the remote, playing on the phone, handing
over to the TV and back, restarts, feedback and profiles:

    nix build .#checks.x86_64-linux.semmeltv-vm -L

What needs Bluetooth headphones or OpenSubtitles stays in a smaller browser test
against a fake TV API: pairing and forgetting headphones, moving the sound to
them and back, and finding subtitles for a film:

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
They are the page's one picture of the TV: the server keeps what the TV plays
as Unknown, Idle or Playing, from the TV's event stream, and phone.js and
player.js read the rendered state rather than keeping their own. A pulse
region changes every 5 s; `src/live.js` restarts the WebSocket when the pulse
stops for 12 s or the page comes back from being away (a phone that slept), and
marks the page `data-live="stale"` until the pulse is back, which dims the
bar's and the remote's playing state and holds their controls.
`src/lockscreen.js` puts what the TV plays on a phone's lock screen. A phone
shows media keys there only for a page that plays sound, so after a tap on the
remote or on a play button for the TV, the page plays a loop of silence while
the TV plays and pauses it while the TV is paused. The Media Session carries
the TV's title and poster, and its play, pause, seek and next-episode keys go to
the TV's API. It lets go when the phone plays the video itself, after the TV
has played nothing for 30 s, and when another app takes the phone's sound;
the next tap engages it again.
The microphone is plain JavaScript (`src/microphone.js`), because Topcoat's
runtime cannot reach `getUserMedia` or audio worklets.

`src/report.js` sends the page's errors to the TV's journal (`/api/log`) and
keeps a trail of the last 300 things the page did: taps, route changes, every
API call with its status and time, socket and video events, toasts and errors.
The trail lives in `sessionStorage`, so it survives reloads. The speech bubble
in the top bar (`src/feedback.js`) sends the trail with what was typed or
dictated, and an optional screenshot, to the TV's `/api/feedback`. Admins read
the reports on `/feedback`; see the [app README](../app/README.md#feedback).
