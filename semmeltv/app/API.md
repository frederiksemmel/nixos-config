# SemmelTV HTTP API

The app serves this API when `SEMMELTV_API` (or `--api`) names an address,
normally `127.0.0.1:8090`. It runs on its own thread, and the TV keeps working
if the address cannot be bound. There is no authentication: bind it to
localhost, and let `tailscale serve` expose it on the tailnet. On semmeltv5 it
is at `https://semmeltv5.cuscus-exponential.ts.net/api/...`.

Requests that change something on the TV take the same path as voice. The app
turns them into an action, resolves it against the library and progress, and
performs it on the UI thread. They answer with what happened:

    { "message": "Playing Outlander · season 2, episode 1", "ok": true }

`ok` is false, with status 422, when the TV refused, for example "Nothing is
playing". The status is 503 when the TV did not answer within 5 s. Other errors
look like `{ "error": "..." }`, with 400 or 404.

## Library

| Method | Path | Answer |
| --- | --- | --- |
| GET | `/api/library` | `{ newest, shows, movies }`, lists of cards |
| GET | `/api/search?q=TEXT` | `{ results }`: cards ranked by how well the title matches, prefixes first, typos forgiven |
| GET | `/api/movies/{key}` | a card plus `details`, `runtime_minutes`, `stopped_at`, `duration` and `tags` |
| GET | `/api/shows/{key}` | a card plus `next` (where playing carries on) and `seasons`, each with `episodes` |

