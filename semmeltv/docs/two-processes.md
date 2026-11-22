# Two processes: `semmeltv core` and `semmeltv tv`

Date: 2026-11-22. Status: proposed. Supersedes the "core owns the player" part
of [ADR 3](adr/0003-core.md); the rest of ADR 3 (one actor, typed requests,
snapshots) stays.

## Goal

> All the mpv-specific stuff should not come from the core. It should be part
> of a separate Slint UI part of the application that deals with mpv and the
> Slint UI. If that freezes, the core should not freeze. I want the core to be
> stable and have a high uptime rate. (Frederik, 2026-11-22)

- **`semmeltv core`**: the API and website backend, the database, the library,
  downloads, push, sessions and sign-in, TV control (CEC, UPnP, Samsung), voice,
  headphones, and the decision of what plays for whom from where. Its own
  systemd service, outside cage.
- **`semmeltv tv`**: the Slint screen, mpv, display modes, the OSD, mpv's
  audio device. Runs under cage. Talks to the core over one local socket.

The core never blocks on the tv, never shares memory with it, and never waits
for it without a deadline. If the tv hangs or crashes, a watchdog restarts it
and the core tells it what was playing.

## Why

On 11-21 at 21:59 the TV froze on a display mode switch and the web app went
down with it until the power button. The cause (fixed in 2d02282) was a
deadlock between the player's threads and mpv's IPC socket that held the
player's `Mutex<Playback>` forever. The actor calls into `Player` at 44 places
(`state()` in the 500 ms playback tick, `set_paused`, `set_muted`, `say`,
`command`, …), and each call takes that lock, so the actor stalled and every
API request queued behind it. 2d02282, fdf4752 and 9039ceb removed that
deadlock and the slow work around it. The structure that turned one stuck mpv
into a dead website is still there: the core holds the `Player` in its own
process and takes its lock. The next stuck lock does the same, whether it is a
Slint bug, a GPU driver hang or an mpv freeze.

## Decisions in brief

1. One binary, two subcommands: `semmeltv core` and `semmeltv tv`. Both come
   from the same Nix store path, so a deploy always runs matching builds.
   `semmeltv ui` keeps running both in one process, connected by in-memory
   channels instead of the socket, for the laptop and for tests.
2. They talk over one Unix socket, `/run/semmeltv/tv.sock`. The tv connects,
   and the core sends commands down the same connection. Messages are
   newline-delimited JSON. Only uid `semmeltv` can connect.
3. The core keeps no handle into the tv. It keeps the last `Report` the tv
   sent, a plain value, and sends orders through a bounded queue with
   `try_send`. A full queue or a silent tv closes the connection. Nothing in
   the core waits for the tv without a timeout.
4. Everything that decides something stays in the core: the up-next countdown,
   voice, watching together, segments, position saving, handover, and where
   the sound goes. The tv draws and plays. It also owns the low-latency
   playback logic that has to react to a key press at once: scrubbing, the
   skip pill and toasts.
5. The TV-set hand-off renderer stays in the core. It plays on the Samsung,
   not under cage, and Frederik put UPnP in the core. It loses its shared
   `Mutex<Seen>` and reports through signals like the tv does.
6. `cage-tty1` runs `semmeltv tv` with a systemd watchdog. A new
   `semmeltv.service` runs the core with a watchdog of its own, fed by the
   actor loop.
7. If the tv dies mid-film, the core resumes that film on the new tv, 3 s
   before the last reported position. If the core dies mid-film, the tv
   keeps playing and the restarted core adopts the running playback.

## 1. Couplings today and who owns them after

### Shared memory and threads (all of these go)

| Today | Where | After |
|---|---|---|
| `Player` = `Arc<Mutex<Playback>>`, locked by the actor, mpv's listener, the 100 ms tick, the segment fetcher and the fader | `player.rs` | Only inside the tv. The core keeps `Report` values. |
| `Subtitles` = `Arc<Mutex<Inner>>`, shared by the API (`/api/subtitles` reads `playing`), the actor (`set_owner`) and mpv's listener (`observe`, `loaded`, `change`) | `subtitles.rs` | Split. The core keeps the settings per profile and the per-file memory (DB-backed). The tv keeps the live track state (`playing`, `sidecars`, `awaited`) and reports it in `Report.subtitles`. |
| renderer `Arc<Mutex<Seen>>`, read by `Player::state/place/played` | `player/renderer.rs` | Stays in the core, but the steering thread sends `Seen` as a `Signal`, so the actor never locks it. |
| `display::SETTLER`, a process-global thread | `display.rs` | tv only |
| `ui::run` runs on the core's Tokio runtime handle and subscribes to `core.subscribe()` (the `Bus`) | `ui.rs`, `main.rs` | The tv has its own runtime and gets menu messages over the socket. |
| `Bus` events marked `#[serde(skip)]`, which only the TV gets: `Home`, `Key`, `Message`, `Panel`, `UpNext`, `Together` | `bus.rs` | Become `ToTv` messages. The bus keeps only the phones' events (`Voice`, `Playback`, `Audio`, `Screen`). |

### Calls from the actor into `Player` (`core/actor.rs`)

