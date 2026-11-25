# SemmelTV

Apple TV style interface for a personal video library. It reads `library.db`
at the root of a library folder, draws menus with Slint, and plays video in a
separate mpv window.

    semmeltv core|tv|list|artwork|import|understand TEXT [--library DIR] [--meta DIR] [--cache DIR] [--db FILE]
             [--vfs-cache DIR] [--tmdb-token-file FILE] [--mdblist-key-file FILE]
             [--manifest FILE] [--listing FILE] [--refresh]
    semmeltv --manifest library.db library show | library from-toml FILE.toml

The library is `pcloud:SemmelTV`: `library.db`, `Movies/`, `Series/` and
`Artwork/`. `--library` defaults to `$SEMMELTV_LIBRARY`, then `~/pcloud/SemmelTV`.
`--meta` is where `library.db` and `Artwork/` are read from, when they live apart
from the videos; it defaults to `$SEMMELTV_META`, then the library.
`--cache` defaults to `$SEMMELTV_CACHE`, then `$XDG_CACHE_HOME/semmeltv`.
`--db` defaults to `$SEMMELTV_DB`, then `$XDG_STATE_HOME/semmeltv/semmeltv.db`.
`--vfs-cache` defaults to `$SEMMELTV_VFS_CACHE`. Nothing in the app writes to the
library: import and artwork work on a local copy of `library.db`, which the
`/semmeltv-incoming` skill and the downloads service upload.

## How it is built

SemmelTV runs as two processes from one binary.

- `semmeltv core` (`semmeltv.service`) is one async core on Tokio that owns
  everything the app knows and decides: the library, every profile's
  progress, the database, HDMI-CEC and the TV, headphones, the offline cache
  and downloads (`src/core/`), and the HTTP and WebSocket API the web app
  uses (`src/api.rs`), through the `Core` handle: typed requests in
  (`core.send`, `core.ask`), events and snapshots out.
- `semmeltv tv` (under cage on tty1) is the TV's screen in Slint
  (`src/ui.rs`) and mpv (`src/player.rs`). It keeps only the focus, the open
  page, loaded artwork and what plays, draws what the core sends and plays
  what it says, over a Unix socket (`src/link/`).

[ADR 3](../docs/adr/0003-core.md) and
[two processes](../docs/two-processes.md) have the reasons.

## library.db

