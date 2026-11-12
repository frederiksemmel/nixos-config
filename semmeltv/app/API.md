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
| GET | `/api/library` | `{ watching, forgotten, newest, shows, movies }`, lists of cards |
| POST | `/api/forgotten/dismiss` | `{ kind, key }` → `{ message, ok }` |
| POST | `/api/watched` | `{ kind, key, watched, season?, episode? }` → `{ message, ok, watched }` |
| GET | `/api/search?q=TEXT` | `{ results }`: cards ranked by how well the title matches, prefixes first, typos forgiven |
| GET | `/api/movies/{key}` | a card plus `details`, `runtime_minutes`, `stopped_at`, `duration`, `tags`, `subtitles`, `about`, `last_played` and `last_played_label` |
| GET | `/api/shows/{key}` | a card plus `next` (where playing carries on), `seasons` (each with `episodes`), `about`, `last_played` and `last_played_label` |

A card is
`{ kind: "movie" | "show", key, title, years, subtitle, poster, backdrop, ratings, finished }`.
`finished` is true for a film that played to its end, stopped in its credits
or was marked as watched, and for a series whose last episode did.
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
`stopped_at`, `duration` and `subtitles`. `band` names the episode's place on the heatmap
scale from its IMDb rating: `awesome` (9.0 and up), `great` (8.0), `good`
(7.0), `regular` (6.0), `bad` (5.0) or `garbage`, and is null without one. Keys are the TV's own: `dune-2021`,
`outlander`.