| Call (count) | Used for | After |
|---|---|---|
| `Player::start` (1), `Player::hand_off` (1) | `play_from` | The core sends `ToTv::Play` (mpv) or starts a renderer session (TV set). The answer comes back asynchronously as `Started` or `Failed`. |
| `state()` (5) | `publish_playback` every 500 ms, `hear_sound` every 500 ms, `PlayPause`, `skip_segment`, `OnTv::playing` | Last `Report.state` |
| `place()` (2), `tracks()` (1), `played()` (2) | `record_place`, `note_tv_play`, `tv_place` (handover) | Last `Report` |
| `muted()` (1) | `hear` (mute while listening) | `Report.state.muted` |
| `command(Command)` (5) | `key`, `act` (Back before a queued play), `control(Stop)`, `seek` steps | `ToTv::Command` |
| `set_paused` (4) | hold (TV waking, handover), Pause and Resume | `ToTv::Pause` |
| `set_muted` (6) | voice listening, offer answers, Mute and Unmute | `ToTv::Mute` |
| `say` (4), `toast` (1), `hide_toast` (1) | dialogue, audio route, speed, `tell`, voice panel | `ToTv::Toast` / `HideToast` |
| `seek_to`, `seek_by` | `/api/playback/seek` | `ToTv::Seek` |
| `skip_segment`, `hide_skip` | `/api/playback/skip`, `hide-skip` | `ToTv::SkipSegment` / `HideSkip`. The core answers from `Report.state.skip` and does not wait for the tv. |
| `set_speed` | `/api/playback/speed` | The core clamps with `player::speed_within`, which is pure, and sends `ToTv::Speed`. |
| `set_audio_device` | headphones route (`Signal::Audio`) | `ToTv::AudioDevice` |
| `set_dialogue` | dialogue mode | `ToTv::Dialogue` |
| `subtitles(Change)` | `/api/subtitles/*`, `Request::Subtitles` | `ToTv::Subtitles { id, change }` → `FromTv::Answered { id, … }`, 2 s timeout |
| `offer_next`, `handed_off` | `play_from` | A field of `Play`, and the core's own record of which backend it started |
| callbacks `on_save`, `on_key`, `on_exit` | `Signal::Saved`, `Request::Key`, `Signal::Finished` | `Report` (the core decides when to save), `FromTv::Key`, `FromTv::Finished` |

### Feature by feature

| Feature | Today | Owner after | Why |
|---|---|---|---|
| **mpv process, IPC, OSD** (`player.rs`, `player/commands.rs`, `fade.rs`, `health.rs`, `overlay.rs`, `seek.rs`) | Spawned from the actor through `block_in_place` | **tv** | This is the code that can hang. |
| **Display modes** (`display.rs`, `wlr-randr`; `ui.on_resized → display::refresh`) | Driven from the player's listener | **tv** | Needs cage's Wayland and the player's video track. |
| **Slint menu** (`ui.rs`, `nav.rs`, `details.rs`, `heatmap.rs`, `loader.rs`, artwork files in `meta/Artwork`) | Events from the `Bus` | **tv** | Pure drawing. It reads artwork files read-only, the same as today. |
| **Menu requests** from `ui.rs`: `PlayFile`, `WatchWithOnTv`, `TvProfile`, `TogetherOnTv` | `core.send` | **core**, reached by `FromTv::Ask` | They change progress, the profile or pairs. |
| **Remote keys** | CEC thread (`cec.rs`) → `Request::Key` → `actor.key` → the player, voice, up next, or `Bus::key` → menu. mpv's keybinds send Back, Listen and Dialogue to `on_key`. Slint `on_key` → `Request::Key`. | Routing stays in the **core**, CEC stays in the **core** | One path, as in ADR 3. The core knows about voice, up next and offers. Keys pressed in the tv (keyboard, mpv window) go to the core as `FromTv::Key`. While the tv is disconnected it handles its own keys locally (see §2.6). |
| **Voice** (`voice.rs`: `arecord`, wake word over Wyoming, parakeet, `intent.rs`, the LED, `/api/listen`, the download offer) | Actor, with `player.set_muted` and `toast`, plus `Bus::panel` for the menu | **core** | It must work while the tv is gone ("turn the TV off", "play X"). The tv only mutes and shows the panel or toast when told. |
| **Phone microphone** (`/api/microphone` → ALSA loopback) | API | **core** | Unrelated to mpv |
| **Headphones** (`audio.rs`, `core/sound.rs`, `/api/audio*`, `/api/volume`): bluetoothctl, bluealsa-cli, pairing, ownership | `Audio` threads in the core; `Route` → `player.set_audio_device` + `say`. `hear_sound` feeds `Pace` from `player.state()` every 500 ms. | **core**. The tv only sets mpv's `audio-device`. | Profile-owned and DB-backed. `Pace` runs on `Report` position and speed, which arrive at the same 500 ms rate. |
| **Dialogue mode** | DB flag in the core → `player.set_dialogue` | Flag in the **core**, filter in the **tv** | |
| **Up next** (`up_next.rs`) | The actor sets `UpNext` on `Finished`, checks `due()` in the 500 ms tick, plays. The card goes over `Bus::up_next` to Slint, which runs a 50 ms timer. | **core** decides and times it; **tv** draws the card | Choosing the next episode needs the library and progress. `up_next::Card` holds `Instant`s, so the message carries the remaining milliseconds instead. |
| **Play next** at the credits | `offer_next(follows)`, mpv exits with code 12 → `Finish::Next` | The **core** sets `Play.next`; the **tv** shows the pill and reports `Finished { Next }` | |
| **Segments / skip intro** (`segments.rs`, `ureq` to theintrodb.org from a player thread) | Fetched inside the player once mpv reports the duration | The **core** fetches once the first `Report` has a duration and sends `ToTv::Segments`. The **tv** shows the pill and skips on OK. | The tv makes no internet calls, and the core can cache. The skip itself stays local so OK reacts at once. |
| **Subtitles** (`subtitles.rs`, `sidecar.rs`, `subfetch.rs`, `/api/subtitles*`, `/api/audio/track`) | Shared `Subtitles` object (see above) | Settings, per-file memory, sidecar search, OpenSubtitles fetch: **core**. Track choice in mpv: **tv**. | `Play` carries the owner's `Settings`, the file's `Remembered` and its sidecars. The tv sends `FromTv::Remember` when a choice should persist. `/api/subtitles` reads `Report.subtitles`. |
| **Position saving** | mpv listener calls `on_save` every 30 s of movement → `Signal::Saved` → `record_place` locks the player | **core**, from `Report`s | The core applies `save_due` to reported positions and also saves on `Finished` and when the link is lost. The most it loses is one report interval. |
| **Usage / watch history** (`db.note_play`, `watching::record`, `tell_partners`, push) | Actor, with `player.played()` | **core**, from `Report.played` | |
| **Watch together** (`together.rs`, `Together` list for the TV) | Actor → `Bus::together` → Slint | **core** computes, **tv** draws (`ToTv::Together`) | |
| **Phone/TV handover** (`screen.rs`, `hold.rs`, `/api/screen`) | Actor, with `tv_place()` from `player.place()` | **core**, with the TV's place from `Report` | |
| **TV wake / input** (`tv/`, `cec.rs`, `wake_tv`, `Signal::TvWaking/TvShowing`) | Actor + `spawn_blocking` | **core**. The hold pause and resume go to the tv as `Pause`. | |
| **TV-set hand-off** (`player/renderer.rs`, HTTP file server on `tv.port`, UPnP AVTransport; semmeltv3 for HEVC/AV1/VP9) | A `Player` backend inside the core | **core**, as a separate backend with no lock shared with the actor | No cage or mpv is involved, and it keeps playing while the tv restarts. It becomes a second implementation of the same "orders in, reports out" seam. |
| **Phone streaming** (`stream/`, ffmpeg HLS) | API | **core** | |
| **Feedback screenshot** (`feedback.rs` runs `grim`) | API | **core**, running `grim` against cage's socket (`WAYLAND_DISPLAY=/run/user/<uid>/wayland-0`) with a timeout | grim talks to cage, not to the tv, so it can also capture a hung tv. |
| **Library, downloads, offline cache, storage, push, auth, deploy** | Core | **core** | Untouched |