A card is
`{ kind: "movie" | "show", key, title, years, subtitle, poster, backdrop, ratings }`.
`years` is a film's year, or the years a show ran ("2014–2025", or "2014–"
while it still runs), or null.
`ratings` is `{ imdb, rotten_tomatoes, fresh, popcorn, full }`, and any of these
can be null. `popcorn` is the Rotten Tomatoes audience score, and `full` is true
when it is 60% or more.
`poster` is an artwork path, or null until the title's 342x513 poster is in
the library. `backdrop` is an artwork path, or null for a title without a TMDB
id. An
episode has `season`, `episode`, `title`, `label`, `short_label`, `still`,
`ratings` (the same shape as a card's, mostly IMDb only), `band`, `details`,
`stopped_at` and `duration`. `band` names the episode's place on the heatmap
scale from its IMDb rating: `awesome` (9.0 and up), `great` (8.0), `good`
(7.0), `regular` (6.0), `bad` (5.0) or `garbage`, and is null without one. Keys are the TV's own: `dune-2021`,
`outlander`.

## Artwork

| Method | Path |
| --- | --- |
| GET | `/api/movies/{key}/poster` |
| GET | `/api/movies/{key}/backdrop` |
| GET | `/api/shows/{key}/poster` |
| GET | `/api/shows/{key}/backdrop` |
| GET | `/api/shows/{key}/episodes/{season}/{episode}/still` |

These return JPEGs with `Cache-Control: public, max-age=86400` and an `ETag`,
and answer `If-None-Match` with 304. The status is 404 when the library has no
such image.

## Playing

| Method | Path | Body |
| --- | --- | --- |
| POST | `/api/play` | `{ "movie": key }`, or `{ "show": key }` to carry on, or `{ "show": key, "season": 2, "episode": 3 }` |
| POST | `/api/command` | `{ "text": "play outlander season 2" }`, understood exactly like speech |
| POST | `/api/listen` | none: start listening, as the remote's red, green and blue keys do |
| GET | `/api/playback` | `{ now_playing }`, see below |
| POST | `/api/playback/{control}` | none. `control` is `pause`, `resume`, `toggle`, `stop`, `mute` or `unmute` |
| POST | `/api/playback/seek` | `{ "to": seconds }` or `{ "by": seconds }`, negative to go back |
| POST | `/api/playback/skip` | `{ "direction": "back" }` or `"forward"`: a press of the remote's left or right key, so presses in a row skip further |

`now_playing` is null or
`{ title, caption, movie, show, season, episode, position, duration, paused, muted, loading, jump }`.
For a movie, `movie` is its key. For an episode, `show`, `season` and `episode`
are set. `jump` is the last skip in seconds, negative going back, while a run of
skips can still continue, and null otherwise.

## TV

| Method | Path | Body |
| --- | --- | --- |
| GET | `/api/tv` | none |
| POST | `/api/tv/{control}` | none. `control` is `on`, `off`, `semmeltv`, `volume-up`, `volume-down`, `mute` or `pair` |

TV control sits behind one interface in `src/tv`, with a backend per ability,
so the API and the app never know which vendor is behind it:

- Power and input go over HDMI-CEC on every TV.
- Volume goes over the UPnP RenderingControl of the renderer that
  `SEMMELTV_TV_RENDERER` names (its device description, such as
  `http://192.168.1.148:9197/dmr`). This is plain DLNA and works on any TV
  that has a renderer.
- Pairing is per vendor. Only `samsung` has one: `src/tv/samsung.rs`.

`SEMMELTV_TV_KIND` (`generic` or `samsung`) says which TV it is. When it is
unset, the kind comes from the `<manufacturer>` in the renderer's description,
read when the TV is first needed. A new vendor is one file in `src/tv` and one
arm in `backends`.

GET `/api/tv` answers `{ "kind": "samsung", "volume": true, "pairing": true }`:
which TV it is and what it can do. `kind` is null while the TV can't be reached
to tell.

`on`, `off` and `semmeltv` do what the same voice commands do. `on` turns the
TV on and switches it to SemmelTV, `semmeltv` switches an already-on TV to
SemmelTV, and `off` puts it in standby. They answer like the playing requests,
with `ok` true once the command is sent.

`volume-up` and `volume-down` move the TV's own volume by 2 and answer with the
new value, `{ "message": "TV volume 13", "ok": true }`. `mute` toggles and
answers "TV muted" or "TV sound on". A Samsung ignores volume keys sent over
CEC by a playback device, which is why volume goes over the network.

`pair` asks the TV to allow SemmelTV, and only works on a TV that pairs. A
Samsung shows a prompt and accepts volume changes only from clients someone
allowed there. The request stays open until someone presses Allow on the TV,
up to 40 seconds, and answers "Paired with the TV". The TV's token is kept in
`tv-token` in the cache directory and sent along when pairing again. Pairing is
needed once per TV.

Failures answer 422 with `ok` false and a message that states the fact:

| Message | When |
| --- | --- |
| The TV is off or not on the network | the TV doesn't answer; the reason goes to the app's log |
| The TV is not paired with SemmelTV yet | a TV that pairs refuses a change (a Samsung answers 501) |
| The TV refused the change | a TV that doesn't pair refuses, with a SOAP fault or a status other than 2xx |
| Volume control is not supported by this TV | no renderer is configured |
| Pairing is not supported by this TV | the TV's kind has no pairing |
| The TV denied the pairing | someone pressed Deny |
| Nobody pressed Allow on the TV in time | the prompt timed out |

The TV's own error code goes to the app's log.

## Subtitles

| Method | Path | Body |
| --- | --- | --- |
| GET | `/api/subtitles` | none |
| PUT | `/api/subtitles` | any of the settings below; fields left out keep their value |
| POST | `/api/subtitles/track` | `{ "track": id }` to show that track, `{ "track": null }` to hide subtitles |
| POST | `/api/subtitles/toggle` | none: show or hide, as the remote's yellow key does |

GET and PUT answer `{ settings, playing }`. The TV keeps the settings in
`subtitles.json` in its cache directory (`/var/lib/semmeltv/cache` on a Pi),
and every video starts with them. A PUT also restyles what is playing at once.
It answers 400 when a value is malformed or a field is unknown, and clamps
numbers into their range.

| Setting | Meaning | Default |
| --- | --- | --- |
| `shown` | subtitles show when a video starts | `false` |
| `languages` | preferred languages in order, as ISO 639 codes | `["en", "de"]` |
| `size` | text scale, 0.5 to 2.5 | `1.0` |
| `position` | how far down the screen, 50 (middle) to 100 (bottom) | `100` |
| `color` | text colour, `#RRGGBB` | `"#FFFFFF"` |
| `outline` | outline width, 0 to 6 | `1.65` |
| `outline_color` | outline colour, `#RRGGBB` | `"#000000"` |
| `shadow` | shadow offset, 0 to 6 | `0` |
| `background` | opacity of a black box behind the text, 0 (no box) to 1 | `0` |
| `restyle` | also restyle subtitles that bring their own styling | `false` |

`playing` is null when nothing plays, or `{ visible, track, tracks }`.
`track` is the selected track's id or null, and `visible` says whether it
shows. Each track is
`{ id, language, title, label, forced, hearing_impaired, external }`, where
`label` is the language's English name, or the title when the language is
unknown.

The track and toggle requests answer like the playing requests above, for
example `{ "message": "Subtitles: English", "ok": true }`, and the TV shows
the same message on screen. They refuse with "Nothing is playing" or "No
subtitles in this file".

## Storage

| Method | Path | Body |
| --- | --- | --- |
| GET | `/api/storage` | |
| POST | `/api/storage/remove` | `{ "kind": "movie" \| "series" \| "other", "key": "dune-2021" }` |

`GET /api/storage` answers what the offline cache holds, as of the app's last
poll (every 30 s, and after each download or removal):

    { "disk": { "total", "free" }, "used", "marks": { "high", "low", "floor" },
      "titles": [ { "kind", "key", "name", "bytes", "complete", "total",
                    "last_watched", "cached_at", "keep" } ],
      "next": [ { "kind", "key" } ] }

Sizes are bytes; `bytes` and `used` count the blocks the sparse cache files
take on disk. `complete` of `total` is how many of the title's files are fully
downloaded. `last_watched` and `cached_at` are Unix seconds, and `last_watched`
is null for a title never played. `keep` says why automatic removal skips the
title: `playing`, `watching` (in Currently watching), `up-next` (in the
download queue) or `started` (a file has a resume point), or null. A file the
library does not list is grouped by its folder as kind `other`, keyed by its
path under the library. `next` is the order titles would be removed in. The
status is 404 when the TV keeps no downloads.

`POST /api/storage/remove` deletes a title's files from the cache, and answers
like the other requests. It refuses a title that is playing or up next, since
it would download again straight away.

## Event stream

`GET /api/events` upgrades to a WebSocket that sends one JSON text message per
event. It starts with the current playback:

    { "type": "playback", "now_playing": { ... } | null }

After that it sends playback whenever it changes, at most about once a second
while playing, and every voice event:

    { "type": "voice", "state": "listening" }
    { "type": "voice", "state": "transcribing" }
    { "type": "voice", "state": "heard", "text": "play dune", "action": "Playing Dune", "understood": true }
    { "type": "voice", "state": "nothing" }
    { "type": "voice", "state": "no-microphone" }
    { "type": "voice", "state": "failed", "message": "..." }

## Phone microphone

`GET /api/microphone` upgrades to a WebSocket that takes binary messages of
16 kHz mono signed 16-bit little-endian audio. The app plays them with `aplay`
into `SEMMELTV_PHONE_MICROPHONE`, an ALSA loopback whose other end is the app's
own `SEMMELTV_MICROPHONE`. Only one stream is taken at a time, and a second one
is closed with code 1013. The endpoint answers 404 when no phone microphone is
set up.

## Downloads

Downloads are a separate service, `semmeltv downloads`, normally on
`127.0.0.1:8092`, so a restart of the TV app never cuts a transfer short. Its
paths start with `/api/downloads` too.

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/downloads` | | `{ proposals, jobs, room, quota, free }` |
| POST | `/api/downloads/search` | `{ text }`, or `{ pick: { kind, tmdb, season, episode } }` | a proposal |
| POST | `/api/downloads/confirm` | `{ proposal, choice }` | `{ message, ok, job }` |
| POST | `/api/downloads/dismiss` | `{ proposal }` | `{ message, ok }` |
| POST | `/api/downloads/jobs/{id}/stop` | | `{ message, ok, job }` |

`text` is what was said or typed, such as "outlander season 8". It resolves to
one TMDB title, or to a `question` with up to four `options`
(`{ kind, tmdb, label }`), one of which is sent back as `pick`. A proposal holds
the `target` (TMDB and IMDb ids, season, aired episodes), up to three `choices`
ranked by the scorer, each with `facts`, `why` and its torrents in `parts`, and
`dropped`, the number of releases the checks removed. Nothing downloads until a
choice is confirmed. Proposals expire after 30 minutes.

A job moves through `checking` (reading the torrent's files), `downloading`,
`uploading` (to pCloud, then the import), and ends `done`, `failed` or
`stopped`. Each part reports `progress`, `total`, `speed`, `peers` and
`uploaded`, the bytes sent to peers. `room` is the space left for a new
download: the quota minus running downloads, and the free disk minus what they
still need and a reserve.

## Trying it

    curl -s localhost:8090/api/search?q=lotus | jq '.results[].title'
    curl -s -X POST localhost:8090/api/play -d '{"show":"the-white-lotus"}' -H 'content-type: application/json'
    curl -s -X POST localhost:8090/api/playback/toggle
    curl -s -X PUT localhost:8090/api/subtitles -d '{"size":1.3,"background":0.5}' -H 'content-type: application/json'
    websocat ws://localhost:8090/api/events
    curl -s -X POST localhost:8092/api/downloads/search -d '{"text":"outlander season 8"}' -H 'content-type: application/json' | jq '.choices[0].facts'