`watching` is Currently Watching: up to three series and three films, latest
first. `forgotten` is Started and Forgotten: every film and series that was
started and never finished and is not in `watching`, most relevant first. A
file is finished once it played to its end or stopped in its end credits; a
series once its last episode did. A finished file leaves `watching` (a series
moves on to its next episode), stays out of `forgotten`, and plays from the
start next time. Relevance is the share watched (of the film, or of
the series' regular episodes counting the started one in part) times
`(days + 7) / (days + 14)`, where `days` is the time since it last played, 90
when unknown. Both lists hold cards plus `season` and `episode` (where a series
carries on, else null), `detail` ("S2 E3", "1h 20m left" or both) and
`progress`, how far into that file it stopped, from 0 to 1.

The credits start where the file has a chapter whose title says "credits", else
where TheIntroDB says they start, else `(duration − 20 min) / 15` before the
end, at most 10 minutes: 1 minute of a 35-minute episode, 5 minutes of a
95-minute film, 10 minutes of anything from 2 h 50 m on. A file of 20 minutes
or less only finishes at its end. A post-credits scene changes nothing: a stop
anywhere after the credits start counts.

Marking a title watched or unwatched takes `kind` (`movie` or `show`), `key`
and `watched`, a boolean. A show with `season` and `episode` marks that
episode; a show without them marks the whole series. Marked watched, the file
is finished as if it had played to its end, and a series moves on past that
episode unless it was already further on; a whole series is marked by its last
episode. Marked unwatched, the file loses its finish and its resume point; an
episode marked unwatched becomes where the series resumes if the series was at
or past it; a film or whole series marked unwatched also leaves `watching`, and
a whole series forgets where it was. The answer's `watched` is the title's (or
episode's) state afterwards, so a button can flip without reloading. `ok` is
false, with 422, for a key or episode the library does not hold, and the status
is 400 for `season` without `episode`, or either with a film.

Dismissing takes a title (`kind` is `movie` or `show`) out of `forgotten` and
keeps its resume point. It is stored by the file playback would resume, so
it outlasts a change of the title's key, and the title comes back once it plays
again. `ok` is
false, with 422, for a key the library does not hold.

`subtitles` counts the subtitles a film or episode is known to have: the
subtitle files beside it in the library (see Subtitles), counted in the
background after each library load, plus the tracks inside the video, which
are known only once it has played. 0 means none are known, so a "Find
subtitles" offer fits.

An episode also has `watched` and `last_played`. `watched` is true once the
episode played to its end, or when it comes before the episode where the show
was last left. `next` is
`{ season, episode, label, title, stopped_at, duration }`: the episode a play
of the show carries on with, and where in it.

`last_played` is when the title last played, in Unix seconds, or null:
the latest of its files' plays and its Currently Watching entry.
`last_played_label` says the same in the TV's local time: "today 21:14",
"yesterday 21:14", "Saturday 20:00" within a week, "20 Oct 20:00" this year,
and "5 Oct 2025" before that.

`about` is what TMDB and MDBList say about the title, fetched when it is
imported:

    { tagline, plot, genres, original_title, language, released,
      first_aired, last_aired, seasons, episodes, networks, directors,
      creators, certificate: { country, rating }, trailer, imdb, tmdb,
      cast: [ { name, character, photo } ],
      scores: { metacritic, letterboxd, ebert } }

Any field can be null, and lists can be empty. A film has `released` and
`directors`; a show has `first_aired`, `last_aired`, `seasons` and `episodes`
(as TMDB counts them, not as the library holds them), `networks` and
`creators`. `original_title` is set only when it differs from the title.
`language` is the original language's English name. `certificate` is the age
rating in Switzerland, else Germany, else the US; `rating` is the country's
own label, such as `12` or `PG-13`. `trailer`, `imdb` and `tmdb` are links.
`cast` is the main cast, up to ten, in billing order for a film and by
episode count for a show; `photo` is an artwork path or null.
`scores.letterboxd` is out of 5 and `scores.ebert` (Roger Ebert) out of 4.

## Artwork

| Method | Path |
| --- | --- |
| GET | `/api/movies/{key}/poster` |
| GET | `/api/movies/{key}/backdrop` |
| GET | `/api/shows/{key}/poster` |
| GET | `/api/shows/{key}/backdrop` |
| GET | `/api/shows/{key}/episodes/{season}/{episode}/still` |
| GET | `/api/people/{person}/photo` |

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
| POST | `/api/playback/speed` | `{ "speed": 1.25 }`: 0.5 to 2, in steps of 0.05. Every video starts at 1 |

`now_playing` is null or
`{ title, caption, movie, show, season, episode, position, duration, paused, muted, speed, subtitle, subtitle_delay, audio, loading, jump }`.
`subtitle` is the id of the subtitle track that shows, or null when none does.
`subtitle_delay` is the subtitle delay in seconds, positive to show them later. `audio` is the playing
audio track's id. They follow what mpv reports, so a change made by the remote, by voice or from the web
reaches every event listener.
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

GET `/api/tv` answers `{ "kind": "samsung", "volume": true, "pairing": true, "paired": false }`:
which TV it is and what it can do. `paired` says whether a TV that pairs has
given SemmelTV a token, and is null for a TV that doesn't pair. `kind` is null while the TV can't be reached
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
| POST | `/api/subtitles/delay` | `{ "delay": seconds }`: show subtitles that much later, negative for earlier |
| POST | `/api/subtitles/fetch` | `{ "movie": key }` or `{ "show": key, "season": 2, "episode": 3 }`, and optionally `"language": "de"` |
| POST | `/api/audio/track` | `{ "track": id }`: play that audio track |

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

`playing` is null when nothing plays, or
`{ visible, delay, track, tracks, audio, audio_tracks }`. `delay` is the
subtitle delay in seconds. `track` is the selected
subtitle track's id or null, and `visible` says whether it shows. `audio` is
the playing audio track's id, and `audio_tracks` lists the audio tracks in the
same shape as `tracks`. Each track is
`{ id, language, title, label, forced, hearing_impaired, external }`, where
`label` is the language's English name, or the title when the language is
unknown.

The track and toggle requests answer like the playing requests above, for
example `{ "message": "Subtitles: English", "ok": true }`, and the TV shows
the same message on screen. They refuse with "Nothing is playing" or "No
subtitles in this file".

### Subtitle files in the library

Every video starts with the subtitle files that sit beside it in the library,
each as its own track named after its language:

- files next to the video whose name starts with the video's name, such as
  `Up.2009.1080p.en.srt` beside `Up.2009.1080p.mkv`
- for a video alone in its folder, any subtitle file there and in its `Subs/`
  folder, such as `Subs/13_English.srt`
- for an episode, the files in `Subs/<the episode's file name>/`

The language comes from the file name: a code (`en`, `eng`, `en-US`) or an
English name (`English`, `Bokmal`). `SDH`, `HI` or `CC` in the name marks the
track `hearing_impaired`, and `forced` marks it `forced`, as does being less
than a fifth of the size of the largest file in the same language. Formats are
`.srt`, `.ass`, `.ssa` and `.vtt`. When subtitles are set to show, the
preferred language among all tracks is chosen once the files are loaded.

### Delay

`POST /api/subtitles/delay` sets the delay of what plays, rounded to 0.1 s
and clamped to ±60 s, and answers like the track request, for example
"Subtitle delay +0.5 s". The TV remembers it per video file, in
`subtitle-files.json` beside `subtitles.json`, and starts the file with it
next time. A delay of 0 forgets it.

### Finding subtitles

`POST /api/subtitles/fetch` looks up subtitles for a film or episode on
OpenSubtitles.org, which needs no account, and saves the best one in the
library as `<video name>.<language>.srt` beside the video, so every later
start shows it. The language is the one asked for, else the first preferred
language. The search goes by the video's OpenSubtitles hash (its size plus the
first and last 64 KiB), which finds subtitles timed for this exact file, and
by the IMDb id, with season and episode for a series. Among the results, a
hash match comes first, then the release name closest to the video's file
name; files split over several CDs, rated bad, machine translated or in other
encodings than UTF-8 or Windows-1252 are skipped, and advert cues are removed.

It takes a few seconds and answers only when done:

    { "message": "English subtitles added", "file": "Movies/Up (2009)/Up.2009.1080p.en.srt",
      "language": "en", "label": "English", "release": "Up.2009.1080p.BluRay.x264",
      "matched": "hash" | "release" | "title", "shown": true }

`matched` says how well the subtitles should fit: `hash` is timed for this
very file, `release` for a release with a close name, `title` only for the
film or episode, so its timing may need the delay. When the video is playing,
the subtitles are added to it at once and shown, and `shown` is true. Errors
are `{ "error": "..." }` with a sentence to show as it is: 404 for an unknown
title, 422 for "No English subtitles found", a title without an IMDb id or a
failed download, and 503 when the TV has no downloads service, which does the
lookup and the upload to pCloud.

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

On a TV with headphones it sends the audio state right after the playback, and
again whenever it changes, with the same body as GET `/api/audio`:

    { "type": "audio", "output": "tv", "headphones": { "name": "AirPods", "address": "34:0E:22:67:03:38", "state": "connecting" }, "pairing": null, "error": null }

## Audio

| Method | Path | Body |
| --- | --- | --- |
| GET | `/api/audio` | none |
| POST | `/api/audio` | `{ "output": "headphones" }` or `{ "output": "tv" }` |
| POST | `/api/audio/pair` | none |
| DELETE | `/api/audio/pair` | none |
| DELETE | `/api/audio/headphones` | none |
| POST | `/api/audio/delay` | `{ "by": 20 }` |

Where the sound goes: the TV over HDMI, or Bluetooth headphones paired with
SemmelTV. Every request answers the whole state:

    {
      "output": "tv",
      "headphones": { "name": "AirPods von Frederik", "address": "34:0E:22:67:03:38", "state": "connected", "volume": { "level": 80, "muted": false } },
      "pairing": null,
      "error": null
    }

- `output` is where the sound is now, `tv` or `headphones`.
- `headphones` is null while none are paired. `name` is "AirPods" for Apple
  headphones (Bluetooth vendor 0x004C in BlueZ's Modalias), and otherwise
  their Bluetooth name without a " - Find My" ending; `state` is `disconnected`, `connecting` or `connected`. `volume` is
  their own volume while they are connected, `{ "level": 0–100, "muted": bool }`,
  and null otherwise; it follows changes made on the headphones too.
  `audio_delay` is the lip-sync nudge for the headphones in milliseconds while
  they are connected (positive: the sound comes later), and null otherwise.
- `pairing` is null, or `{ "state": "scanning" | "pairing" | "paired" |
  "failed", "message": "..." }` with a sentence to show as it is.
- `error` is a sentence about the last switch that went wrong, or null.

All of them answer 404 on a TV without Bluetooth (`SEMMELTV_HEADPHONES` unset;
`services.semmeltv.headphones.enable` sets it to `bluetooth`), so a client
hides its switch. The state also goes out on the event stream whenever it
changes.

### Switching

POST `/api/audio` answers at once and clears `error`. Choosing `headphones`
while they are disconnected answers with state `connecting`, runs
`bluetoothctl connect`, and moves the sound once BlueALSA offers their A2DP
sink. Headphones already connected take the sound at once. The rest comes on
the event stream:

- connected: `output` becomes `headphones`, `state` `connected`.
- not connected within about 30 seconds: `output` stays `tv`, `state` goes
  back to `disconnected`, and `error` is "AirPods didn't connect. Open the case
  near the TV and try again."
- the headphones drop later (case closed, out of range): `output` goes back to
  `tv` on its own and `error` is "AirPods disconnected. The sound is back on
  the TV."
- none paired: `error` is "No headphones are paired. Pair them in Settings."

Choosing `tv` moves the sound back and disconnects the headphones, so a phone
can take them again. Headphones that connect on their own don't take the
sound; they only show as `connected`.

A film moves without stopping: the app sets mpv's `audio-device` to
`alsa/bluealsa:DEV=<address>,PROFILE=a2dp`, or back to `auto` for HDMI, and the
TV shows "Sound on AirPods" or "Sound on the TV". A film started while the
headphones have the sound starts on them. BlueALSA counts the headphones'
reported delay in the ALSA delay mpv reads, so mpv keeps the picture in sync.
`volume-up` and `volume-down` under `/api/tv` always set the TV's volume; the
volume keys follow the sound with `/api/volume` below.

### Lip sync

The headphones report their own latency over A2DP delay reporting (AirPods:
150 ms with AAC), BlueALSA adds it to the ALSA delay mpv reads, and mpv holds
the picture back to match. When the sound still comes a little early or late,
POST `/api/audio/delay` with `{ "by": milliseconds }` moves it: positive makes
the sound later, negative earlier, up to 500 ms either way. It sets BlueALSA's
delay adjustment for these headphones and their current codec (with the sign
turned, since a smaller reported delay lets the picture go sooner), which
BlueALSA keeps in `/var/lib/bluealsa` across reboots. The delay BlueALSA
reports to mpv changes at once, during playback too. It answers the audio state, or 409 when the
headphones aren't connected.

### Volume

| Method | Path | Body |
| --- | --- | --- |
| POST | `/api/volume/up` | none |
| POST | `/api/volume/down` | none |
| POST | `/api/volume/mute` | none |

The volume keys go where the sound is. While `output` is `headphones`, they
set the headphones' own volume over AVRCP absolute volume (BlueALSA's
`--a2dp-volume`, through `bluealsa-cli volume` and `mute`), so the level is
the one the headphones and a phone would show: 16 steps over BlueALSA's
0–127, and up or down also unmutes. They answer `{ "message": "AirPods volume
56%", "ok": true }`, "AirPods muted" or "AirPods sound on", or 422 with
"AirPods didn't take the volume change". The new level also goes out as
`headphones.volume` in the audio state.

Otherwise they do what `/api/tv/volume-up`, `volume-down` and `mute` do, with
the same answers. The app doesn't act on CEC volume keys from the TV's
remote.


### Pairing

POST `/api/audio/pair` starts a 60 second Bluetooth scan and answers at once
with `pairing.state` `scanning` and the message "Open the case near the TV and
hold the button on its back until the light flashes white." It answers 409
while a pairing is running. The first headphones that show up (Bluetooth icon
`audio-headphones` or `audio-headset`) are paired and trusted with
`bluetoothctl`, then connected without taking the sound; `pairing.state`
becomes `pairing`, then `paired` with `headphones` filled in, or `failed` with
a message saying what to do. The adapter is pairable only during the pair, and
a pair that leaves no link key (BlueZ "Bonded: no") counts as failed. New
headphones replace the ones paired before; the same ones found again are
forgotten and paired once the scan sees them again. Only bonded headphones
count as paired, and BlueZ keeps them in `/var/lib/bluetooth`, so they survive
reboots.

BlueZ's own reconnecting is off (`ReconnectAttempts=0`), so the TV connects to
the headphones only when asked to, and never fights a phone for them.

DELETE `/api/audio/pair` stops a scan that hasn't found anything yet.
DELETE `/api/audio/headphones` moves the sound to the TV and forgets the
headphones (`bluetoothctl remove`).

## Where it plays

One thing plays at a time: on the TV or on one phone. The app keeps that as
the screen, and every front end shows what it says.

| Method | Path | Answer |
| --- | --- | --- |
| GET | `/api/screen` | the screen, see below |
| POST | `/api/screen` | a command, see below → `{ message, ok }` |

The screen is `{ kind, device, play, notice }`. `kind` is `tv` (Play goes to
the TV, whether or not it plays), `moving` (the TV is paused while a phone
starts the same video at the same place) or `phone` (Play goes to `device`).
`device` is `{ id, name }`, a browser's own random id and a name such as
"iPhone". `play` is null or what the phone plays: `{ state, kind, movie, show,
season, episode, name, caption, position, session, paused }`, where `state` is
`moving`, `starting` or `playing`. `notice` is the last thing worth saying,
`{ id, text, tone }` with `tone` `quiet` or `error`; a new `id` means a new
notice.

Commands carry `action` and the sending `device`:

- `{ "action": "flip", "to": "phone" }`: the screen moves to this phone. If
  the TV plays, it pauses, and once the phone has started the same video at
  the same place the TV stops the usual way, keeping its place. If the phone
  doesn't start within 20 s, or reports a failure, the TV carries on.
- `{ "action": "flip", "to": "tv", "position": 1234.5 }`: the screen moves to
  the TV. If a phone plays, the TV plays the same video from `position` (else
  from the phone's last report), turning the TV on.
- `{ "action": "play", "movie": key }` or `{ "action": "play", "show": key,
  "season"?, "episode"? }`: this phone plays it, from where it last stopped.
  A TV that plays stops. Any other phone loses the screen.
- `{ "action": "failed", "reason": text }`: this phone could not play.

A phone that starts, plays or stops reports it through the stream's
`progress`, which moves the screen too. A phone that hasn't reported for 45 s
(asleep, gone) stops being played on, keeping its place; one that doesn't
start within 30 s likewise. Anything the TV itself plays, from the remote,
voice or `/api/play`, takes the screen back to the TV.

The event stream sends `{ "type": "screen", "screen": {...} }` when it
connects and whenever the screen changes.

## Watching on a phone

These stream a film or episode to the browser that asks, as HLS with fMP4
segments, instead of playing it on the TV. They answer only requests that came
over the tailnet: `tailscale serve` adds `Tailscale-User-Login` to those, and
adds `Tailscale-Funnel-Request` to requests from the internet. Anything else
gets 403 with `{ "error": "Watching on this phone needs Tailscale on" }`.

| Method | Path | Answer |
| --- | --- | --- |
| GET | `/api/stream/access` | `{ ok, user }`: 200 on the tailnet, 403 otherwise |
| POST | `/api/stream/open` | `{ movie }` or `{ show, season?, episode? }`, plus `client`, `device?`, `audio?`, `start?`, `replaces?` → a session |
| GET | `/api/stream/{id}/main.m3u8` | the master playlist, to give the `<video>` or hls.js |
| GET | `/api/stream/{id}/marks` | `{ segments, credits }`: TheIntroDB's recap and intro, and where the credits start |
| POST | `/api/stream/{id}/progress` | `{ position, played, state }` → `{ message, ok }` |
| DELETE | `/api/stream/{id}` | ends the session |

`client` says what the browser decodes: `{ hevc, eac3, ac3 }`, all booleans.
The video is never converted, since the Pi has no video encoder: H.264 (8-bit)
and HEVC are copied, and a title in anything else is refused with 422 and a
reason, as is HEVC for a browser without `hevc`. The audio is copied when the
browser plays it (AAC always; E-AC-3 and AC-3 when `client` says so), and
otherwise converted to stereo AAC. `audio` picks an audio track by its `index`
in the answer, else the file's default track plays. A show without `season`
and `episode` plays where it carries on. `start` is in seconds; without it,
the video starts where the TV or a phone last stopped it, or at the beginning
once it was finished or stopped in its credits. `replaces` ends that session
first, for the next episode or another audio track.

At most three sessions play at once, and only one while the TV plays; the
others get 429. Three minutes after its last request a session stops its
ffmpeg and drops its segments, and no longer counts towards that limit; it
picks up again on the next request, and is forgotten after 12 hours.

The answer is

    { id, playlist, kind, key, title, caption, season, episode, poster,
      backdrop, start, duration, audio, audio_index, converted_audio,
      subtitles, subtitle_delay, credits, next, bitrate, heavy }

`audio` lists the tracks as `{ index, codec, channels, language, name,
default }`. `subtitles` lists the subtitle files beside the video, then the
tracks inside it, as `{ id, name, language, forced, default, supported }`.
Picture subtitles (PGS, VobSub) are listed with `supported` false: they show
only on the TV. The others are WebVTT renditions in the master playlist,
moved by the file's subtitle delay. `default` is the first one in the
subtitle languages when subtitles are on. `next` is the episode after it,
`{ season, episode, label, title, still }`, or null. `heavy` is true above
8 Mbit/s, a hint for mobile data.

The playlist cuts the video at its keyframes, read from the Matroska Cues or
the MP4 sample table, about every 6 seconds, so any point can be sought at
once. ffmpeg copies the video from the segment asked for, keeps up to a
minute ahead of the player and pauses beyond that. A file without a keyframe
index is refused.

`progress` records the place exactly as the TV does: `state` is `start`,
`playing`, `paused`, `stopped` or `ended`, `position` is in seconds and
`played` is the seconds watched in this session. It moves Currently Watching
and the show's place, finishes the file when it stops in the credits or ends,
and otherwise writes the position into the file's mpv resume entry, so the TV
carries on from there. While the TV plays the same file, `ok` is false and
nothing changes.

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
| POST | `/api/downloads/subtitles` | `{ video, imdb, season, episode, language, hash, size }` | `{ file, language, release, matched }` |

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

`/api/downloads/subtitles` is what `/api/subtitles/fetch` calls: `video` is
the path under the library, `hash` and `size` the video's OpenSubtitles hash
and size or null. It saves the subtitles beside the video on pCloud, refreshes
that folder on the TV's mount, and answers with the saved file's path, or 422
and `{ error }`.

## Trying it

    curl -s localhost:8090/api/search?q=lotus | jq '.results[].title'
    curl -s -X POST localhost:8090/api/play -d '{"show":"the-white-lotus"}' -H 'content-type: application/json'
    curl -s -X POST localhost:8090/api/playback/toggle
    curl -s -X PUT localhost:8090/api/subtitles -d '{"size":1.3,"background":0.5}' -H 'content-type: application/json'
    websocat ws://localhost:8090/api/events
    curl -s -X POST localhost:8092/api/downloads/search -d '{"text":"outlander season 8"}' -H 'content-type: application/json' | jq '.choices[0].facts'