## 2. Protocol

### 2.1 Transport: one Unix socket

The core listens on `/run/semmeltv/tv.sock`. The tv connects, and both sides
then send newline-delimited JSON over that connection, one message per line.

I chose this over the HTTP API with `/api/events` because:

- The traffic is two-way and mostly pushed by the core: `Play`, `Pause`,
  `Toast`, `Home`. Over HTTP the core would have to be a client of a second
  server inside the tv, or every order would need a long poll.
- Most messages exist only for the TV: `Home` with the whole library,
  `Key`, `Panel`, `UpNext`, `Together`, `Play` with file paths and subtitle
  memory. None of them belongs in the API that nginx serves to phones, and
  ADR 3 already keeps them off the phones' WebSocket (`#[serde(skip)]`).
- One connection tells the core whether the tv is alive: EOF means gone. HTTP
  requests would leave the core guessing.
- Authentication comes from the filesystem (see §2.2), so there is no token to
  provision or rotate.
- The phones' API keeps its contract and its tests unchanged.

The cost is debuggability: you can't curl it. `socat - UNIX-CONNECT:/run/semmeltv/tv.sock`
shows the JSON lines, and `semmeltv tv --dump` (stage 3) prints them.

### 2.2 Authentication

- The core's unit has `RuntimeDirectory=semmeltv`, mode 0700, owned by
  `semmeltv`. Only that uid, and root, can reach the socket.
- On accept, the core checks `SO_PEERCRED`: the peer's uid must be its own.
  This is a second guard in case the directory mode ever changes.
- Each side sends `Hello`/`Welcome` with a protocol number and the build
  revision (`/etc/semmeltv/revision`). If they differ, the core closes the
  connection and the tv exits with code 3. systemd restarts it, and after a
  switch the restart picks up the new binary.
- One tv at a time. A new connection replaces the old one, so a half-dead tv
  can't keep the slot.

### 2.3 Messages from the core to the tv (`ToTv`)

