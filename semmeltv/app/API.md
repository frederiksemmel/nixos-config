# SemmelTV HTTP API

The app serves this API when `SEMMELTV_API` (or `--api`) names an address,
normally `127.0.0.1:8090`. It runs on its own thread, and the TV keeps working
if the address cannot be bound. Bind it to localhost and let `tailscale serve`
and nginx expose it; on semmeltv5 it is at
`https://semmeltv5.cuscus-exponential.ts.net/api/...` and, at home,
`https://semmeltv5.semmeltv.ch/api/...`. Every request needs a signed-in
session, see Sign-in.

Requests that change something on the TV take the same path as the remote. The
app turns them into a request to its core, which resolves it against the library
and progress and performs it, without waiting for the TV's screen. They answer
with what happened:

    { "message": "Playing Outlander · season 2, episode 1", "ok": true }

`ok` is false, with status 422, when the TV refused, for example "Nothing is
playing". The status is 503 when the TV did not answer within 5 s. Other errors
look like `{ "error": "..." }`, with 400 or 404.

## Sign-in

`SEMMELTV_ORIGINS` lists the addresses people sign in on, such as
`https://semmeltv5.cuscus-exponential.ts.net,https://semmeltv5.semmeltv.ch`.
With it set, every request passes one gate in the app before anything else:

- It needs the `semmeltv_session` cookie of a live session, else 401
  `{ "error": "Not signed in" }`. Open without one: the sign-in requests below,
  `/api/log`, and a phone stream's playlist, segments, marks and progress,
  which its random session id already guards. Opening a stream does need one.
- A request that changes something (any method but GET, HEAD and OPTIONS) with
  an `Origin` outside the list gets 403, whatever its cookie.
- Storage, usage, `/api/tv/pair`, `/api/auth/members/...`, stopping a download
  (`/api/downloads/jobs/{id}/stop`), pCloud's space and deleting a title
  (`/api/downloads/pcloud/...`) and reading feedback need an admin, else
  403 "Only an admin can do that".
- Controlling the TV needs the `control-tv` capability, else 403 "Only someone
  allowed to control this TV can do that": `/api/play`, `/api/command`,
  `/api/volume/*`, any change under
  `/api/playback/*`, `/api/tv/*` and `/api/tv-profile`, `/api/audio/track`
  and `/api/subtitles/track|toggle|delay`. Reading them, dialogue mode and the
  subtitle style stay open to everyone signed in. On `POST /api/screen`,
  someone without it can't `flip`, and their `play` is taken only while
  nothing else plays (the TV plays nothing and no other phone plays), else 422
  "Something else plays now".