The only source of truth for what is in the library: a SQLite file on pCloud,
shared by both TVs and every profile, with a table of shows, one of episodes and
one of movies (`src/library_db/`, through Toasty like the TV's own database).
Every show, episode and movie has a stable id, which survives a new title or a
moved file, and each profile's progress refers to those ids. It is written by
`semmeltv import`, `semmeltv artwork` and the downloads service, each on a local
copy that it uploads only if pCloud's copy did not change meanwhile. The app
reads nothing else: no file names, no folders. An entry whose file is missing is
skipped. Paths are relative to the library root, and `added` is a date.

A new film or episode from the downloads service enters library.db before its
upload to pCloud. Until the upload is done, the service lists the downloaded
copy in `ready.json` in its downloads folder (`SEMMELTV_READY`), and the TV that
downloaded it shows the entry and plays it, on the TV and on phones, from that
copy; the other TV skips it as missing until the file arrives. A replacement is
uploaded first, so the old copy stays playable until the new one is there.

It is not edited by hand. `semmeltv --manifest library.db library show` prints
it as text, which is how the incoming skill shows what changed, and `library
from-toml FILE.toml` builds one from that text. Schema changes are Toasty
migrations in `toasty/library/`, generated with `semmeltv toasty library
migration generate --name …`. A TV whose app is older than the file's schema
keeps its last good copy and says why. An app newer than the file reads it
through a temporary copy with its migrations applied, leaving the file as it
is; the next write applies them to the file itself. Printed, an entry looks like this:

    [[show]]
    title = "Dark"
    year = 2017
    last_aired = 2020
    tmdb = 70523
    imdb = "tt5753856"
    ratings = { imdb = 8.7, rt = 94, popcorn = 92, metacritic = 74 }
    rated = 2026-11-08
    episodes = [
      { season = 1, episode = 1, title = "Secrets", path = "Series/Dark/S01/Dark.S01E01.1080p.x265.DD5.1.mkv", added = 2024-05-01, runtime = 51, video = "HEVC", resolution = "1080p", audio = "5.1" },
    ]

    [[movie]]
    title = "Wind"
    path = "Movies/Wind/WIND.avi"
    added = 2024-05-01
    review = true

`year`, `tmdb` and an episode's `title` are optional. So are `video`, `resolution`
and `audio`, which import reads from the release tags in the file name, then from
the folders above it, and `runtime` in minutes, which the artwork step takes from
TMDB along with a missing episode `title`. A show's `last_aired` year and
`running = true`, while TMDB lists it as neither ended nor canceled, come from
the artwork step as well. `imdb`, `ratings` (`imdb` out of 10;
`rt`, `popcorn` and `metacritic` out of 100) and `rated` come from the artwork
step too; see Ratings. `review = true`, on a show, an episode or a film, marks
something a person should check; it changes nothing in the app.
`downloaded_by`, on an episode or a film, is the profile name of whoever asked
the downloads service for it: the person signed in on the phone who searched or
confirmed, the TV's profile for a download told to the TV, or `semmeltv get fetch --by`.
The title page lists it under Downloaded by; titles from before it have none.

Home and the rows sort by `added`, shows by their newest episode. Newly Added is
the 12 newest titles.

## Importing

    semmeltv import --manifest library.db [--library DIR | --listing FILE]

Compares the videos under `Series/` and `Movies/` with the manifest, parsing file
names as it goes, and adds what is new. Existing entries stay as they are, except
that a missing `video`, `resolution` or `audio` is filled from the path; a value
already there is never replaced. With no manifest yet it is a first import: `added` is each file's date. Later,
`added` is today, because a film sorted today may have an old file date.

`--listing` reads `rclone lsf -R --files-only --format tsp` output instead of the
tree. A new title takes its TMDB id from a search if a token is set. A title with no id, a file with no
episode marker, or a second copy of a title is marked for review. The report also
names entries whose file is missing and videos that were left out.

New videos reach the library through `pcloud:Incoming`: a person starts a Claude
session on beast and runs the `/semmeltv-incoming` skill, which sorts them into
place, runs the import and the artwork step against a local copy, and uploads
the result.

## Title details

A quiet line at the bottom of Home and of a show describes the focused card:

    HEVC 10-bit · 1080p · 5.1 · 2h 11m · Stopped at 1h 04m · Downloaded

Encoding and audio come from `library.db`. The duration is the one mpv reported
once a file has played, and until then the `runtime` in `library.db`. The stopping
point comes from the database, and a finished title has none, nor does one left
in its first minute. A show on Home describes its resume episode, named first, as
in `S2 E3`. Anything unknown is left out. No video file is read for any of this.

## Refreshing

The app checks `library.db` every 30 seconds and reloads it when it changed,
also during playback, so a new title shows on the TV and in the web app at
once. A title whose file is not on the mount yet is left out until the file
arrives: each check looks again for the files the last reload missed. Focus
stays on the same title and an open show stays open. Art for cards that had none
is looked for again on every reload.

## Streaming and offline

On semmeltv5, `/var/lib/semmeltv/library` is a read-only rclone mount of pCloud's
`Movies/` and `Series/` with a full VFS cache on local disk. `library.db` and
`Artwork/` are not read through the mount: `semmeltv-sync` copies them to
`/var/lib/semmeltv/meta` (`SEMMELTV_META`), at boot and every 15 minutes. It
compares `library.db`'s SHA-1 with pCloud's and does nothing more while they
match. When they differ it downloads the new manifest beside the old one, copies
new artwork, renames the manifest into place, then deletes artwork pCloud no
longer has, so every entry's artwork is on disk before the app sees the entry.
A failed run leaves the previous copy as it was. Run it by hand with
`systemctl start semmeltv-sync`. The cache size is
`services.semmeltv.offlineCacheGiB` in `semmeltv/hosts/semmeltv5/default.nix`.
Null only streams: rclone runs with `--vfs-cache-mode off`, so nothing lands
on the SD card and nothing is read ahead beyond rclone's in-memory buffer.

rclone only keeps what was read, so when a title starts the app reads the rest of
the file through the mount in the background, slowly, followed by the next
episode of a show. A file larger than the cache is skipped. `SEMMELTV_VFS_CACHE`
is the cache's file tree (`.../vfs/pcloud/SemmelTV`), `SEMMELTV_VFS_CACHE_BYTES` its size,
and `SEMMELTV_REMOTE` a `host:port` probed every poll to tell whether pCloud can
be reached. Without `SEMMELTV_VFS_CACHE` everything counts as streaming and cards
show no state.

The app keeps the cache in bounds itself, by title. Every poll it adds up the
blocks the cache files take, and when that passes `SEMMELTV_VFS_CACHE_BYTES`, or
the disk has less than 10 GiB free, it deletes whole titles, a movie or all of
a show's cached episodes, until the cache is under 90% of its size and the disk
has room again. Titles go least recently watched first, by the newest
`last_played` among their files or their Currently watching entry; a title never
played counts from when its newest file was cached. Files the library does not
list go first. It never removes what is playing, in Currently watching, in the
download queue, or has a resume point, and it logs each removal to the journal
with its title and size. It deletes the data and metadata files from rclone's
cache directly (`SEMMELTV_VFS_CACHE_META` is the metadata tree): rclone notices a
missing cache file when it next opens the item, forgets its ranges and downloads
again. rclone itself has no size limit, because it counts the ranges it holds in
memory, which stay stale after such a deletion until the item is opened again;
its `--vfs-cache-min-free-space 5G`, which reads the real free space, is the
last resort below the app's 10 GiB. The web app's Storage page shows all this
and removes a title by hand. Below it are pCloud's space and every title in the
library by what it takes on pCloud, biggest first: an admin deletes a whole film
or series there, after a second tap. The downloads service takes it out of
library.db and moves its files to pCloud's trash, and the TV drops its download
(see Downloads in API.md).

Cards show what is on the device: Downloaded, Downloading or Not downloaded, and
for a show a count such as 3 of 10 downloaded. A file counts as downloaded when
its allocated blocks in the cache cover its size, since cache files are sparse.
Newly added titles are not downloaded until watched.

Offline, the app uses the last good copy of the manifest,
`library.last-good.db` in the cache, and plays downloaded files straight from
the rclone cache, which also works when the mount is down or was started without
a network. Selecting anything else says it can't be reached. rclone's own view of
a cached file only survives offline until `--dir-cache-time` (1h) runs out, which
is why the app does not depend on it. Files played straight from the cache do not
refresh their place in the eviction order. Artwork, ratings and posters come from
the local copy, so they are all there offline.

## Remote

The TV's own remote works over HDMI-CEC through the kernel's `/dev/cec*` devices,
which needs membership of the `video` group. The app appears on the TV as a
playback device named SemmelTV. Machines without CEC simply skip this.

Arrows move, OK selects, Back or Exit goes back, and Play and Pause toggle
playback. During a video the arrows seek, and Stop quits.

The app wakes the TV and switches it to its input when the machine cold boots
with HDMI connected, or when HDMI is plugged in after being out for at least 10
seconds. Restarting the app never wakes it, and neither does a shorter hot-plug
pulse. Once the TV sends Standby, plugging in leaves it off until the TV shows it
is on again. A cold boot wakes it regardless. With a stored EDID (below) the
connector never unplugs, so only the cold boot wakes it this way.

Starting or resuming a video wakes the TV too, from the TV, the web app or the
API. The app asks the TV for its power status over CEC. If the TV is on
and SemmelTV is the active source, it does nothing. If the TV is on another
input, it switches it to SemmelTV and keeps playing. If the TV is in standby or
does not answer, it turns it on and pauses the video until the TV reports on with
SemmelTV as the active source, plus 2 seconds for the picture, or at most 20
seconds. A pause or resume in the meantime wins. SemmelTV is the active source
from its own Active Source until the TV sends Standby or routes elsewhere.

Nothing else switches the TV's input, so a deploy never takes the TV from the
PlayStation. A restarted core announces nothing: it does not know which input
the TV shows until it announces itself for a video or another device claims the
input. It answers the TV's Request Active Source unless another device took the
input since SemmelTV last had it, and a Standby does not change that.

Waking the TV, when told to, for a video or on plug-in, sends Image View On, asks the
TV for its power status every half second until it reports on (at most 15
seconds), then claims the input with Active Source. Until 5 seconds after that,
the app takes the input back if the TV routes elsewhere while it starts up.

## A TV that is off

Without help, a Raspberry Pi's HDMI port goes "disconnected" when the TV is
unplugged or, on some TVs, in standby. The kernel then forgets the TV's EDID: the
compositor turns the output off and modesets it again when the TV returns, ALSA
refuses the HDMI audio device (error 524, the driver treats an output without an
EDID as DVI), mpv falls back to software decoding with no audio and stays that
way, and HDMI-CEC loses its address. `services.semmeltv.tv.edid` stores the TV's
EDID and sets `vc4.force_hotplug=1` and `drm.edid_firmware`, so HDMI0 stays
connected with that EDID whatever the TV does. Video mode, decoding, audio and
CEC are then the same with the TV on, in standby or unplugged. Capture the EDID
from `/sys/class/drm/card0-HDMI-A-1/edid` with the TV connected, before setting
the option; a different TV or TV input needs a new capture, since the EDID
carries the CEC address.

The stock kernel only consults that stored EDID when the TV answers on the
HDMI DDC lines. A TV in deep standby does not, so every connector probe,
including the one the compositor makes when it starts, used to blank the EDID
for an instant and leave HDMI-CEC without an address until the next reboot. With
`services.semmeltv.tv.edid` set, the host loads a rebuilt vc4 module that, when
the probe found no CEC address, applies the stored EDID itself and takes the CEC
address and the audio ELD from it. Only that module is built, against the
cached kernel, in seconds.

Whatever the cause, the app watches each CEC adapter's physical address. When it
goes missing while the adapter's DRM connector still reads "connected", the app
writes `detect` to that connector's `status` in sysfs, after 2 seconds and then
at doubling intervals of up to 5 minutes, until the address is back. The kiosk
module's udev rule lets the `video` group write that file. An address restored
this way announces SemmelTV as the active source only if SemmelTV had the
input, and never turns the TV on.

Switching the TV to another input stops playback and closes the player, with the
position saved, the same as Stop. The app takes a Routing Change or Set Stream
Path to another address, or an Active Source from another device, as that
switch.

## Playing a video

The remote and the keyboard do the same thing in mpv. Left and Right skip 10
seconds, and repeating or holding them speeds up: 10, 10, 20, 30, 60, then 120
seconds a press. Changing direction or leaving them alone for 1.2 seconds starts
over. Up and Down skip a minute. Seeks stop at the start and 5 seconds before the
end, so a long fast-forward does not end the episode.

While a video opens the screen stays black. The display switches to 1080p at
23.976 Hz, what nearly every film is, the moment playback is asked for, while
mpv opens the file; `wlr-randr` takes about 75 ms. Once mpv knows the video's
real size and rate, a different one (a 25 fps episode, a 4K film) switches
again. Playback starts as soon as mpv has its first frame, whether or not the
TV has finished re-syncing. The menu's 60 Hz comes back 5 seconds after a film
stops, so a next episode or a restart at the same rate keeps the mode and the
TV never blanks between them. Each `wlr-randr` call gets 5 seconds before it
is given up. Commands to mpv go through a queue to one writer thread, and the
thread reading mpv's events does nothing slow.

Starting, seeking, pausing and resuming show the title, a progress bar, and the
elapsed and remaining time at the bottom of the screen, for 4 seconds or for as
long as the video is paused. A seek also shows where it lands and the size of the
last jump. It is drawn with mpv's `osd-overlay` because mpv shows nothing of its
own for commands sent over its IPC socket, which is how the remote reaches it.

Stopping a video saves its position and playing it again resumes there. So does
a restart, and a crash or power cut loses at most the last 30 seconds. A video
that played to its end starts over. The database keeps, per profile, the last
episode started per show with how its subtitles last showed (off, or on in a
language), and each file's stopping point with the audio track,
subtitle track and whether subtitles showed. An episode that starts from the
beginning takes its show's subtitles, on the TV and on a phone, before the
profile's own "Show when a video starts". mpv keeps nothing of its own: it
starts with `--start`, `--aid`, `--sid` and `--sub-visibility` from the
database and `--no-resume-playback`. A file's real duration and where its
credits start are kept once for everyone. Files are keyed by their path in the
library, so a position saved while streaming also applies to the same file
played from the rclone cache.

## Sound

The TV folds the sound into stereo with a lavfi chain (`audio::stereo`),
ending in a limiter at -1 dBFS so loud scenes no longer clip. A browser gets
the file's own AAC, E-AC-3 or AC-3 when it plays it, as Safari does. Otherwise
a surround track stays 5.1 as 384 kbit/s AAC with the same limiter
(`audio::SURROUND`), and the browser downmixes it for stereo speakers itself;
a stereo track becomes 256 kbit/s AAC through `audio::stereo`. Dialogue mode
always folds to stereo.

Dialogue mode, off by default and kept per profile, makes voices easier to
follow on TV or laptop speakers. A 5.1 track is mixed to stereo with the
centre channel, where the dialogue is, a little louder than the rest, and every
track then goes through a gentle compressor and a slow dynamic normaliser, so
explosions and music come down and quiet speech comes up. Measured on The
Matrix, the gap between a dialogue scene and the lobby shootout drops from
9.4 LU to 3 LU, and the loudness range of a dialogue scene from 21 LU to 12 LU.
The green key on the remote, or D on a keyboard, switches it for whoever's
video the TV plays, at once during a video; the web app has a switch in
Settings and in the player sheet. A phone stream whose sound is converted opens
again where it is within 10 s, through the other chain; an E-AC-3 stream to
Safari changes to converted AAC with the next video.

## Subtitles

The yellow key on the remote, or S on a keyboard, shows or hides subtitles
during a video and says which: "Subtitles: English", "Subtitles off", or "No
subtitles in this file". Showing them with no track selected picks the first
preferred language that has a full track, ahead of its hearing-impaired one.
Subtitle files next to the video with a similar name load too.

The database holds each profile's settings: whether subtitles show when a
video starts, the preferred languages in order, and the size, position, colours,
outline, shadow and background box. Every video starts with them. The web app
changes them through the [HTTP API](API.md#subtitles), and a change restyles the
playing video at once. A resumed video keeps its own track and whether its
subtitles showed, but always takes the current style.

## Skipping intros and recaps

During an episode's recap or intro a Skip button shows in mpv, and OK presses it.
Otherwise OK pauses, as Play and Pause always do. Skipping is never automatic,
because the times do not fit every release: a file with a recap the database's
release lacks lands partway through the titles. Back hides the button instead of
stopping, and the remote's × next to its Skip button does the same; the button
stays away for that kind until the file plays again, and the next Back stops.

Times come from [TheIntroDB](https://theintrodb.org) when an episode starts, and
are not stored. An episode needs its show's `tmdb` in `library.db`, and without
one or without a network it plays with no button. When
several ranges exist the earliest counts, since later ones have been wrong. An
intro said to start at 0:00 and last over 95 seconds is dropped as a cold open
someone marked as the intro, as on several Community episodes, unless the
episode before or after has an intro within 8 seconds as long: Game of Thrones
opens with its 1:50 titles. Their
terms allow personal, non-commercial use and forbid passing the data on, so
timings must not be exported or committed.

## Telling the TV

For those who may control the TV, the web app's search box also tells the TV
what to do: the text goes to `/api/command`. `semmeltv understand TEXT` shows
what a sentence would do, in English, German or Spanish:

    continue playing Outlander
    spiele Outlander Staffel 2 Folge 3
    descarga la siguiente temporada de Outlander

Continuing plays the next episode once the last one ended. Titles match the
library loosely, so slips are forgiven; translated titles are not known. A
download request asks the downloads service for the title, in the TV
profile's name.

"Turn on the TV" and "switch to SemmelTV" ("auf SemmelTV umschalten",
"SemmelTV input") both end with the TV on and showing SemmelTV. The name is
recognised however it is spelt: Semmel TV, SimilTV, Simmel TV, Semel TV,
Samuel TV, Simple TV and the like.

What a download search found shows in a panel above the details line, for a
minute when there is something to confirm on the phone's Downloads page and
for 6 seconds otherwise. During a video the panel is drawn at the top instead,
clear of the progress bar and the Skip button; mpv cannot measure text, so its
width is estimated from the length and lines longer than 72 characters are cut
short. The same goes out on the API's event stream as a `notice`; see
[API.md](API.md).

## Database

Everything the app learns while running lives in one SQLite database,
`/var/lib/semmeltv/semmeltv.db` on a Pi, owned by the `semmeltv` user, through
the [Toasty](https://github.com/tokio-rs/toasty) ORM
([why](../docs/adr/0002-toasty.md), [how](../docs/toasty.md)). `library.db`
stays the only source of truth for the library, and `library.last-good.db`
stays in the cache. The models are in `src/db/models.rs`, the migrations Toasty
generated from them in `toasty/tv/`; the app applies new ones when it starts.
`src/db/` is the only code that touches the database, and its methods are
async: the core and the API await them.

| Table | Holds | Per |
| --- | --- | --- |
| `profiles` | Frederik (admin), Mami, Miriam, Ramu (viewers): name, role, password hash, colour | |
| `capabilities` | what an admin granted a viewer on this TV: `control_tv`, `download` (old `headphones` rows mean nothing) | profile |
| `passkeys`, `sessions`, `invites` | sign-in, kept by the auth code | profile |
| `video_places` | an episode's or film's stopping point, when it was last played, finished, audio and subtitle track, subtitles shown | profile |
| `series_places` | the last episode started per show, whether it finished, and how its subtitles last showed | profile |
| `started` | Currently watching, newest first | profile |
| `dismissed_videos` | episodes and films taken off Started and Forgotten | profile |
| `subtitle_settings` | the subtitle settings, as JSON | profile |
| `audio_prefs` | no headphones, or the paired headphones and whether the sound goes to them; dialogue mode on or off | profile |
| `video_facts` | an episode's or film's real duration, where its credits start, its subtitle delay and how many subtitle tracks it carries | video |
| `settings` | one value per key, such as the Samsung pairing `tv_token` | TV |
| `usage` | who used the TV and what they watched, see [Usage](#usage) | event |
| `watch_together` | who watches which show together with whom: a group per show, one row for each pair, each way | profile |
| `push_subscriptions` | the browsers someone turned notifications on in: push endpoint and keys, see [Notifications](API.md#notifications) | profile |

Episodes, films and shows are referred to by their id in `library.db`, so a
renamed title or a moved file keeps everyone's progress. The rest of the app
still speaks in file paths and title keys: the store maps them to ids through
the library the app loaded last, and leaves out what that library does not
know, without deleting it. Until the app has loaded a library, it reads no
progress and deletes none. The `tv_profile` setting is the profile the TV's own
screen acts as.

Progress from before library ids, kept by file path and title key, is moved
over once the app knows the library: migration `0002` renames those tables to
`legacy_*`, and the app copies what it can match into the new ones and drops
them.

Read it on the Pi with `sudo -u semmeltv sqlite3 /var/lib/semmeltv/semmeltv.db`.

After changing a model, generate the next migration from `semmeltv/app`, with a
scratch database since generating only compares the models with the last
snapshot in `toasty/tv/snapshots/`:

    cargo run -- --db /tmp/scratch.db toasty tv migration generate --name what-changed

Use `toasty library` for the library's models in `src/library_db/models.rs`. When
a table goes and another comes in the same change, the command asks whether it
was a rename, which needs a terminal; generating the additions and the removals
as two migrations avoids the question.

`semmeltv toasty …` is Toasty's own migration CLI, built into the app because
it needs the app's models. Review the SQL it writes before committing it.

## Profiles

Frederik is the admin; Mami, Miriam, Ramu and Chris are viewers. Viewers
browse, watch on their own phone or laptop while nothing else plays, set
dialogue mode and keep their own subtitle style. Playing on the TV is a
capability an admin grants per person and per TV, on the People page
(`control-tv`): with it a viewer also plays on the TV, uses its remote and
the TV/phone switch, and a phone of theirs takes over from
whatever plays. New profiles start without it; on semmeltv5 only Chris has it
(migration `0006` granted it). It also lets them pair their own Bluetooth
headphones with the TV in Settings, send the sound to them and set their timing
in the remote's sheet. Headphones belong to the profile that paired them:
nobody else sees them, sends the sound to them, sets their timing or forgets
them, and choosing your own takes them on the TV at once. Downloading is a
second capability (`download`, migration `0008`): Settings > Downloads to find films and seasons
and follow the running downloads, and a show's "More seasons" buttons; nobody
but the admin has it until it is granted. Without a capability the web app
shows none of its controls, and the API refuses them. Only an admin stops a
download, removes titles from the TV's storage, pairs the TV for volume, and
adds, invites, signs out or removes people, and grants or takes back
capabilities; an admin has every capability. Everyone signs in, at home and on
the tailnet alike; the API documents how ([Sign-in](API.md#sign-in)).

Per profile: Currently Watching, Started and Forgotten and what was dismissed
from it, every file's stopping point, last play, finish and chosen audio and
subtitle tracks, each show's place, watched marks, the subtitle settings,
and the headphones with the choice of TV or headphones.

Shared, because there is one of each: what the library holds, the TV and what
it plays, a file's real duration, its credits and its subtitle delay (these are
facts about the file), the Samsung pairing token, which the TV gives to
SemmelTV rather than to a person, the offline cache, downloads and the
Bluetooth bonds themselves. The rules:

- A video's progress always goes to the profile that started it, whoever
  pauses, seeks or stops it, on the TV or a phone. A phone stream belongs to
  the profile that opened it; a handoff keeps the owner.
- The TV's own screen and remote act as the TV profile: whoever last
  started something on the TV from the web app or a phone, moved a video to
  the TV, paired headphones or pressed Show mine in Settings. It survives a
  restart, and its name shows at the top right of Home. Up from the top row of
  Home opens the profiles there; Up and Down move, OK switches, and Left or
  Back return to the rows. The remote
  works without signing in, because whoever holds the remote is in the room;
  a PIN on the TV would only slow them down.
- When B plays something on the TV while A's video plays, A's video stops
  with its place saved to A, the TV becomes B's, and B's video plays. Pausing
  or seeking A's video changes nothing about whose it is.
- Everyone sees the same now-playing bar and can use the remote.
- Subtitles on the TV use the style of the video's owner. Changing the style
  restyles the playing video only for its owner; the others save theirs for
  their next video.
- The TV uses the headphones of whoever's video plays, or of the TV profile
  while nothing plays. When a video starts, the sound leaves someone else's
  headphones for the TV, and moves to the owner's if they chose headphones and
  theirs are connected. Nobody's headphones change in the middle of their
  video. Pairing never removes headphones another profile still uses.

## Usage

The `usage` table keeps, from migration `0009` on, one row per:
- **sign-in**: a password, passkey or invite opening a session, with the device
  label such as `iPhone · Safari`;
- **visit**: a signed-in device back in the app, at most one per session and
  hour (when its session renews);
- **play**: an episode or film played on the TV or a phone, with the seconds
  actually watched, which its saves and reports move forward.

Rows hold the profile id, not a relation, so they outlive a deleted profile.
`semmeltv stats [DAYS]` prints the last 7 days by default: per person the
sign-ins, days and hours active, plays and time watched on the TV and the phone,
then the most watched shows and films, then the plays. On a TV it is
`semmeltv-stats [DAYS]`, and in the devenv shell `semmeltv-stats [DAYS] [HOST]`
runs that over ssh, on semmeltv5 unless a host is given. Admins see the same
as charts on the web app's Usage page (Settings → Usage), which reads
`/api/usage` (see [API.md](API.md#usage)).

## HTTP API

`SEMMELTV_API=127.0.0.1:8090` (or `--api`) serves JSON over HTTP and a WebSocket
event stream from the app itself: the library, search, details, artwork, playing
and playback control. [API.md](API.md) lists the endpoints. The TV keeps running
if the address cannot be bound. The web app in `../web` is built on it.

## Feedback

Every page of the web app has a speech bubble in its top bar. It opens a box to
type or dictate into, with an optional screenshot. Sending stores a report on
the TV with:

- the text, who sent it and the page;
- the phone's trail of the last 300 things it did;
- the TV's own snapshot at that moment: what plays, the screen and audio state,
  the last 20 minutes of the app's journal, warnings from `semmeltv-web`,
  `semmeltv-mount`, nginx and the kernel, uptime, load, memory, temperature,
  the Pi's throttle flags, the deployed revision (`/etc/semmeltv/revision`),
  and a screenshot of the TV taken with grim.

Reports are files, not rows in `semmeltv.db`, so they survive a database reset.
`SEMMELTV_FEEDBACK` names the folder: by default `feedback` next to the
database, which on the TVs is `/var/lib/semmeltv/feedback`. It holds
`<id>.json`, `<id>-phone.jpg`, `<id>-tv.jpg` and `index.jsonl`, one summary per
line. The TV keeps the newest 300. Nothing leaves the TV. Admins see the list
on the web app's Feedback page (Settings, then Feedback). Its "Copy for Claude"
button copies a summary and the command that fetches the full report.

Each report has a status, kept beside it in `<id>.status.json`:

- New, until someone marks it;
- Reopened, when a person comments on a report that was settled; it keeps the
  status it had before;
- Needs review, with why and what a person has to decide, and optionally up
  to 6 `questions`, each with 2 to 8 `options` and the index of the
  `recommended` one. The Feedback page shows them as radio buttons with the
  recommendation picked; Answer posts the picks as the person's comment, which
  reopens the report;
- Done, with the commit that fixed it, or why nothing changes.

Each report also has a thread of comments in `<id>.comments.jsonl`. A comment
is signed by the person who wrote it, or by Claude when the service token sent
it. Only a person's comment reopens a report.

`POST /api/feedback/<id>/status` sets the status, and
`POST /api/feedback/<id>/comments` adds a comment. Both work for an admin or
with the service token. `GET /api/feedback?state=new|reopened|needs_review|done`
filters by status. The Feedback page shows the statuses and threads, filters
by status, and has a comment box and a status editor under each report. A
status set or a comment sent from the page moves the report between the
filters and their counts at once, without a reload.

Beast triages New and Reopened reports on its own. The
`semmeltv-feedback-watch` user timer asks both TVs every two minutes. For each
New or Reopened report it starts one background Claude session,
`feedback-<id>`, in its own worktree, at most four at a time; they show in the
Claude app and in `beast`. The prompt is `home/hosts/beast-semmeltv/prompt.md`
with the report and its thread appended. Each session builds what its report
asks for, new features included, takes the option it would recommend instead
of asking, lands on main and deploys by itself, and ends with `session-end`.
Needs review is left for what stays unclear or is off limits. For a reopened
report, the person's newest comments are its instruction, such as the option
to build. Every session answers in the thread and appends what it learned to
one shared memory note, `project_semmeltv_triage_lessons.md`.
`semmeltv-deploy` always deploys the newest origin/main, so sessions deploying
one after another never roll back each other's work. By hand, from beast:

    semmeltv-feedback semmeltv5 list new
    semmeltv-feedback semmeltv5 show <id> | jq '.message, .client.trail[-40:], .tv.problems'
    semmeltv-feedback semmeltv5 show <id> | jq -r '.tv.app_journal[]' | tail -80
    semmeltv-feedback semmeltv5 pictures <id> ./shots
    semmeltv-feedback semmeltv5 thread <id>
    semmeltv-feedback semmeltv5 comment <id> "what Claude did"
    semmeltv-feedback semmeltv5 mark <id> needs-review "why" "what to decide"
    semmeltv-feedback semmeltv5 mark <id> needs-review "why" "what to decide" \
      --ask "Which way?" --recommend "the first" --option "the second"
    journalctl --user -u semmeltv-feedback-watch
    claude agents --json | jq '.[] | select(.name | startswith("feedback-"))'

A report's `tv.problems` lists what the TV
couldn't collect, such as a journal it couldn't read.

The top of the Feedback page lists what main has that this TV doesn't run
yet: each commit that changes the TVs, with the reports it fixed, and a Deploy
now button. The `semmeltv-deploy-watch` user timer on beast asks both TVs
every 20 seconds, sends main's newest 100 commits when the TV doesn't have
them yet (`semmeltv-feedback HOST deploy main FILE`), and runs
`semmeltv-deploy --now HOST` from origin/main when the button was pressed. It
deploys at once, even mid-film, and tells the page whether it is running,
finished or failed. Without `--now`, `semmeltv-deploy` deploys nothing while
anyone watches (`semmeltv-feedback HOST deploy watching`): it marks the deploy
held, the page says so, and the timer deploys origin/main once nobody
watches. The box stays away until beast has sent main, so the
button never shows without beast behind it. `semmeltv-feedback HOST deploy
state` shows what the TV knows; `journalctl --user -u semmeltv-deploy-watch`
shows the deploys.

## Artwork

    semmeltv artwork --manifest DIR/library.db [--refresh]

Artwork is fetched when titles are added, not on the TVs, which need no TMDB
token. The step reads the manifest and, for every title with a `tmdb` id, saves
images into `DIR/Artwork`, beside the manifest:

    Artwork/movie/<tmdb id>/backdrop.jpg
    Artwork/movie/<tmdb id>/poster.jpg
    Artwork/tv/<tmdb id>/backdrop.jpg
    Artwork/tv/<tmdb id>/poster.jpg
    Artwork/tv/<tmdb id>/s01e02.jpg

Backdrops and episode stills are 640x360 JPEGs. Posters are 342x513 portrait
JPEGs from TMDB's `w342` size, for the web app's grid; the TV cards use the
backdrops. Neither is named in the manifest: the path follows from the `tmdb`
id, and `Show::poster` / `Movie::poster` give it relative to the library root
when the file is there.
It also writes a missing `runtime` and episode `title` into the manifest, and a
show's `last_aired` and `running`, which `--refresh` asks again for running shows. It only
asks TMDB for what is missing, so a run over a copy of `pcloud:SemmelTV/Artwork`
is quick. It refuses a manifest inside `--library`, so it never writes through
the mount. Titles without a `tmdb` id are listed and keep the gradient.

The app shows `<library>/Artwork/...` when the file is there and a gradient with
the title otherwise. The TMDB notice on Home appears once any title has an id.

TMDB's terms allow keeping images for six months, so refreshing is a manual
step: `--refresh` downloads again every image older than five months and deletes
one it cannot replace. The `/semmeltv-incoming` skill describes the upload. The
step needs a TMDB API Read Access Token, from `TMDB_TOKEN_FILE`,
`--tmdb-token-file` or `TMDB_TOKEN`.

## Ratings

The artwork step also stores ratings from MDBList: IMDb, the Rotten Tomatoes
Tomatometer (`rt`), the Rotten Tomatoes audience score (`popcorn`) and
Metacritic. It looks each title up by its TMDB id
(`https://api.mdblist.com/tmdb/{movie,show}/ID`) and writes `imdb`, `ratings`
and `rated`, the day it asked, into the manifest. A title with a `rated` date is
not asked again; one MDBList knows nothing about gets a `rated` date and no
`ratings`. Any score can be missing, and a score of 0 counts as missing.

The details strip shows the Tomatometer (a tomato from 60%, a splat below), the
audience score (a full popcorn bucket from 60%, a tipped-over one below) and
IMDb, each only when present. Metacritic is stored but not shown.

`--refresh` asks again for ratings 30 days old or older. MDBList's free key
allows 1000 requests a day (and 1000 per 5 minutes), so a run stops after 900
and on HTTP 401, 403 or 429, keeps what it has and leaves the rest for the next
run. Ratings need an MDBList key from `MDBLIST_API_KEY_FILE`,
`--mdblist-key-file` or `MDBLIST_API_KEY`; without one the step skips them. The
TVs never get the key.

Episodes get their own IMDb rating, written as `ratings = { imdb = 8.1 }` on the
episode line. MDBList has no episode scores, so the step downloads IMDb's
non-commercial datasets (`title.episode.tsv.gz` and `title.ratings.tsv.gz` from
`datasets.imdbws.com`, about 65 MB) and matches episodes by the show's `imdb` id,
season and number. It only downloads them when a regular episode of a show with an
`imdb` id has no rating yet, or with `--refresh`, which takes every episode's
rating anew. Specials rarely match IMDb's numbering and are left alone when
missing. Rotten Tomatoes scores episodes too rarely for any source the step can
reach, so an episode's `rt` is only there if written by hand.

The show page on the TV lists each episode's ratings under its card and ends with
a heatmap: a row per season, a cell per episode, coloured by IMDb rating
(`heatmap.rs`: Awesome from 9.0, Great 8.0, Good 7.0, Regular 6.0, Bad 5.0,
Garbage below). Down from the last season lands on the best episode; left and
right step through the episodes from best to worst (`heatmap::ORDER`, which can be
`Order::Grid` for season and episode order instead), OK plays it and up goes back
to the seasons. Cells shrink to fit the screen, and below 50 px they drop the
number. The web app's show page has the same grid, where a tap plays the episode.

Up from the first season reaches a "Watch together" row above it, once the app
has answered which other profiles there are (`Request::TogetherOnTv`, then
`Event::Together`). It lists the TV profile's partners on that show first, each
with where they are, then everyone else. OK on someone else pairs the two of
you, as the web page's Watch together button does; OK on a partner moves your
place to theirs (`Request::WatchWithOnTv`). Ending a pairing is on the web page.

## Development

    cd semmeltv/app
    devenv shell
    cargo test
    semmeltv-run

`semmeltv-run` starts the app as the TV runs it, `semmeltv core` and
`semmeltv tv` as two processes talking over a socket in
`$XDG_RUNTIME_DIR/semmeltv-run`; its arguments go to both.

`semmeltv/devenv.nix` holds the tooling: the Rust toolchain, `slint-lsp`, `mpv`,
`wlr-randr` and `bacon`, with the build environment the package uses
(`app/build-env.nix`). It imports the light root `devenv.nix` of the repo, and
`devenv shell` picks it up anywhere under `semmeltv/`. The shell sets
`SEMMELTV_LIBRARY` to `~/pcloud/SemmelTV`, where the desktop's rclone mount of
the whole pCloud account has the library, unless it is already set.

Loading the library stats every file, which over that mount means one pCloud
listing per folder, about a minute in all. Entering the shell runs
`semmeltv:warm-library`, which asks the mount over its rc socket
(`$XDG_RUNTIME_DIR/rclone-pcloud.sock`) to list the library recursively in one
go, about two seconds, after which a load takes about as long. The mount keeps
listings for 30 days and learns of changes from pCloud within a minute.