| Message | When | Notes |
|---|---|---|
| `Welcome { protocol, revision }` | In reply to `Hello` | |
| `Library { revision, library }` | After `Welcome`, and when the library changes | `Library` needs `Serialize`/`Deserialize`. It is a few MB of JSON at most, and only sent when it changes. |
| `Home { library_revision, profile, people, progress, offline, downloaded, downloading, problem }` | Wherever `publish_home` runs today | `Home` without the library. Progress is the TV profile's only. |
| `Key(Command)` | A key the menu handles (today `Bus::key`) | `Command` gets serde. |
| `Message(String)`, `Panel(Option<Panel>)`, `Together(Together)`, `UpNext(Option<UpNextCard { …, remaining_ms, total_ms }>)` | As today on the bus | `total_ms` lets the tv draw how much of the countdown is left. |
| `Play { play, owner, file, source, start, paused, tracks, caption, subtitles: { settings, remembered, sidecars }, audio_device, dialogue, next }` | `play_from` | `play` is the token `OnTv.play` already holds, used to match every later message. `source` is the path mpv opens (cache, ready copy or mount). |
| `Command { play, command }` | A player key | |
| `Pause { play, paused }`, `Mute { play, muted }`, `Speed { play, speed }`, `Seek { play, to \| by }`, `SkipSegment { play }`, `HideSkip { play }` | Controls | Fire and forget. The next `Report` shows the result. |
| `AudioDevice { device, said }`, `Dialogue { on, said }` | Route and filter changes | Apply to the running mpv and to the next start. |
| `Toast { heading, status, hold_ms }`, `HideToast` | `say`, `tell`, voice | Toasts show over the video while something plays; the menu uses `Panel`/`Message`. |
| `Segments { play, segments, credits }` | Once the core has fetched them | |
| `Subtitles { id, play, change }` | `/api/subtitles/*` | Expects `Answered { id }` |
| `Stop { play }` | Stop, handover, a queued play | |

### 2.4 Messages from the tv to the core (`FromTv`)

| Message | When | Notes |
|---|---|---|
| `Hello { protocol, revision, playing: Option<Running>, ended: Option<Ended> }` | On every connect | `Running { play, owner, file, position, paused }` lets a restarted core adopt the playback. `ended` carries a `Finished` that happened while the core was away, so its progress isn't lost. |
| `Report { play: Option<…>, state, place, tracks, played, subtitles }` | Every 500 ms while playing, every 1 s while idle (a heartbeat) | `state` is today's `player::State`. This one message replaces every `state()`, `place()`, `tracks()`, `played()` and `muted()` call. No `at`: the core stamps a report with its own clock when it arrives, so the two clocks never need to agree. |
| `Started { play }`, `Failed { play, error }` | After `Play` | Replaces the `io::Result` that `Player::start` returns today. |
| `Finished { play, finish, place, tracks, played }` | mpv exited | Carries the final place, so the core needs no last read of the player. |
| `Key(Command)` | Keyboard in Slint, or Back/Listen/Dialogue in mpv's window | |
| `Ask(UiRequest)` | `PlayFile`, `WatchWithOnTv`, `TvProfile`, `TogetherOnTv` from the menu | Fire and forget, as `core.send` is today. |
| `Answered { id, message, ok }` | Reply to `Subtitles` | |
| `Remember { file, remembered }` | A subtitle or audio choice that should persist | Today this is the `files` channel inside `Subtitles`. |

### 2.5 The core never blocks

1. **Writing.** The actor calls `link.send(ToTv)`, which is a `try_send` into a
   bounded queue (256). A writer task drains it and gives each `write_all` 2 s.
   A full queue or a timed-out write closes the connection and logs
   `tv: not reading, dropped`. The actor never awaits the tv.
2. **Reading.** A reader task parses lines (at most 16 MB each, otherwise the
   connection is closed) and passes them to the actor as
   `Signal::Tv(FromTv)`, the same way CEC and voice signal today.
3. **Answers.** Only `Subtitles` waits for a reply. It waits on a `oneshot`
   with a 2 s timeout and otherwise returns `Outcome::refused("The TV did not answer")`,
   as `api::ask` already does for the actor's own 5 s.
4. **Liveness.** No message for 5 s marks the link *stale*. The core keeps the
   session and shows `loading` to phones. After 15 s it closes the
   connection. Killing the process is systemd's job (§3.3), not the core's.
5. **Reads come from the core's own state.** `/api/playback`, `/api/events`,
   `/api/screen` and `/api/subtitles` answer from the last `Report` or from
   `Lost`. They never ask the tv.

### 2.6 When the tv is gone or slow

The core keeps `OnTv` as today, with `player: Player` replaced by
`link: Link`:

| `Link` state | The core does |
|---|---|
| `Playing { last: Report }` | As today, from `last`. |
| `Stale { last, since }` | Same, but `NowPlaying.loading = true`. Orders are still queued. |
| `Lost { last, since, resumes }` | Saves the place from `last` at once. Controls answer `refused("The TV app is restarting")`. Phones see `loading`. Voice still works: "play X" queues X for when the tv returns. |

When the tv says `Hello`:

- **No playback, and a session is `Lost` for less than 2 minutes and not
  `Hold::Stopping`.** The core sends `Play` with the same `play` and owner,
  `start = last.position - 3 s`, and the paused state it had. The toast says
  "Picked up where it stopped". At most 2 automatic resumes per `play` in 10
  minutes. After that it stops and says "This video stopped the TV twice", so
  a file that crashes mpv can't make a crash loop.
- **No playback, and the session is older than that.** The core finishes it
  as `Finish::Stopped` (`watching::stopped`, usage noted), as if Back had been
  pressed.
