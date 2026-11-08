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
`{ kind: "movie" | "show", key, title, year, subtitle, backdrop, ratings }`.
`ratings` is `{ imdb, rotten_tomatoes, fresh, popcorn, full }`, and any of these
can be null. `popcorn` is the Rotten Tomatoes audience score, and `full` is true
when it is 60% or more.
`backdrop` is an artwork path, or null for a title without a TMDB id. An
episode has `season`, `episode`, `title`, `label`, `short_label`, `still`,
`details`, `stopped_at` and `duration`. Keys are the TV's own: `dune-2021`,
`outlander`.

## Artwork

| Method | Path |
| --- | --- |
| GET | `/api/movies/{key}/backdrop` |
| GET | `/api/shows/{key}/backdrop` |
| GET | `/api/shows/{key}/episodes/{season}/{episode}/still` |

These return JPEGs with `Cache-Control: public, max-age=86400` and an `ETag`,
and answer `If-None-Match` with 304. The status is 404 when the library has no
such image. The library has backdrops and stills but no posters.

## Playing

| Method | Path | Body |
| --- | --- | --- |
| POST | `/api/play` | `{ "movie": key }`, or `{ "show": key }` to carry on, or `{ "show": key, "season": 2, "episode": 3 }` |
| POST | `/api/command` | `{ "text": "play outlander season 2" }`, understood exactly like speech |
| POST | `/api/listen` | none: start listening, as the remote's colour keys do |
| GET | `/api/playback` | `{ now_playing }`, see below |
| POST | `/api/playback/{control}` | none. `control` is `pause`, `resume`, `toggle`, `stop`, `mute` or `unmute` |
| POST | `/api/playback/seek` | `{ "to": seconds }` or `{ "by": seconds }`, negative to go back |

`now_playing` is null or
`{ title, caption, movie, show, season, episode, position, duration, paused, muted, loading }`.
For a movie, `movie` is its key. For an episode, `show`, `season` and `episode`
are set.

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
    websocat ws://localhost:8090/api/events
