# Two processes: `semmeltv core` and `semmeltv tv`

Date: 2026-11-22. Status: done, and the only layout. SemmelTV runs as these
two processes on semmeltv5, in the VM tests and on a laptop
(`semmeltv-run`). This supersedes the "core owns the player" part of
[ADR 3](adr/0003-core.md); the rest of ADR 3 (one actor, typed requests,
snapshots) stays.

## Goal

> All the mpv-specific stuff should not come from the core. It should be part
> of a separate Slint UI part of the application that deals with mpv and the
> Slint UI. If that freezes, the core should not freeze. I want the core to be
> stable and have a high uptime rate. (Frederik, 2026-11-22)

- **`semmeltv core`**: the API and website backend, the database, the library,
  downloads, push, sessions and sign-in, TV control (CEC, UPnP volume,
  Samsung pairing), headphones, and the decision of what plays for whom from
  where. Its own systemd service, `semmeltv.service`, outside cage.
- **`semmeltv tv`**: the Slint screen, mpv, display modes, the OSD, mpv's
  audio device. Runs under cage in `cage-tty1`. Talks to the core over one
  local socket.

The core never blocks on the tv, never shares memory with it, and never waits
for it without a deadline. If the tv hangs or crashes, it is restarted and
the core tells it what was playing.

## Why

On 11-21 at 21:59 the TV froze on a display mode switch and the web app went
down with it until the power button. The cause (fixed in 2d02282) was a
deadlock between the player's threads and mpv's IPC socket that held the
player's `Mutex<Playback>` forever. The actor called into `Player` at 44
places, each taking that lock, so the actor stalled and every API request
queued behind it. The structure that turned one stuck mpv into a dead website
was the core holding the `Player` in its own process. Now the next stuck lock,
whether a Slint bug, a GPU driver hang or an mpv freeze, takes only the tv.

## Decisions in brief

1. One binary, two subcommands: `semmeltv core` and `semmeltv tv`. Both come
   from the same Nix store path, so a deploy always runs matching builds.
2. They talk over one Unix socket, `/run/semmeltv/tv.sock`. The tv connects,
   and the core sends orders down the same connection. Messages are
   newline-delimited JSON. Only uid `semmeltv` can connect.
3. The core keeps no handle into the tv. It keeps the last `Report` the tv
   sent, a plain value, and sends orders through a bounded queue with
   `try_send`. A full queue or a silent tv closes the connection. Nothing in
   the core waits for the tv without a timeout.
4. Everything that decides something stays in the core: the up-next countdown,
   watching together, segments, position saving, handover, and where the
   sound goes. The tv draws and plays. It also owns the playback logic that
   has to react to a key press at once: scrubbing, the skip pill and toasts.
5. The core has a systemd watchdog fed by the actor. The tv has a watchdog of
   its own inside the process, and the core kills a tv that went silent.
6. If the tv dies mid-film, the core resumes that film on the new tv, 3 s
   before the last reported position. If the core dies mid-film, the tv keeps
   playing and the restarted core takes the playback over.

## 1. Who owns what