- **`playing: Some(running)`** (the core restarted while the tv kept
  playing). The core rebuilds `OnTv` from `running` (`shown` from the library,
  `Hold::default()`) and carries on. With `ended: Some`, it records that
  finish first.

While disconnected, the tv:

- keeps playing, applies its own keys locally (the player's `command`, menu
  navigation on the cached `Home`), and shows `Panel("SemmelTV is restarting…")`
  for anything only the core can do;
- reconnects every 0.5 s, backing off to 5 s;
- keeps at most one `Ended` to report in its next `Hello`.

## 3. Processes and systemd

```
           semmeltv-mount.service (rclone)        semmeltv-sync
                       │ After/Wants                    │ writes meta/
                       ▼                                ▼
 nginx ──HTTP──▶ semmeltv.service  "semmeltv core"   (User=semmeltv, Type=notify,
 :443/LAN        API :8090, DB, library, downloads,   WatchdogSec=30, Restart=always)
 phones          CEC, UPnP, voice, BlueALSA, LED,
                 TV-set renderer :tv.port
                       │  /run/semmeltv/tv.sock  (NDJSON, uid-checked)
                       ▼
                 cage-tty1.service  "cage -- semmeltv tv"  (User=semmeltv,
                 Slint, mpv (+ its IPC socket),        WatchdogSec=15,
                 wlr-randr                             NotifyAccess=all, Restart=always)
```

### 3.1 Units

- **`semmeltv.service`** (new, in `modules/kiosk.nix`):
  `ExecStart = semmeltv core`, `User=semmeltv`, `Type=notify`,
  `WatchdogSec=30`, `Restart=always`, `RestartSec=1`,
  `RuntimeDirectory=semmeltv`, `RuntimeDirectoryMode=0700`,
  `After/Wants=semmeltv-mount.service` (moved from `cage-tty1` in
  `hosts/mount.nix`), `wantedBy = multi-user.target`. It gets the
  supplementary groups of the `semmeltv` user (video, render, input, audio,
  systemd-journal) for `/dev/cec*`, the LED, ALSA and the journal.
  `READY=1` is sent once the API listens and the actor runs. `WATCHDOG=1` is
  sent from the actor's 500 ms playback tick, so an actor stuck in an `await`
  or a `block_in_place` gets restarted within 30 s.
- **`cage-tty1.service`** (exists): `services.cage.program = semmeltv tv`.
  Add `WatchdogSec=15` and `NotifyAccess=all`, and keep `Restart=always`. The
  tv sends `WATCHDOG=1` from a Slint `Timer` on the UI thread, and only while
  every other tv thread has checked in within 5 s: the link reader, and while
  playing the player's 100 ms tick, which takes the `Playback` lock. A frozen
  UI thread, a stuck player lock or a dead link stops the pings, and systemd
  kills the whole cgroup: cage, the tv and mpv. `After/Wants=semmeltv.service`
  only orders startup. It has no `BindsTo`/`PartOf`, so restarting either unit
  leaves the other running.
- The polkit rule that lets `adminUser` restart `cage-tty1` also covers
  `semmeltv.service`.

### 3.2 Environment split

| Core (`semmeltv.service`) | tv (`services.cage.environment`) |
|---|---|
| `SEMMELTV_LIBRARY`, `_META`, `_CACHE`, `_DB`, `_FEEDBACK`, `_STREAM_DIR`, `_VFS_CACHE*`, `_REMOTE`, `_READY` | `SEMMELTV_META` (artwork), `SEMMELTV_DECODER` |
| `SEMMELTV_API`, `_ORIGINS`, `_SERVICE_TOKEN_FILE`, `_PHONE_MICROPHONE` | `FONTCONFIG_FILE`, `XCURSOR_THEME`, `WLR_LIBINPUT_NO_DEVICES` |
| `SEMMELTV_SPEECH`, `_WAKE`, `_WAKE_WORD`, `_MICROPHONE`, `_LED` | `SEMMELTV_TV_SOCKET=/run/semmeltv/tv.sock` |
| `SEMMELTV_TV_KIND`, `_TV_RENDERER`, `_TV_DECODES`, `_TV_PORT` | |
| `SEMMELTV_HEADPHONES`, `_DOWNLOADS`, `_GRIM` + `WAYLAND_DISPLAY=/run/user/<uid>/wayland-0` | |

`speech.nix`, `wake.nix`, `phone-mic.nix`, `headphones.nix` and `web.nix` set
`services.cage.environment` today. They move to a new
`services.semmeltv.coreEnvironment` option that feeds the core's unit.

### 3.3 Restarts and what the viewer sees

| What happens | Viewer sees | Mechanism |
|---|---|---|
| tv crashes (panic, segfault, mpv takes Slint down) | ~3 s black, then the film resumes 3 s before where it was | cage exits → `Restart=always` → `Hello` → resume (§2.6) |
| tv hangs (UI thread, player lock, GPU) | Frozen picture for up to 15 s, then as above | watchdog |
| mpv hangs but the tv is fine | Frozen picture. Today nothing notices; see open question 4. | |
| core crashes or hangs | Film keeps playing, remote keys stop for ≤30 s (CEC lives in the core), phones lose the API briefly | core watchdog → adoption via `Hello.playing` |
| deploy (`deploy-rs`) | Both restart, since the binary is shared. The film resumes as after a tv crash. | `restartIfChanged` on both units |
| cage itself hangs | As a tv hang | Same unit, same watchdog |