- The TV's Bluetooth headphones need `control-tv` too: `POST /api/audio`,
  `/api/audio/pair`, `/api/audio/headphones` and `/api/audio/delay`. `GET
  /api/audio` stays open. Headphones are the paired profile's own: `{ "output":
  "headphones" }` takes the caller's, `DELETE /api/audio/headphones` forgets the
  caller's, and `/api/audio/delay` on someone else's answers 409 "These
  headphones aren't yours".
- Downloading needs the `download` capability, else 403 "Only someone allowed
  to download on this TV can do that": the rest of `/api/downloads/*`. The
  web app checks the same capability before it calls the downloads service.
- `GET /api/events` also takes `Authorization: Bearer <token>`, the token in
  `SEMMELTV_SERVICE_TOKEN_FILE`, which is how the web app's server follows the
  TV. The token opens nothing else.
- The `Tailscale-User-*` and `Tailscale-Funnel-Request` headers are never
  read, so a route from the public internet can't claim an identity with them.

The app then knows the profile behind each request and answers with that
profile's state; see Profiles in the README. Without `SEMMELTV_ORIGINS` there
is no gate and every request acts as profile 1, which is for development only.

A session lasts 7 days from its last use: a request more than an hour after the
previous one moves its end and sends the cookie again. The cookie is
`HttpOnly; Secure; SameSite=Lax` and holds every session of the browser,
separated by dots, the active one first, so a shared iPad switches between
profiles without signing in again. Switching *to* an admin always asks for a
passkey or password, and switching away from one ends the admin's session.
Tokens and invite links are stored only as SHA-256 hashes; passwords as
Argon2id.

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/auth/choices` | | `{ profiles }`: each person with `password`, `passkey` (one for this address), `active` and `signed_in` (on this browser) |
| GET | `/api/auth/me` | | `{ person, others, passkeys, password }` |
| POST | `/api/auth/password` | `{ profile, password }` | `{ ok, person }` and the cookie |
| POST | `/api/auth/passkey/start` | `{ profile }` | `{ ceremony, options }` for `navigator.credentials.get` |
| POST | `/api/auth/passkey/finish` | `{ ceremony, credential }` | `{ ok, person }` and the cookie |
| POST | `/api/auth/switch` | `{ profile }` | `{ ok, person }`, or 401 with `sign_in: true` |
| POST | `/api/auth/sign-out` | | `{ ok, person }`: the next profile on this browser, or null |
| GET | `/api/auth/invite/{token}` | | `{ person }`, 404 for an unknown link, 410 for a used or expired one |
| POST | `/api/auth/invite/{token}` | | `{ ok, person }` and the cookie; the link is used up |
| POST | `/api/auth/account/password` | `{ password }` | `{ ok, message }` |
| POST | `/api/auth/passkeys/start` | | `{ ceremony, options }` for `navigator.credentials.create` |
| POST | `/api/auth/passkeys/finish` | `{ ceremony, credential }` | `{ ok, passkey }` |
| DELETE | `/api/auth/passkeys/{id}` | | `{ ok, message }` |
| GET | `/api/auth/members` | | admin: `{ members }`, each with `password`, `passkeys`, `sessions`, `last_seen_at`, `invite_expires_at` |
| POST | `/api/auth/members` | `{ name, role }` | admin: `{ ok, person }` |
| DELETE | `/api/auth/members/{id}` | | admin: the profile and all its state go |
| POST | `/api/auth/members/{id}/invite` | | admin: `{ ok, link, expires_in_days }` |
| POST | `/api/auth/members/{id}/sign-out` | | admin: ends all of that person's sessions |
| PUT | `/api/auth/members/{id}/capabilities/{capability}` | | admin: grants it, `{ ok, message, person }` |
| DELETE | `/api/auth/members/{id}/capabilities/{capability}` | | admin: takes it back, `{ ok, message, person }` |

A person is `{ id, name, role, color, capabilities }`, `role` being `admin` or
`viewer`. `capabilities` lists what the person may do on this TV beyond
browsing and watching on their own device: `control-tv` (which includes
pairing and using their own headphones) and `download`. An
admin has every capability; a viewer only those granted, none at first. Each
TV keeps its own grants.
Passkeys use WebAuthn with user verification (Face ID, Touch ID, a screen
lock); a passkey belongs to the address it was made on, so the tailnet and the
home address each need their own. `credential` is the browser's
`PublicKeyCredential` with its binary fields as unpadded base64url. A ceremony
lasts 5 minutes and is used up by any attempt. Five wrong passwords lock that
profile's password for a minute, doubling up to an hour.

An invite link is good for 7 days and one use. Opening it shows a button;
only the POST signs in, so a messenger's link preview can't use it up. The web
app then replaces the invite in the browser's history with Home, so Back never
returns to a used link, and sends someone already signed in Home when they open
a used, expired or their own invite again. A new
invite for the same person replaces the old one. `semmeltv invite NAME` makes
one from the command line, with `SEMMELTV_DB` and `SEMMELTV_ORIGINS` set, for
the first admin or one who is locked out.

| Method | Path | Answer |
| --- | --- | --- |
| GET | `/api/tv-profile` | `{ profile, name, mine }`: whose state the TV's own screen shows |
| POST | `/api/tv-profile` | makes it the caller's, `{ message, ok }` |

## Library

| Method | Path | Answer |
| --- | --- | --- |
| GET | `/api/library` | `{ watching, forgotten, newest, shows, movies }`, lists of cards |
| POST | `/api/forgotten/dismiss` | `{ kind, key }` → `{ message, ok }` |
| POST | `/api/watched` | `{ kind, key, watched, season?, episode? }` → `{ message, ok, watched }` |
| GET | `/api/search?q=TEXT` | `{ results }`: cards ranked by how well the title matches, prefixes first, typos forgiven; then titles whose director, creator, cast, character, original title, network or genre match, each with `because` (like `Director: Denis Villeneuve`) |
| GET | `/api/movies/{key}` | a card plus `details`, `runtime_minutes`, `stopped_at`, `duration`, `tags`, `subtitles`, `about`, `last_played` and `last_played_label` |
| GET | `/api/shows/{key}` | a card plus `next` (where playing carries on), `seasons` (each with `episodes`), `about`, `last_played`, `last_played_label` and `together` |
| PUT | `/api/shows/{key}/together/{profile}` | joins your group and theirs into one, where everyone watches with everyone: `{ message, ok }` |
| DELETE | `/api/shows/{key}/together/{profile}` | takes that person out of the group, for everyone; your own id leaves it: `{ message, ok }` |
| POST | `/api/shows/{key}/together/{profile}/catch-up` | moves your place in the show to theirs: `{ message, ok }`, 422 when you don't watch it together or they haven't started |

Watching together is a group per show, of any size: adding someone who
already watches it with others brings them all in. `together` is `{ me,
partners, others }`, `me` being your own id. Each partner is `{ id, name, color,
spot, with_me }`: `spot` is `{ state: "not_started" | "finished" }` or
`{ state: "at", season, episode, label, title, stopped_at, duration }`, where
they carry on, and `with_me` is true when that is your place too (the same
episode, less than a minute apart). `others` lists everyone else as `{ id,
name, color }`. Catching up copies their episode and time into your progress,
keeps your own audio and subtitle choice, and puts the show first in your
Continue Watching.

Playing an episode, on the TV or a phone, pauses once where someone in your
group for that show stopped in it, further on than where you started; people
less than a minute apart share one pause. The TV shows "Paused where Mami
stopped · Press OK to watch on", a phone "Paused where Mami stopped"; a seek
past the place does not pause.

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
      tmdb_id, cast: [ { name, character, photo } ],
      scores: { metacritic, letterboxd, ebert } }

Any field can be null, and lists can be empty. A film has `released` and
`directors`; a show has `first_aired`, `last_aired`, `seasons` and `episodes`
(as TMDB counts them, not as the library holds them), `networks` and
`creators`. `original_title` is set only when it differs from the title.
`language` is the original language's English name. `certificate` is the age
rating in Switzerland, else Germany, else the US; `rating` is the country's
own label, such as `12` or `PG-13`. `trailer`, `imdb` and `tmdb` are links;
`tmdb_id` is the bare TMDB id.
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
| POST | `/api/command` | `{ "text": "play outlander season 2" }`, a sentence in English, German or Spanish, as typed into the web app's search box |
| GET | `/api/playback` | `{ now_playing }`, see below |
| POST | `/api/playback/{control}` | none. `control` is `pause`, `resume`, `toggle`, `stop`, `mute`, `unmute`, `skip-segment` or `hide-skip`. `skip-segment` does what OK does while the TV shows Skip intro, Skip recap or Play next, `hide-skip` what Back does: it hides that button until the file plays again. Both are refused with 422 while the TV shows none |
| POST | `/api/playback/seek` | `{ "to": seconds }` or `{ "by": seconds }`, negative to go back |
| POST | `/api/playback/skip` | `{ "direction": "back" }` or `"forward"`: a press of the remote's left or right key, so presses in a row skip further |
| POST | `/api/playback/speed` | `{ "speed": 1.25 }`: 0.5 to 2, in steps of 0.05. Every video starts at 1 |

`now_playing` is null or
`{ title, caption, kind, …, position, duration, paused, muted, speed, subtitle, subtitle_delay, audio, loading, jump, skip }`.
`subtitle` is the id of the subtitle track that shows, or null when none does.
`subtitle_delay` is the subtitle delay in seconds, positive to show them later. `audio` is the playing
audio track's id. They follow what mpv reports, so a change made by the remote or from the web
reaches every event listener.
`kind` says what plays: `"movie"` with `movie`, its key; `"episode"` with
`show`, `season` and `episode`; or `"file"`, a file outside the library, with
none of them. `jump` is the last skip in seconds, negative going back, while a run of
skips can still continue, and null otherwise.
`skip` is `{ kind, label }` while the TV shows its prompt to skip the intro or the recap, with
`kind` `"intro"` or `"recap"` and `label` the prompt's text, such as `"Skip intro"`, and null
otherwise.

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

`on`, `off` and `semmeltv` do what the same sentences to `/api/command` do. `on` turns the
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
the database's `tv_token` setting and sent along when pairing again. Pairing is
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
its database (`/var/lib/semmeltv/semmeltv.db` on a Pi), and every video starts
with them. A PUT also restyles what is playing at once.
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
its database, and starts the file with it next time. A delay of 0 forgets it.

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

## Usage

`GET /api/usage?days=30` sums up who watched what over the last `days` days
(1 to 3660, 30 when left out), from the same `usage` table `semmeltv stats`
reads. It needs an admin:

    { "days": 30,
      "totals": { "tv", "phone", "plays", "people", "titles" },
      "span": "day" | "week",
      "buckets": [ { "label": "Tue 17 Nov", "mark": "17", "tv", "phone" } ],
      "hours": [ [ 24 numbers ] × 7 ],
      "people": [ { "name", "tv", "phone", "plays", "days", "sign_ins",
                    "favourite", "last_seen", "devices" } ],
      "titles": [ { "title", "seconds", "plays", "people": [ { "name", "seconds" } ] } ],
      "plays": [ { "at", "when", "name", "device", "tv", "title", "seconds" } ] }

Watched time is in seconds; `tv` counts plays on the TV, `phone` plays on a
phone or laptop. A play's time is spread over the hours it covered from its
start, in the TV's local time, so a film from 23:30 counts on both days.
`buckets` are one per day up to 62 days, else one per week from Monday;
`label` names the bucket and `mark` is a short axis label. `hours` is
Monday first, hour 0 first. `people` is everyone who signed in, came back or
played, most watched first; `days` counts the days they did any of that,
`favourite` is the title they watched longest and `last_seen` a local time
like "yesterday 20:15". `titles` are the 10 most watched, an episode counted
under its series. `plays` are the newest 30, `when` like `last_seen`.

## Notifications

The TV sends Web Push notifications to the browsers people turn them on in
(Settings → Notifications; on an iPhone, SemmelTV opened from the Home
Screen). Each push is `{ title, body, url, tag }`; the web app's service worker
`/sw.js` shows it and opens `url` when tapped.

| Method | Path | Does |
| --- | --- | --- |
| GET | `/api/push` | `{ key }`: the TV's public VAPID key, base64url, to subscribe with |
| POST | `/api/push/subscriptions` | keeps a browser's `PushSubscription.toJSON()` (`{ endpoint, keys: { p256dh, auth } }`) for you; "Notifications are on" goes to it only when it is new for you, not when Settings checks in again; 400 unless `endpoint` is an https push service |
| DELETE | `/api/push/subscriptions` | `{ endpoint }`: no more pushes to that browser |

What is sent:
- **Carries on**: when someone starts an episode of a show, on the TV or a
  phone, everyone in their watch-together group for it hears "Mami carries on
  with Dark", at most once in three hours per person and show.
- **Added**: when the library gains a film or episodes that someone downloaded
  in the last two days, every admin but that person hears "Mami added Arrival
  (2016)" or "Mami added 3 episodes of Dark": a film once in two days, more
  episodes of a show by the same person once in three hours.
- **Playing on the TV**: when the TV starts a film or episode, whoever it
  plays for hears "Playing on the TV · Arrival (2016)", so it sits on the
  phone's lock screen; its `url` ends in `#remote`, which opens the remote.
  At most once in three hours per person and film or show, since iOS stacks
  notices of the same tag rather than replacing them. A tap with SemmelTV
  already open posts `"semmeltv:remote"` to that window instead of navigating.

What each person was told, and when, is kept in the `push_notices` table, so
the quiet times hold across a restart of the TV app.

The VAPID key is made on first use and kept as the `push_key` setting; a reset
`semmeltv.db` makes a new one, and browsers subscribed to the old one are
dropped the next time they open Settings.

## Event stream

`GET /api/events` upgrades to a WebSocket that sends one JSON text message per
event. It starts with the current playback:

    { "type": "playback", "now_playing": { ... } | null }

After that it sends playback whenever it changes, at most about once a second
while playing, and what a download asked for through `/api/command` found,
as the TV shows it; `heading` may be empty, and `ok` is false when nothing can
be downloaded:

    { "type": "notice", "heading": "Dark season 2", "status": "1080p, 4.2 GB. Confirm on the phone's Downloads page.", "ok": true }

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
| GET | `/api/audio/dialogue` | none |
| POST | `/api/audio/dialogue` | `{ "on": true }` |

Where the sound goes: the TV over HDMI, or Bluetooth headphones paired with
SemmelTV. Every request answers the whole state:

    {
      "output": "tv",
      "headphones": { "name": "AirPods von Frederik", "address": "34:0E:22:67:03:38", "owners": [1], "state": "connected", "volume": { "level": 80, "muted": false } },
      "with_headphones": [1, 2],
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
  `headphones` are the ones the TV holds now, those of the profile on the TV
  unless someone took their own since; `owners` are the profile ids whose
  headphones these are.
- `with_headphones` lists the profile ids that have headphones paired with this
  TV, so a phone can offer someone their own while the TV holds another's.
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

GET `/api/audio/dialogue` answers `{ "on": false }`: whether the signed-in
profile has dialogue mode on. POST it with `{ "on": true }` to switch it; the
answer is an outcome such as `{ "message": "Dialogue mode on", "ok": true }`.
Dialogue mode works on every TV, with or without headphones.

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
| GET | `/api/screen?device=ID` | the screen, as the device `ID` reads it, see below |
| POST | `/api/screen` | a command, see below → `{ message, ok }` |

The screen is `{ kind, boot, notice }` plus what its kind carries. `kind` is
`tv` (Play goes to the TV, whether or not it plays; nothing more), `moving`
(the TV is paused while a phone starts the same video at the same place; with
`device` and `play`) or `phone` (Play goes to `device`; with `device` and
`play`, which may be null). `device` is `{ id, name }`, a browser's own random
id and a name such as "iPhone". `play` is what the phone plays,
`{ state, kind, movie | show season episode, name, caption, position }`, where
`state` is `moving` (only on a moving screen), `starting`, or `playing`, which
adds `session` and `paused`. `boot` names this run of the app: it changes
whenever the app restarts, and the screen then starts over on the TV with nothing
playing. `notice` is the last thing worth saying,
`{ id, text, tone }` with `tone` `quiet` or `error`; a new `id` means a new
notice.

`here` is what the asking device's player does, decided by the TV so the phone
only carries it out: `{ do: "open", kind, movie | show season episode, at }`
when the screen moves to this device or it plays there (open that title from
`at`, unless the player is on it already), `{ do: "hold" }` when the screen is
on this device but plays nothing (a video still playing goes on and reports, a
stopped one stays), and `{ do: "close" }` otherwise. Without `device` it is
always `close`. `/api/events?device=ID` sends each screen event with the same
`here` for its device.

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
- `{ "action": "resume", "movie": key, "position": 1234.5 }` (or `show`,
  `season`, `episode`): this phone plays it again from `position`, after the app
  restarted under it. It is taken only while nothing else plays: the screen is
  on the TV and the TV plays nothing, or this phone has the screen and plays
  nothing. Otherwise it answers 422 "Something else plays now".
- `{ "action": "stop" }`: this phone's player closed. What it was starting or
  playing stops; during a handoff to it the TV carries on at once.

A phone that starts, plays or stops reports it through the stream's
`progress`, which moves the screen too. A `playing` or `paused` report counts
only for the title the screen has the phone on, so a late report from the
episode before doesn't take the place of the next one. A phone that hasn't reported for 45 s
(asleep, gone) stops being played on, keeping its place; one that doesn't
start within 30 s likewise. Anything the TV itself plays, from the remote,
`/api/command` or `/api/play`, takes the screen back to the TV.

The event stream sends `{ "type": "screen", "screen": {...} }` when it
connects and whenever the screen changes.

## Watching on a phone

These stream a film or episode to the browser that asks, as HLS with fMP4
segments, instead of playing it on the TV.

| Method | Path | Answer |
| --- | --- | --- |
| POST | `/api/stream/open` | `{ movie }` or `{ show, season?, episode? }`, plus `client`, `device?`, `audio?`, `start?`, `replaces?` → a session |
| GET | `/api/stream/{id}/main.m3u8` | the master playlist, to give the `<video>` or hls.js |
| GET | `/api/stream/{id}/subtitles/{n}` | `{ cues: [{ start, end, text }], complete }` for subtitle `n` |
| GET | `/api/stream/{id}/marks` | `{ segments, credits, stops }`: TheIntroDB's recap and intro, where the credits start, and `stops`, each `{ at, said }`, where people you watch the series with stopped further on in this episode; the player pauses there once and shows `said` ("Paused where Mami stopped") |
| POST | `/api/stream/{id}/progress` | `{ position, played, state }` → `{ ok, kept_from }`, or `{ ok, reopen, kept_from }` |
| DELETE | `/api/stream/{id}` | ends the session |

`client` says what the browser decodes: `{ hevc, eac3, ac3 }`, all booleans.
The video is never converted, since the Pi has no video encoder: H.264 (8-bit)
and HEVC are copied, and a title in anything else is refused with 422 and a
reason, as is HEVC for a browser without `hevc`. The audio is copied when the
browser plays it (AAC always; E-AC-3 and AC-3 when `client` says so), and
otherwise converted to stereo AAC, through the dialogue-mode chain when the
profile has it on. `audio` picks an audio track by its `index` in the answer,
else the file's default track plays. A show without `season` and `episode`
plays where it carries on. `start` is in seconds; without it, the video starts
where the TV or a phone last stopped it, or at the beginning once it was
finished or stopped in its credits. `replaces` ends that session first, for the
next episode, another audio track or a seek. Opening a session also ends the
other sessions of the same `device`, such as the one a reloaded page left
behind.

A session is one ffmpeg run with ffmpeg's own HLS muxer: it copies the file
from the keyframe at or before `start` to the end, at twice the playback pace
after a minute's head start, into an EVENT playlist that lists each 6-second
segment once it is written and ends with `#EXT-X-ENDLIST`. The open answers
once the first segment is there. The segments carry the file's own times, and
the stream's clock starts at `offset`, the file time of its first frame, so a
player's position in the file is `offset` plus how far it has played. A seek
the playlist doesn't reach yet, or before `kept_from`, opens a new session there.
A session keeps its segments from 90 s before the newest one fetched, and
ffmpeg waits while it is more than three minutes past it; `kept_from` is the
file time from which the segments are still there.
When the profile switches dialogue mode while its sound is converted, the next
progress report answers `reopen`, and the phone opens the stream again where it
is.

At most eight sessions play at once, and at most three of them on films that
aren't downloaded to the TV, where the TV playing such a film counts as one;
the reasons and measurements are in [streaming](../docs/streaming.md). A
handoff doesn't count as the TV playing for the phone it moves to. An open past
a limit gets 429 with `{ error, full: { heading, message, question, watchers
} }`: `watchers` are the sessions whose end would make room, each
`{ seat: { kind: "stream", id } or { kind: "tv" }, profile, person, device, title, line, note?, pickable }`.
`question` is the sheet's line above them (whom to challenge, or how long the
asker's dough still rests), `line` the row's "device · title", `note` why they
can't be challenged, and `pickable` whether the asker can challenge them now.
Three minutes after its last request a session ends, stopping its
ffmpeg and deleting its segments; a phone that comes back opens it again.

The answer is

    { id, playlist, kind, key, …, title, caption, poster,
      backdrop, start, offset, duration, audio, audio_index, converted_audio,
      subtitles, subtitle_delay, credits, bitrate, heavy }

`kind` is `"movie"`, with only `key`, or `"show"`, with `key`, `season`,
`episode` and `next`.

`audio` lists the tracks as `{ index, codec, channels, language, name,
default }`. `subtitles` lists the subtitle files beside the video, then the
tracks inside it, as `{ id, name, language, forced, default, supported }`.
Picture subtitles (PGS, VobSub) are listed with `supported` false: they show
only on the TV. The phone adds the others' cues to the video itself, from
`/subtitles/{n}`: cues in file time, moved by the file's subtitle delay. A track
inside the video is written out by the same ffmpeg as it goes, so its answer
has the cues so far and `complete` false until the remux has finished; a
subtitle file beside the video is converted whole. `default` is the first one
in the subtitle languages when subtitles are on. An episode's `next` is the
episode after it, `{ season, episode, label, title, still }`, or null. `heavy`
is true above 8 Mbit/s, a hint for mobile data.

`progress` records the place exactly as the TV does: `state` is `start`,
`playing`, `paused`, `stopped` or `ended`, `position` is in seconds and
`played` is the seconds watched in this session. It moves Currently Watching
and the show's place, finishes the file when it stops in the credits or ends,
and otherwise writes the position into the file's mpv resume entry, so the TV
carries on from there. While the TV plays the same file, `ok` is false and
nothing changes. A `stopped` or `ended` report also ends the session.

## Bake-offs

When every place is taken, the 429's `full.watchers` can be challenged to a
bake-off for their place; the rules are in [streaming](../docs/streaming.md).

| Method | Path | Answer |
| --- | --- | --- |
| POST | `/api/challenges` | `{ seat, device? }`: the stream id of someone in `full.watchers`, or `"tv"` → `{ id }`, or 409 with the reason (resting, already in a bake-off) |
| POST | `/api/challenges/practice` | a practice bake against the Bäckermeister, which bakes at once → `{ id }` |
| GET | `/api/challenges/{id}` | the challenger's view, see below |
| DELETE | `/api/challenges/{id}` | the challenger gives up |
| POST | `/api/challenges/{id}/presses` | `{ input, presses }` → the view |
| GET | `/api/stream/{id}/challenge` | the view of the challenge on this stream, or 204 |
| POST | `/api/stream/{id}/challenge/accept` | bake → the view |
| POST | `/api/stream/{id}/challenge/drop-off` | stop watching instead; the stream ends |
| POST | `/api/stream/{id}/challenge/presses` | `{ input, presses }` → the view |

The challenger's calls need the challenger's sign-in; the defender's go through
the stream's id like the stream's other calls. `input` is `touch`, `mouse`,
`keys` or `remote`. `presses` holds three numbers or nulls: each Semmel's press
in milliseconds after its golden moment on that device's screen, or null where
it burnt. A view is `{ game, side, sheet, stage, challenger, defender, … }`, where
`game` is the game's name. `sheet` is what that side's screen shows, worded by
the server for the phone and the TV alike: `{ title, detail, note?, actions }`,
each action `{ label, strong, does }` with `does` one of `accept`, `drop-off`,
`give-up`, `watch`, `resume`, `again` or `close`. With `stage`

- `asked`: `title`, `answer_within_ms`;
- `baking`: `starts_in_ms`, `golden_ms` (three), `length_ms`, `bias_ms` (each
  input's lateness the oven takes off), `pressed`;
- `done`: `winner` (`challenger`, `defender` or null), `ending` (`baked`,
  `dropped-off`, `no-answer`, `withdrawn`, `seat-gone`) and `baked`, per side
  `{ crust, rise, total }` or null.

## Downloads

Downloads are a separate service, `semmeltv downloads`, normally on
`127.0.0.1:8092`, so a restart of the TV app never cuts a transfer short. Its
paths start with `/api/downloads` too.

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/downloads` | | `{ proposals, jobs, room, quota, free }` |
| GET | `/api/downloads/suggest?text=` | | `[ { kind, tmdb, title, year, poster, season, owned, credit? } ]`: TMDB's films and series for the text; when TMDB's best match is a person, their films and series too, with `credit` like `directed by Christopher Nolan` |
| POST | `/api/downloads/search` | `{ text }`, or `{ pick: { kind, tmdb, season, episode } }` | a proposal |
| POST | `/api/downloads/confirm` | `{ proposal, choice }` | `{ message, ok, job }` |
| POST | `/api/downloads/fetch` | `{ pick: { kind, tmdb, season, episode }, offer, mode }` | `{ message, ok, job }` |
| POST | `/api/downloads/dismiss` | `{ proposal }` | `{ message, ok }` |
| POST | `/api/downloads/jobs/{id}/stop` | | `{ message, ok, job }` |
| POST | `/api/downloads/subtitles` | `{ video, imdb, season, episode, language, hash, size }` | `{ file, language, release, matched }` |
| GET | `/api/downloads/shows/{tmdb}/missing` | | `[ { number, downloading } ]` |
| GET | `/api/downloads/pcloud` | | `{ space: { total, used, free }, titles: [ { kind, key, name, videos, bytes } ] }` |
| POST | `/api/downloads/pcloud/delete` | `{ kind, key }` | `{ message, ok }` |

`text` is what was said or typed, such as "outlander season 8". A proposal is
`{ id, asked, from, created, outcome, … }`, and `outcome` says what the rest is:
`refused` with a `message`; `question` with its `text`, up to four `options`
(`{ kind, tmdb, label }`) and the `season` and `episode` that were asked for,
all of which are sent back as `pick`; or `searched` with the `label`, the
`target` (TMDB and IMDb ids, season, aired episodes), a `message`, up to three
`choices` ranked by the scorer, each with `facts`, `why` and its torrents in
`parts`, `dropped`, the number of releases the checks removed, and `problems`
from the indexers. A search with no `choices` found nothing that passed. Nothing
downloads until a choice is confirmed. Proposals expire after 30 minutes.

`/api/downloads/fetch` starts a job for one release chosen elsewhere, with no
proposal and no scoring: `offer` is `{ name, size, seeders, peers, infohash,
magnet, indexer }`, a line of `semmeltv get search`. `mode` is `add`, refused
for a title already in the library, or `replace`, refused for one that is not.
A replacing job takes over the library entry of the film, or of each episode,
with its id, so watch progress stays, and moves the old video and the
subtitles named after it to `Replaced/` beside `Movies/` on pCloud. Every job
carries its `mode`. `semmeltv get search|fetch|jobs` is the command line for
these, run on the TV with the downloads service's environment.

A job moves through `checking` (reading the first torrent's files),
`downloading`, `uploading` (the last files to pCloud and into the library),
and ends `done`, `failed` or `stopped`. It fetches one file at a time, in
episode order: rqbit is asked for that one file only, and each finished file is
uploaded and imported while the next one downloads, so episode 1 plays while
the rest of the season is still coming. `placed` lists the library paths
already imported. A job of several torrents gives them to rqbit one after the
other. Each part reports its `phase`, `new` (not yet given to rqbit),
`checking` (rqbit verifies pieces it already has), `downloading` or `done`
(all its files fetched and the torrent paused), its `files` with their
`progress`, and `progress`, `total`, `speed`, `peers` and `uploaded`, the
bytes sent to peers. `room` is the space left for a new
download: the quota minus running downloads, and the free disk minus what they
still need and a reserve.

`/api/downloads/shows/{tmdb}/missing` lists the seasons TMDB has for a show
that have aired and of which the library holds no episode, with `downloading`
set while a job fetches that season. It asks TMDB at most every six hours per
show, and answers 502 and `{ error }` when TMDB fails.

`/api/downloads/pcloud` is pCloud's space, from `rclone about`, and every
title in the library with the bytes deleting it would free, biggest first; it
answers 502 and `{ error }` when pCloud does not. `kind` is `movie` or
`series` and `key` the title's key, as in `/api/storage`.
`/api/downloads/pcloud/delete` deletes a whole title: it takes the film or
the show with all its episodes out of library.db on pCloud, read and written
again when pCloud's copy changed meanwhile, as an import does, then deletes the
files on pCloud, which keeps them in its trash. A title's folder under
`Movies/` or `Series/` goes whole when no other title has a video in it,
else only its videos and the subtitles named after them; its artwork goes
unless another entry shares it. Then the TV refreshes its mount and copies
library.db again, and the other TV notices within 15 minutes.

`/api/downloads/subtitles` is what `/api/subtitles/fetch` calls: `video` is
the path under the library, `hash` and `size` the video's OpenSubtitles hash
and size or null. It saves the subtitles beside the video on pCloud, refreshes
that folder on the TV's mount, and answers with the saved file's path, or 422
and `{ error }`.

## Feedback

`POST /api/feedback` stores a report from anyone signed in:

```json
{
  "message": "The episode list jumped back to season 1",
  "page": "/shows/dark",
  "client": { "device": "iPhone 1a2b", "trail": [] },
  "screenshot": "data:image/jpeg;base64,…"
}
```

It needs `message` or `screenshot`; everything else is optional. `client`
is whatever the page sends. The web app sends its device, viewport and its
trail of the last 300 things it did: taps, route changes, API calls with status
and time, socket and video events, and errors. The TV cuts the trail from the
oldest end to fit 400 KB, adds its own snapshot, and answers 201
`{ "id": "20261115-131211-ab12" }`, or 422 `{ "error": … }`. The body may be
up to 6 MB.

Admins read them back:

- `GET /api/feedback`: `{ "dir": …, "reports": [summary, …] }`, the newest
  100. A summary has `id`, `at` (Unix seconds), `when` (UTC), `host`, `person`
  (`id`, `name`), `device`, `page`, `message`, `screenshot` and
  `tv_screenshot` (file names, or null).
- `GET /api/feedback/{id}`: the whole report, the summary plus `client` and
  `tv`.
- `GET /api/feedback/files/{name}`: a screenshot named in a summary.

### Deploy

The Feedback page shows what main has that this TV doesn't run yet, with a
Deploy now button. Beast's `semmeltv-deploy-watch` feeds it and runs the
deploys. These need an admin or the service token; the state lives in memory
and starts empty when the app restarts.

- `GET /api/deploy`: `{ "revision", "checked_at", "main", "pending",
  "others", "asked", "waiting", "run" }`. `revision` is `/etc/semmeltv/revision`,
  `checked_at` when beast last sent main (null until it has), `main` main's
  newest commit. `pending` lists main's commits after `revision` that change
  the TVs, newest first, each `{ "sha", "at", "subject", "reports" }` with the
  reports it fixed (`id`, `person`, `message`); null when `revision` is not
  among the commits beast sent. `others` counts the commits in between that
  change only other machines. `asked` is `{ "at", "by" }` while a deploy waits
  for beast; `waiting` is `{ "at", "by" }` while beast holds a deploy because
  someone watches; `run` is the last deploy, `{ "state": "running" | "failed" |
  "finished", "at", "by", "message" }`.
- `POST /api/deploy`: asks for a deploy. 409 while one runs, or before beast
  has sent main.
- `POST /api/deploy/main`: beast sends main's newest commits (at most 200),
  `{ "commits": [{ "sha", "at", "subject", "reports", "tv" }] }`, where
  `reports` are the report ids its message names and `tv` says it changes
  the TVs. A report also counts as fixed by a commit when its status names
  that commit.
- `POST /api/deploy/progress`: beast says how the asked deploy goes,
  `{ "state": "running" }` (which takes the ask), `{ "state": "finished" }`
  or `{ "state": "failed", "message": … }`. Running also ends `waiting`.
- `GET /api/deploy/watching`: `{ "watching": [{ "person", "device", "title",
  "paused" }] }`, everyone a deploy would interrupt: the TV while a film is
  open in its player (paused too, until it has been paused for an hour) and
  each phone or laptop stream.
- `POST /api/deploy/wait`: beast holds a deploy until nobody watches, and the
  Feedback page says so; `DELETE /api/deploy/wait` takes that back.

## Trying it

    curl -s localhost:8090/api/search?q=lotus | jq '.results[].title'
    curl -s -X POST localhost:8090/api/play -d '{"show":"the-white-lotus"}' -H 'content-type: application/json'
    curl -s -X POST localhost:8090/api/playback/toggle
    curl -s -X PUT localhost:8090/api/subtitles -d '{"size":1.3,"background":0.5}' -H 'content-type: application/json'
    websocat ws://localhost:8090/api/events
    curl -s -X POST localhost:8092/api/downloads/search -d '{"text":"outlander season 8"}' -H 'content-type: application/json' | jq '.choices[0].facts'
