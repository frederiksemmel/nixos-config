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
`{ kind: "movie" | "show", key, title, year, subtitle, poster, backdrop, ratings }`.
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

`now_playing` is null or
`{ title, caption, movie, show, season, episode, position, duration, paused, muted, loading }`.
For a movie, `movie` is its key. For an episode, `show`, `season` and `episode`
are set.

## TV

| Method | Path | Body |
| --- | --- | --- |
| POST | `/api/tv/{control}` | none. `control` is `on`, `off`, `semmeltv`, `volume-up`, `volume-down` or `mute` |

These go to the TV over HDMI-CEC and do what the same voice commands do. `on`
turns the TV on and switches it to SemmelTV, `semmeltv` switches an already-on
TV to SemmelTV, and `off` puts it in standby. The volume controls press the
TV's own Volume Up, Volume Down and Mute keys; `mute` toggles. They answer
like the playing requests, with `ok` true once the command is sent. The TV
gives no reply to tell whether it obeyed a volume key.

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

## Trying it

    curl -s localhost:8090/api/search?q=lotus | jq '.results[].title'
    curl -s -X POST localhost:8090/api/play -d '{"show":"the-white-lotus"}' -H 'content-type: application/json'
    curl -s -X POST localhost:8090/api/playback/toggle
    curl -s -X PUT localhost:8090/api/subtitles -d '{"size":1.3,"background":0.5}' -H 'content-type: application/json'
    websocat ws://localhost:8090/api/events