Startup order: mount → core → tv. The tv still starts if the core is late. It
shows the splash and reconnects until it gets `Welcome`.

### 3.4 Per host

- **semmeltv5** (Pi 5, 8 GB): everything above, with voice, Hailo/parakeet,
  headphones, downloads and the phone microphone on the core's side.
- **semmeltv3** (Pi 3, 1 GB, zram): the TV-set renderer is the main way HEVC
  plays here, and it stays in the core, so a tv restart doesn't interrupt a
  hand-off. Memory: two Tokio runtimes and two copies of `Library`. Measure
  RSS in stage 4 before switching semmeltv3.
- **laptop / `semmeltv ui`**: one process, the in-memory link, the same code
  paths.

## 4. Migration plan

Each step lands on main, passes `cargo nextest` and the VM groups, and can be
deployed. Steps 1–3 change no process layout, so they deploy as today.

**Conflict note.** Steps 1a, 1b and 1d all edit `core/actor.rs` and
`player.rs`, as do most feedback-triage sessions. Run them one after another,
in one session, and ask other sessions to stay out of `actor.rs`'s playback
functions (`play_from`, `record_place`, `publish_playback`, `hear`, `key`,
`control`) until 1b lands.

| Step | Change | Tests added | Parallel? |
|---|---|---|---|
| **0a** | serde for everything that crosses: `Library` (+ `Show`, `Episode`, `Movie`), `Command`, `player::State`, `Place`, `Panel`, `Together`, `subtitles::{Remembered, Sidecar, Playing}`, `segments::Segment`, `up_next` card with `remaining_ms`. Add `src/link/messages.rs` with `ToTv`/`FromTv`. | A round-trip test per message; a test that a generated library of 5,000 episodes serialises under 16 MB | **Yes**, touches no actor code |
| **0b** | Nix: `services.semmeltv.coreEnvironment`; move the env vars there and keep feeding them to cage for now, so nothing changes yet. | `nix build` of both hosts; VM tests unchanged | **Yes** |
| **0c** | VM test group `tv-restart`, marked expected-to-fail until step 4: play, `kill -9` the tv process mid-film, assert `/api/playback` answers in < 1 s throughout and the film resumes within 10 s at ±5 s. A second case sends `SIGSTOP` and asserts the API stays up and the watchdog restarts the tv. | — | **Yes** |
| **1a** | **Reads off the lock.** A reporter thread beside the player sends `Report` every 500 ms (and `Finished` with the final place) into the actor as `Signal::Tv`. The actor replaces every `state/place/tracks/played/muted` call with the last report, and saving moves to `save_due` on reports (drop `on_save`, the TV-set renderer's too: the core saves for both backends, the first time 30 s after the start). | Actor tests with a fake tv end: a report moves `NowPlaying`; the place is saved after 30 s of reported movement; `Finished` saves its own place. Player test: a held `Playback` lock delays reports but `publish_playback` still returns at once. | After 0a |
| **1b** | **Orders off the lock.** A `TvLink` in-process implementation: the actor sends `ToTv` via `try_send` to a tv-driver thread, which owns the `Player` and calls its methods. `Play` → `Started`/`Failed`. Subtitles change → `Answered` with a 2 s timeout. After this step the actor holds no `Player`. | Actor test: a tv end that never reads makes `ask(Seek)`, `ask(Speed)` and `/api/playback` answer within 100 ms, and `Subtitles` within 2.1 s with "did not answer". The 2d02282 scenario as a unit test: the player lock is held forever, and the API still answers. | After 1a |
| **1c** | Subtitles split: core `SubtitleSettings` (DB, owner, per-file memory, sidecar counts); tv `SubtitleTracks` (observe/loaded/change). `Play` carries settings, remembered and sidecars; `Remember` persists. `/api/subtitles` reads `Report.subtitles`. | Existing subtitles tests split in two; a round-trip test that a track chosen in the tv is remembered for the next `Play` of that file | After 1b. Mostly `subtitles.rs`, so it can overlap with 1d. |
| **1d** | Segments move to the core: fetch on the first `Report` with a duration, send `ToTv::Segments`. The player loses `ureq`. | Actor test: segments are fetched once per play and sent with the right `play` token | After 1b. Parallel with 1c. |
| **1e** | Renderer: the steering thread sends `Seen` as `Signal::TvSet`. The actor keeps it like a `Report`. The renderer backend leaves `Player`. | The renderer's existing session tests, plus one where `seen` is never locked by the actor | After 1b. Parallel with 1c and 1d (different functions, `renderer.rs`). |
| **2** | **Menu over the link.** `Home`, `Key`, `Message`, `Panel`, `UpNext`, `Together` become `ToTv`; `ui.rs` sends `FromTv::Ask`/`Key` instead of `core.send`. The bus drops its `#[serde(skip)]` variants. `ui::run` no longer gets the core's runtime handle. | The existing "a new library reaches both frontends" and "a remote key goes to the screen" tests rewritten against the link; a test that the library is sent only when its revision changes | After 1b. Parallel with 1c–1e if it keeps to `ui.rs`/`bus.rs`. |
| **3** | **The socket.** `link::socket`: the core's listener (peer uid, single slot, bounded writer, timeouts, stale/lost), the tv's client (reconnect, `Hello`, local keys while disconnected). Subcommands `semmeltv core` and `semmeltv tv`; `semmeltv ui` keeps the in-memory link. `semmeltv tv --dump`. | Socket tests on a temp dir: wrong uid refused (skipped when not root); a second client replaces the first; a reader that stops makes the core drop it within 2 s without blocking `send`; a revision mismatch closes it; a 16 MB line limit. Recovery tests (§2.6): a resume after `Lost`, giving up after 2 minutes, the crash-loop guard, adoption from `Hello.playing`, `Hello.ended`. | After 2 |
| **4** | **Two units.** `semmeltv.service` + `cage-tty1` running `semmeltv tv`, env from 0b, sd_notify (`READY`, `WATCHDOG`) in both, polkit, `hosts/mount.nix` ordering. Deploy to semmeltv5 first; semmeltv3 after a day and an RSS check. | VM `tv-restart` (0c) now passes; all VM groups pass with the split; a VM case that stops `semmeltv.service` mid-film and checks the film keeps playing and is adopted. | After 3 |
| **5** | Clean-up: drop `Core::bus()` uses by `ui.rs`, the leftovers of `block_in_place` around playback, and the README/ADR text. Write ADR 4 from this doc. | — | After 4 |

How 1b came out, where it differs from the table above:

- The in-process tv end is `link::local`: a thread per link that owns the
  player and turns each `ToTv` into a `Player` call. Only it ever waits on
  the `Playback` lock. `Link::send` is a `try_send` into a queue of 256; a
  full queue drops the order and logs `tv: not reading, dropped <kind>`.
  There is no connection to close in-process, so closing it waits for the
  socket in step 3, as do *stale* and *lost*.
- The TV-set hand-off gets a `link::local` of its own for each play until
  1e, so the actor holds no `Player` for it either. Dropping that link
  stops the TV set, as dropping the player did.
- What `play_from` did after a successful start (waking the TV, recording
  the start, the download lead) now happens on `Started`. `Failed` clears
  the playback and shows "The video did not start: …".
- `Stop` quits the player. Before, Stop sent Back, which only hid the skip
  pill when one showed.
- Orders apply only to the play they name, so a late order can't reach the
  next video. `AudioDevice`, `Dialogue` and toasts go to whatever plays.
- `Play.subtitles` carries the owner's settings and the file's remembered
  delay; `sidecars` stays empty and the tv still reads the shared
  `Subtitles` until 1c.
- The skip pill and hide-skip answer from the last report, and speed is
  clamped in the core (1× while the TV set plays).

How 1d came out:

- The core looks segments up through `Hardware::segments`, a
  `segments::Finder` (theintrodb.org in `semmeltv ui`, a fake in tests), on a
  blocking task, so the actor never waits on the internet. The answer goes
  straight down the play's link as `ToTv::Segments`; the actor keeps no copy,
  so when step 3 resumes a play on a new tv, it sets `OnTv.unsegmented`
  again and the next report looks them up anew.
- Nothing is looked up for a hand-off to the TV set, which shows no skip pill.
- `Play.title` is gone and `PROTOCOL` is 2.

How 1e came out:

- `Player` is mpv only. The TV set's `renderer::Session` is the other
  backend of `OnTv`, beside the tv's `Link`. It takes the same `ToTv`
  orders (`Session::take`) through a `try_send` into a queue of 256, and
  ignores the ones a TV set has no use for. Dropping it stops the TV set.
- The steering thread tells the core `Signal::TvSet { play, told }`:
  `Started` once the file is served, `Failed` if it can't be, `Seen` every
  500 ms while the TV set plays and right after each order it carries out,
  and `Ended`. The actor turns `Seen` into a
  `Report` and `Ended` into an `Ended`, so both backends share `reported`
  and `ended`. Nothing shares the `Seen`: it is only ever copied.
- Serving the file (the local address, the HTTP listener) moved onto the
  steering thread, so starting a hand-off costs the actor nothing; a port in
  use comes back as `Failed`.
- Subtitle changes while the TV set plays are refused by the core at once
  ("choose them with the TV's remote") instead of a round trip to the tv end.

Steps 0a, 0b and 0c can start today in three sessions. 1a → 1b is the
critical path. 1c, 1d, 1e and 2 can then go in parallel if each stays inside
its listed files.

## 5. Risks and open questions for Frederik

1. **Automatic resume after a tv crash**: resume on its own 3 s back (proposed),
   or show "Continue from 41:12?" first?
2. **A deploy restarts the film**, since both units share the binary. Should a
   core-only change leave the tv running (`restartIfChanged = false` on
   `cage-tty1`, with the tv restarted only when the protocol number changes)?
   It saves an interruption per deploy but adds a skew case. Proposed: not now.
3. **Remote keys depend on the core** (CEC lives there). While the core
   restarts, the TV remote does nothing for up to ~30 s, but the film plays
   on. Moving CEC keys into the tv would fix that but split the remote's path.
   Proposed: keep CEC in the core.
4. **A frozen mpv picture with a healthy tv** isn't caught by either watchdog.
   The core could notice from `Report`s: position stuck, not paused, not
   `paused-for-cache`, for 30 s. It would then restart the player at the same
   place. Proposed as a follow-up after step 4.
5. **Bluetooth and D-Bus outside a logind session.** The core no longer runs
   in cage's seat session. bluetoothctl, BlueALSA and any `uaccess`
   device ACLs must work for a system service with `User=semmeltv`.
   **Answer (step 0b, from the code and the NixOS modules): nothing the core
   uses depends on the session.** With `User=` systemd applies the user's
   supplementary groups (video, render, input, audio, systemd-journal), and
   every device and bus name the core touches is granted by a group or to
   everyone:

   | The core uses | Granted by | Session needed? |
   |---|---|---|
   | `bluetoothctl` (`info`, `devices`, `connect`, `scan bredr`) → `org.bluez` | bluez's `bluetooth.conf`: `<policy context="default"><allow send_destination="org.bluez"/>`; its callbacks into bluetoothctl's agent are allowed by the `user="root"` policy. bluez asks no polkit. | No |
   | `bluealsa-cli` (`list-pcms`, `monitor`, `volume`, `delay-adjustment`) → `org.bluealsa` | bluez-alsa's D-Bus policy: `<policy group="audio">`. dbus-broker takes the peer's groups from `SO_PEERGROUPS`, so the unit's supplementary groups count. | No |
   | `arecord -D plughw:…` (voice mic, `SEMMELTV_MICROPHONE`), `aplay` into `plughw:Loopback,0,0` (phone mic) | `/dev/snd/*` is `GROUP="audio"` (udev `50-udev-default.rules`). `uaccess` adds an ACL on top for the seat's user, which the core no longer gets and does not need. | No |
   | `/dev/cec*` (`cec_linux`) | `SUBSYSTEM=="cec", GROUP="video"`; not tagged `uaccess` at all | No |
   | The ACT LED (`SEMMELTV_LED`) | `semmeltv-led.service` chgrps `brightness` to video | No |
   | `/sys/class/drm/*/status`, thermal, `get_throttled` | world-readable (`status` writable by video through kiosk.nix's udev rule) | No |
   | `grim` (feedback screenshot) | cage's socket `/run/user/<uid>/wayland-0`, a 0700 directory owned by semmeltv | Exists only while cage's session is open, so a screenshot fails while the tv is restarting and works while it hangs. The core's unit must not set `ProtectHome`, which hides `/run/user`. |
   | `semmeltv-downloads` and `semmeltv-mount` | already run as `User=semmeltv` outside any session | — |

   The `uaccess` ACLs (sound, `drm` cards, `rfkill`, video4linux, media) stay
   with cage's session, where mpv and Slint need them for the decoders and
   the GPU. The core opens none of `/dev/dri`, `/dev/video*`, `/dev/media*`
   or `/dev/rfkill` (bluetoothd handles rfkill as root), and `ffmpeg` in
   `stream/` encodes in software. `XDG_RUNTIME_DIR` is unset in the core's
   unit; only the mpv IPC socket uses it, and that moves to the tv.

   Two session effects to keep in mind. (a) logind's default `RemoveIPC=yes`
   applies to semmeltv, a normal user: every time cage's session closes (each
   tv restart, each deploy), logind deletes all SysV and POSIX IPC of the uid,
   including the core's. The core creates none today (plain `plughw` devices,
   no `dmix`/`dsnoop`, the stream dir is on disk, not `/dev/shm`); if that
   changes, set `services.logind.settings.Login.RemoveIPC = false`. (b)
   `KillUserProcesses`/`KillOnlyUsers=semmeltv` kills only the session's
   scope, not `semmeltv.service`.

   `semmeltv hear` is not a check of any of this: it reads a WAV file and
   talks TCP to the speech server. The TV-side check is to run each command
   above under `systemd-run --uid=semmeltv --pipe --wait` (a transient
   service, which gets the same groups and no session) and expect them to
   behave as they do from cage.

   **Checked on semmeltv5 (2026-11-22)**, under `systemd-run --uid=semmeltv`:
   - Groups: audio, video, systemd-journal, input, render and rqbit.
   - `bluetoothctl show` worked, and `devices Bonded` listed the AirPods.
   - `bluealsa-cli list-pcms` came back empty with no AccessDenied; the
     headphones weren't connected.
   - `aplay` into `Loopback,0,0` worked.
   - `/dev/cec0` and `/dev/cec1` opened, and the LED was writable.
   - grim captured 1 MB.

   Only `arecord` on `plughw:Loopback,1,0` failed, with "Device or resource
   busy". That is contention, not permission. The app under cage holds that
   device all the time: the actor's `voice::spawn` with
   `Source::Microphone(SEMMELTV_MICROPHONE)` keeps an `arecord` open on it, and
   `snd-aloop` is loaded with `pcm_substreams=1`, so there is one capture
   substream. The wake word and speech services get their audio from the app
   over TCP and open no ALSA device. After step 4 only the core runs that
   `arecord`, so nothing contends. No `SupplementaryGroups=` is needed.
6. **Memory on semmeltv3**: two processes, each holding the library. If RSS is
   too high, the tv could keep only what the menu draws. Measure first.
7. **The renderer stays in the core.** A hung UPnP call is on its own thread
   and touches no shared lock after step 1e, but it is still core code talking
   to a flaky device. The alternative, a third process, is not proposed.