| Feature | Owner | Why |
|---|---|---|
| **mpv process, IPC, OSD** (`player.rs`, `player/commands.rs`, `fade.rs`, `health.rs`, `overlay.rs`, `seek.rs`) | **tv**; `link::local` is the tv's thread that owns the `Player` and turns each `ToTv` into a call | This is the code that can hang. |
| **Display modes** (`display.rs`, `wlr-randr`) | **tv** | Needs cage's Wayland and the player's video track. |
| **Slint menu** (`ui.rs`, `nav.rs`, `details.rs`, `heatmap.rs`, `loader.rs`, artwork files in `meta/Artwork`) | **tv**, fed by `ToTv` through `ui::inbox` | Pure drawing. It reads artwork files read-only. |
| **Menu requests**: `PlayFile`, `WatchWithOnTv`, `TvProfile`, `TogetherOnTv` | **core**, reached by `FromTv::Ask` | They change progress, the profile or pairs. |
| **Remote keys** | CEC in the **core** routes them; keys pressed in the tv (keyboard, mpv's window) go to the core as `FromTv::Key` | One path, as in ADR 3. While the core is away the tv handles its own keys (§2.6). |
| **Telling the TV** (`/api/command`, `intent.rs`) | **core** | It works while the tv is gone ("turn the TV off", "play X"). |
| **Headphones** (`audio.rs`, `core/sound.rs`, `/api/audio*`, `/api/volume`) | **core**; the tv only sets mpv's `audio-device` | Profile-owned and DB-backed. |
| **Dialogue mode** | Flag in the **core**, filter in the **tv** | |
| **Up next** (`up_next.rs`) | **core** decides and times it; **tv** draws the card from `remaining_ms` | Choosing the next episode needs the library and progress. |
| **Segments / skip intro** (`segments.rs`) | **core** fetches on the first `Report` with a duration and sends `ToTv::Segments`; **tv** shows the pill and skips on OK | The tv makes no internet calls; OK reacts at once. |
| **Subtitles** (`subtitles.rs`, `sidecar.rs`, `subfetch.rs`) | Settings, per-file memory, sidecars, OpenSubtitles: **core**. Track choice in mpv: **tv**, reported in `Report.subtitles` and persisted with `FromTv::Remember` | |
| **Position saving, usage, watch history** | **core**, from `Report`s and `Finished` | The most it loses is one report interval. |
| **Watch together, phone/TV handover, TV wake and input** | **core** | |
| **Phone streaming** (`stream/`, ffmpeg HLS) | **core** | |
| **Feedback screenshot** (`feedback.rs` runs `grim`) | **core**, against cage's socket | grim talks to cage, not to the tv, so it also captures a hung tv. |
| **Library, downloads, offline cache, storage, push, auth** | **core** | |

## 2. Protocol

### 2.1 Transport: one Unix socket

The core listens on `/run/semmeltv/tv.sock` (`SEMMELTV_TV_SOCKET`). The tv
connects, and both sides then send newline-delimited JSON over that
connection, one message per line, at most 16 MB each.

It is a socket and not the HTTP API because the traffic is two-way and mostly
pushed by the core, most messages exist only for the TV and don't belong in
the API that phones see, EOF tells the core at once that the tv is gone, and
authentication comes from the filesystem. `socat - UNIX-CONNECT:/run/semmeltv/tv.sock`
shows the lines; `semmeltv tv --dump` connects as a tv and prints them (the
real tv loses the link until it reconnects).

### 2.2 Authentication and the build

- `semmeltv.service` has `RuntimeDirectory=semmeltv`, mode 0700, and the
  socket is mode 0600. On accept the core checks `SO_PEERCRED`: the peer's
  uid must be its own.
- The tv says `Hello` with the protocol number (`PROTOCOL`, 4) and its build,
  the path of the running program: in the Nix store it names the build. The
  core answers `Welcome` with its own. A tv of another build exits with 3,
  and `Restart=always` starts the new binary.
- One tv at a time. A new connection replaces the old one, so a half-dead tv
  can't keep the slot.

### 2.3 Messages from the core to the tv (`ToTv`)

| Message | When | Notes |
|---|---|---|
| `Welcome { protocol, revision }` | In reply to `Hello` | |
| `Library { revision, library }` | After `Welcome`, and when the library changes | |
| `Home(Home)` | When the TV's home screen differs from the last one sent | Without the library. |
| `Key(Command)`, `Message`, `Panel`, `Together`, `UpNext(Option<UpNextCard>)` | For the menu | The up-next card carries `remaining_ms` and `total_ms`. |
| `Play(Play)` | A play starts | `play` is the token every later message names; `source` is the path mpv opens; `subtitles` carries the owner's settings, the file's memory and its sidecars. |
| `Command`, `Pause`, `Mute`, `Speed`, `Seek`, `SkipSegment`, `HideSkip`, `Stop` | Controls | Each names its play, so a late order can't reach the next video. The next `Report` shows the result. |
| `AudioDevice`, `Dialogue` | Route and filter changes | Apply to whatever plays and to the next start. |
| `Toast { heading, status, hold_ms }` | `say`, `tell`, download notices | |
| `Segments { play, segments, credits }` | Once the core has them | |
| `Subtitles { id, play, change }` | `/api/subtitles/*` | Expects `Answered { id }` within 2 s. |
| `Bakeoff(Option<View>)`, `BakeKey(Command)` | The bake-off on the TV | |
| `Update(bool)` | Before an update restarts SemmelTV | Shows the update card. |

### 2.4 Messages from the tv to the core (`FromTv`)

| Message | When | Notes |
|---|---|---|
| `Hello { protocol, revision, playing: Option<Running>, ended: Option<Ended> }` | On every connect | `playing` lets a restarted core take the playback over; `ended` is a finish that happened while the core was away. |
| `Report(Report)` | Every 500 ms while playing, and a heartbeat after each second with nothing to say | The core stamps it with its own clock. |
| `Started { play }`, `Failed { play, error }` | After `Play` | |
| `Finished(Ended)` | mpv exited | Carries the final place, the file and the owner. |
| `Key(Command)`, `Ask(UiRequest)` | Keys and asks from the menu or mpv's window | |
| `Answered { id, message, ok }` | Reply to `Subtitles` | |
| `Remember { file, remembered }` | A subtitle or audio choice that should persist | |
| `Bakeoff(Answer)` | A press in the bake-off | |

### 2.5 The core never blocks on the tv

1. **Writing.** The actor calls `link.send(ToTv)`, a `try_send` into a queue
   of 256. One thread forwards it to the connected tv's own queue of 256; a
   full queue or a write that takes 2 s drops the tv (`Signal::TvGone`).
2. **Reading.** A reader parses lines and passes them to the actor as
   `Signal::Tv(FromTv)`.
3. **Answers.** Only `Subtitles` waits for a reply, 2 s, then
   `Outcome::refused("The TV did not answer")`.
4. **Liveness.** No message for 5 s marks the link stale: phones see
   `loading`. After 15 s the core drops the tv and kills it by the pid
   `SO_PEERCRED` gave (same uid, never its own pid). The 15 s count only time
   the core ran steadily: a check over 2 s late starts the silence again, so
   a core that was stopped or starved never kills a healthy tv.
5. **Reads come from the core's own state.** `/api/playback`, `/api/events`,
   `/api/screen` and `/api/subtitles` answer from the last `Report`.

### 2.6 When the tv is gone or slow

`Signal::TvGone` marks the playback lost and saves its place at once. While
it is lost, everything that steers playback (controls, seek, speed, skip,
subtitles) answers "The TV app is restarting", and a play asked for
meanwhile waits in `queued`. Phones see `loading`.

When the tv says `Hello`:

- **Nothing playing, and a playback lost for under 2 minutes.** The core sends
  the same `Play` token again from 3 s before the last reported place, paused
  if it was, and toasts "Picked up where it stopped". It gives up instead,
  finishing the playback as stopped, when a handover was stopping it,
  something waits in `queued` (which then plays), or it was picked up twice
  in the last 10 minutes ("This video stopped the TV twice"). A playback lost
  for 2 minutes also ends on the tick, without waiting for a `Hello`.
- **`playing` with the same token**: the link came back, and the core carries
  on. **Another token**: the core restarted, and takes the playback over with
  the library's caption and segments looked up again. With `ended`, it
  records that finish first.
- **Nothing playing, and the core left a film for an update** (the update
  card): it adopts that as a lost playback and picks it up.

On `Hello` the core also sends the library and the home screen again.

While the core is away, the tv keeps playing, applies its own keys (the
player's commands, menu navigation on the last `Home`), shows the front's
bakery panel with its estimate for anything only the core can do and on the
menu while nothing plays, and drops its reports. It waits up to 60 s for
`Welcome` on the connection it opened, since a core that is still starting
has bound its socket but accepts nothing yet; a core that dies closes the
connection and ends the wait. Then it reconnects every 0.5 s, backing off to
5 s, and keeps at most one `Ended` for its next `Hello`.

### 2.7 The core never waits

The core answers within 100 ms whatever a device or service does. The actor
(`core/actor.rs`) awaits nothing that touches a disk, a device, a subprocess
or the network. Such work runs on a task or thread of its own and comes back
as a `Signal`. toasty's SQLite driver runs rusqlite on the awaiting thread,
so a database call is a blocking call no timeout can cut short: it has to
leave the actor.

| Wait | Where it runs | Test |
|---|---|---|
| A film's file on the pCloud mount (`store.source`, `sidecar::find`) | `core/prepare.rs` on a blocking task, 4 s, then `Signal::Prepared`; too late: "Wir kriegen es gerade nicht gebacken: pCloud antwortet nicht" | `a_mount_that_never_answers_leaves_the_api_answering_and_back_cancels_the_play`, `prepare::tests` |
| mpv that doesn't quit | the player kills mpv 0.8 s after any quit | `player::tests::an_mpv_that_does_not_quit_on_back_is_killed_soon_and_still_counts_as_stopped` |
| Headphones (`bluetoothctl`, BlueALSA) | the `headphones` thread: which are paired comes back as `Signal::Headphones`, preferences go to it; every `bluetoothctl` call has a timeout | `headphones_whose_radio_never_answers_leave_the_api_answering`, `a_start_whose_radio_never_says_what_is_paired_serves_at_once` |
| Deleting a title from pCloud | a task answers the request; `Signal::Removed` tends the cache | `a_removal_from_pcloud_that_never_ends_leaves_the_api_answering` |
| Database writes and reads after startup | `core/saves.rs`: one thread runs the jobs in order on the core's runtime; results come back as `Signal::People`, `WatchWith`, `Reloaded` | `a_database_that_stops_writing_leaves_the_api_answering_and_keeps_every_write`, `saves::tests` |
| The tv | `try_send` into a bounded queue (§2.5) | `a_tv_end_that_never_reads_leaves_the_api_answering`, `a_player_lock_held_forever_leaves_the_api_answering` |
| CEC: wake, on, off, input | `spawn_blocking`, waking guarded by `waking_tv` | |
| Segments, downloads, push, tending the cache, the library scan | tasks and threads of their own | `segments_are_looked_up_once_per_play_and_sent_with_its_token` |

What is left, on purpose:

- **Startup.** `Actor::new` reads the profiles, their progress and the
  downloaded files before the actor serves anything. Every step is named in
  the log, and again every 5 s while it waits.
- **A profile the core has not seen yet.** Its progress and dialogue mode are
  read once, on first use. That's a read of an empty profile in WAL mode, so
  no writer can hold it up.
- **CEC on and off** start one blocking task per request. A CEC adapter that
  hangs keeps those threads, not the actor.

## 3. Processes and systemd

```
           semmeltv-mount.service (rclone)        semmeltv-sync
                       │ After/Wants                    │ writes meta/
                       ▼                                ▼
 nginx ──HTTP──▶ semmeltv.service  "semmeltv core"   (Type=notify, WatchdogSec=30,
 :443/LAN        API :8090, DB, library, downloads,   Restart=always)
 phones          CEC, UPnP volume, BlueALSA
                       │  /run/semmeltv/tv.sock  (NDJSON, uid-checked)
                       ▼
                 cage-tty1.service  "cage -- semmeltv tv"  (Restart=always;
                 Slint, mpv (+ its IPC socket),        watchdog inside the tv)
                 wlr-randr
```

### 3.1 Units (`modules/kiosk.nix`, `modules/web.nix`, `hosts/mount.nix`)

- **`semmeltv.service`**: `ExecStart = semmeltv core`, `User=semmeltv`,
  `Type=notify`, `WatchdogSec=30`, `Restart=always`, `RestartSec=1`,
  `StartLimitIntervalSec=0`, `RuntimeDirectory=semmeltv` mode 0700, after the
  mount, the network and `cage-tty1`. It sends `READY=1` once it serves the
  socket and the API (it exits with the reason when the API can't listen),
  and `WATCHDOG=1` from the actor's 500 ms tick. Its `ExecStop` tells the
  front it restarts, so phones see "Der Ofen wird neu angeheizt".
- **`cage-tty1.service`**: `services.cage.program = semmeltv tv`,
  `Restart=always`, wants `semmeltv.service`. cage's PAM session moves cage
  and the tv into a `session-cN.scope`, outside the unit's cgroup, so systemd
  can't watch the tv. Instead the tv's Slint thread, its post thread and,
  while something plays, the player's 100 ms tick beat (`notify::beat`), and
  a thread in the tv aborts it when one went 15 s without a beat. A tv frozen
  whole is killed by the core (§2.5). `semmeltv tv` marks everything it
  inherited close-on-exec, so mpv doesn't hold cage's pipe and the tv's death
  ends cage.
- **`semmeltv-update-card.service`**: a oneshot part of `cage-tty1`, whose
  `ExecStop` POSTs `/api/restarting` before an update stops the units. The
  core shows the update card, keeps what plays and stops it, and picks it up
  after the restart.
- The polkit rule that lets `adminUser` restart `cage-tty1` also covers
  `semmeltv.service`. The core starts after cage, so the TV shows its splash
  and the "restarting" panel at once.

### 3.2 Environment

`services.semmeltv.coreEnvironment` is the core's environment, plus
`SEMMELTV_TV_SOCKET`. The tv under cage gets only `SEMMELTV_LIBRARY` and
`SEMMELTV_META` from it, the socket, and cage's own (`FONTCONFIG_FILE`,
`XCURSOR_THEME`, `WLR_LIBINPUT_NO_DEVICES`). `semmeltv core` sets
`XDG_RUNTIME_DIR` and `WAYLAND_DISPLAY` to cage's when they are unset, so
grim reaches cage.

### 3.3 Restarts and what the viewer sees

| What happens | Viewer sees | Mechanism |
|---|---|---|
| tv crashes | ~3 s black, then the film resumes 3 s before where it was | cage exits → `Restart=always` → `Hello` → resume (§2.6) |
| tv hangs (UI thread, player lock, GPU) | Frozen picture for up to 15 s, then as above | the tv's watchdog, or the core killing a silent tv |
| core crashes or hangs | Film keeps playing, remote keys stop for ≤30 s (CEC lives in the core), phones lose the API briefly | core watchdog → takeover via `Hello.playing` |
| deploy | The update card, then the film plays on where it was | `semmeltv-update-card`, `restartIfChanged` on both units |

### 3.4 Running it elsewhere

- **The VM tests** (`tests/vm`) run the same units, with the headphones on.
- **A laptop**: `semmeltv-run` in the devenv shell starts `semmeltv core` and
  `semmeltv tv` as two processes on a socket in
  `$XDG_RUNTIME_DIR/semmeltv-run`.
- **The actor's tests** use `link::local` with a fake tv end, or `Link::pair`.

### 3.5 Sessions and permissions

The core runs outside cage's logind session, and nothing it uses needs one.
With `User=` systemd applies the user's supplementary groups (audio, video,
systemd-journal, input, render), and every device and bus name the core
touches is granted by a group or to everyone:

| The core uses | Granted by |
|---|---|
| `bluetoothctl` → `org.bluez` | bluez's D-Bus policy allows everyone; bluez asks no polkit |
| `bluealsa-cli` → `org.bluealsa` | `<policy group="audio">`; dbus-broker takes the peer's groups from `SO_PEERGROUPS` |
| `/dev/cec*` | `SUBSYSTEM=="cec", GROUP="video"` |
| `/sys/class/drm/*/status`, thermal, `get_throttled` | world-readable (`status` writable by video through kiosk.nix's udev rule) |
| `grim` | cage's socket in `/run/user/<uid>`, a 0700 directory owned by semmeltv; it exists only while cage's session is open, so the core's unit must not set `ProtectHome` |

Checked on semmeltv5 under `systemd-run --uid=semmeltv` on 2026-11-22. The
`uaccess` ACLs (sound, `drm`, video4linux) stay with cage's session, where mpv
and Slint need them. Two session effects to keep in mind: logind's
`RemoveIPC=yes` deletes the uid's SysV and POSIX IPC whenever cage's session
closes (the core creates none; if that changes, set `RemoveIPC = false`), and
`KillUserProcesses` for semmeltv kills only the session's scope, not
`semmeltv.service`.

## 4. History

The migration ran in steps on 2026-11-22, each landed on main with its tests:

- **0a–0c**: serde for everything that crosses and `link/messages.rs`;
  `coreEnvironment`; the VM group `tv-restart`.
- **1a, 1b**: the actor reads the last `Report` and sends `ToTv` orders
  through `link::local`, so it holds no `Player`.
- **1c, 1d, 1e**: subtitles split between the core's settings and the tv's
  tracks; segments looked up by the core; the TV set's own player (semmeltv3)
  as a second backend.
- **2**: the menu over the link; the bus carries only the phones' events.
- **3**: the socket (`link::socket`, `link::client`) and recovery.
- **4**: two units, behind an option, on for semmeltv5. The first switch
  (b432185) left the website down for five minutes: database writes ran on a
  runtime of their own and stranded toasty's connections (fixed with the
  writes on the core's runtime), and startup steps now log where they wait.
  Live on semmeltv5 at c45f96f.
- **After the switch**: the core serves without waiting on the Bluetooth
  radio (b0adff1), and the tv opens one connection while the core starts
  (8cffb1a).
- **One layout**: the option and the one-process mode `semmeltv ui` went, the
  Pi 3 left SemmelTV, and with it the hand-off to the TV set's own player.

## 5. Open questions

1. **A frozen mpv picture with a healthy tv** isn't caught by either watchdog.
   The core could notice from `Report`s: position stuck, not paused, not
   `paused-for-cache`, for 30 s, and restart the player at the same place.
2. **Remote keys depend on the core** (CEC lives there): while the core
   restarts, the remote does nothing for up to ~30 s, but the film plays on.
   Moving CEC keys into the tv would split the remote's path.
