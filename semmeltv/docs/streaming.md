# Watching on several devices at once

Date: 2026-11-22. Owner: the `semmeltv-streaming` session. Other sessions
read this before they change `src/stream/` or `src/challenge/`.

This doc covers how many phones and laptops can watch at the same time from
one TV, why, how that is enforced, and the bake-off: the challenge for a
place when every place is taken.

## In short

- On semmeltv5, the Pi's CPU and the home's upload are never the limit. The
  SD card is. A film that isn't downloaded to the TV streams from pCloud
  through rclone's VFS cache, and rclone writes every byte it fetches to the
  card. The card takes about 270 Mbit/s (33 MB/s) of writes. Past that,
  streams take 30–60 s to start and stall.
- Films that are downloaded cost almost nothing. Eight of them stream at
  once, locally or from outside, without a stall.
- Limits (`src/stream/mod.rs`): **at most 8 phones and laptops at once**, and
  **at most 3 of them on films that aren't downloaded**. The TV playing a
  film that isn't downloaded counts as one of those 3.
- When a limit is hit, the phone shows "Der Ofen ist voll" and offers a
  **bake-off** against someone watching. The winner keeps or takes the place;
  the loser stops and rests 10 minutes before challenging again.

## 1. How a phone stream works

A phone or laptop calls `POST /api/stream/open`. The core probes the file,
starts one ffmpeg per stream (`src/stream/remux.rs`), and answers once the
init segment and three media segments are on disk. ffmpeg:

- reads the file through the rclone mount (`hosts/mount.nix`). On semmeltv5
  the mount runs `--vfs-cache-mode full --vfs-read-ahead 256M`: every byte
  read from pCloud is also written into the VFS cache on the SD card, which
  doubles as the 60 GiB offline cache;
- copies the video (`-c:v copy`). It copies the sound when the browser plays
  it (AAC always; E-AC-3 and AC-3 on Safari) and otherwise encodes it to AAC;
- reads the first 60 s of film as fast as it can (`-readrate_initial_burst
  60`), then at twice the playing speed (`-readrate 2`);
- writes fMP4 HLS segments of ~6 s into `SEMMELTV_STREAM_DIR`, which is
  `/var/lib/semmeltv/stream` on the SD card, and keeps every segment until the
  stream closes.

The phone plays with hls.js (or Safari's own HLS) and keeps 30 s ahead.
Viewers outside the house come in over the home's upload: the public address
`semmeltv5.semmeltv.ch` (router forward to nginx) or Tailscale.

When the TV plays a film that isn't downloaded, the TV adds its own load on
top of the phones. mpv streams the film through the same mount, and the
offline cache downloads the whole file in the background, because the film
the TV plays is the cache's lead (`actor::started` → `watching::downloads`).

## 2. Measurements

### Method

The tools are in `tests/load/`:

- `viewers.py` plays N viewers at once, the way phone.js's hls.js does: it
  opens a stream, starts playing after the first segment, keeps 30 s ahead,
  reads the EVENT playlist again every 2 s while the next segment isn't
  listed, and counts a stall whenever the playhead reaches the end of what it
  has. It reports per viewer how long the open took, the stalls and their
  seconds, and the Mbit/s it fetched.
- `farm.py` runs on the TV as the `semmeltv` user. It starts ffmpeg with
  exactly the app's arguments and serves the segments, without the app's
  limit, so N can go past 3. At N = 1 and 3 it was checked against the
  app's own API.
- `sample.py` samples the TV every 5 s: CPU per process, iowait, how busy the
  SD card is, card reads and writes, LAN and Tailscale traffic, free memory.
- `tv.py` runs the TV-playing test in one go. It starts a film on the TV as a
  test profile, mutes mpv over its socket as soon as the socket answers
  (0.27 s), runs phases of extra streams, reads mpv's drop counters and the
  player's health lines, and stops the film.

Runs lasted 120 s each, with the viewers opening 1 s apart. "Heavy" files are
the eight heaviest 1080p films not downloaded to the TV (5–10 Mbit/s: Guardians
of the Galaxy Vol. 3, The Endless Summer, Gladiator, The Princess Diaries,
Anyone but You, Ice Age, Saving Grace, Fantastic Mr. Fox), each viewer at a
random place, so rclone's cache never had the part already. "Cached" files
are downloaded ones (The Last of Us S02, Game of Thrones S01, Harry Potter 1;
2–4 Mbit/s). The clients ask for AAC, so the sound is re-encoded whenever the
file's isn't AAC: the CPU's worst case. Remote runs went from beast (another
network) over Tailscale. A test profile signed the API in and was deleted
afterwards with its sessions, rows and cache files.

The library is mostly 1080p at 2–3 Mbit/s; the heaviest file is 10 Mbit/s.
The only 4K film (Now Days, HEVC 10-bit) is 6 Mbit/s, so in this library 4K
behaves like a heavy 1080p film. There are no high-bitrate 4K remuxes to test.

### Results

| Run | N | Worst open | Stalls | Served Mbit/s | CPU busy, avg/peak (of 4) | iowait | Card busy | Card writes, avg/peak Mbit/s | From pCloud Mbit/s | Free RAM |
|---|---|---|---|---|---|---|---|---|---|---|
| API, public address, Guardians (10 Mbit/s) | 1 | 5.2 s | 0 | 16 | 0.4/2.1 | 0.0 | 0% | 86/261 | 82 | 4.2 GB |
| API, public address, Guardians | 3 | 3.5 s | 0 | 37 | 0.7/3.3 | 0.0 | 0% | 62/193 | 56 | 4.2 GB |
| heavy, segments on the card | 1 | 9.3 s | 0 | 7 | 0.3/0.9 | 1.0 | 57% | 132/270 | 124 | 4.0 GB |
| heavy, segments on the card | 2 | 32.4 s | 0 | 17 | 0.4/1.5 | 2.0 | 63% | 140/272 | 102 | 4.1 GB |
| heavy, segments on the card | 3 | 3.9 s | 0 | 27 | 0.3/1.2 | 1.5 | 55% | 130/277 | 123 | 4.0 GB |
| heavy, segments on the card | 4 | 2.9 s | 0 | 27 | 0.5/2.1 | 1.1 | 39% | 90/264 | 78 | 4.1 GB |
| heavy, segments on the card | 6 | 33.9 s | 2 (35 s) | 46 | 0.5/2.1 | 2.6 | 71% | 161/268 | 144 | 4.0 GB |
| heavy, segments on the card | 8 | 48.0 s | 1 (2 s) | 75 | 0.8/1.8 | 2.5 | 69% | 161/243 | 130 | 3.7 GB |
| heavy, segments in RAM | 3 | 4.6 s | 0 | 18 | 0.2/1.1 | 0.5 | 20% | 45/238 | 46 | 3.8 GB |
| heavy, segments in RAM | 6 | 3.5 s | 0 | 54 | 0.7/2.9 | 0.5 | 25% | 45/241 | 69 | 2.6 GB |
| heavy, segments in RAM | 8 | 63.6 s | 2 (26 s) | 82 | 1.1/3.2 | 2.0 | 65% | 136/277 | 149 | 2.3 GB |
| cached, local | 3 | 3.8 s | 0 | 15 | 0.7/1.8 | 0.1 | 13% | 25/119 | 0 | 4.2 GB |
| cached, local | 6 | 3.1 s | 0 | 19 | 0.9/2.4 | 0.4 | 19% | 30/125 | 0 | 4.2 GB |
| cached, local | 8 | 3.2 s | 0 | 31 | 1.0/3.0 | 0.6 | 26% | 44/185 | 0 | 4.1 GB |
| cached, remote over Tailscale | 3 | 2.9 s | 0 | 14 | 1.0/2.2 | 0.2 | 15% | 24/82 | 0 | 3.6 GB |
| cached, remote over Tailscale | 6 | 7.9 s | 0 | 19 | 1.2/2.6 | 0.5 | 26% | 42/209 | 6 | 3.1 GB |
| cached, remote over Tailscale | 8 | 2.7 s | 0 | 30 | 1.2/3.3 | 0.9 | 33% | 50/182 | 16 | 2.6 GB |

The app gives up on an open after 45 s (`remux::WAIT`).

**The TV playing.** The TV played Guardians of the Galaxy Vol. 3 (H.264, which
the Pi 5 decodes in software at ~0.8 core, the worst case for the TV), muted,
next to heavy streams:

| Extra streams | TV frames dropped / late | Streams: worst open, stalls |
|---|---|---|
| 0 | 0 / 0 | – |
| 1 | 0 / 0 | 3.7 s, none |
| 3 | 4 / 3 | 18.9 s, 2 stalls (34 s) |
| 6 | 7 / 9 | 52.2 s, 3 stalls (13 s) |
| 8 | 65 / 36 | 41.9 s, 3 stalls (7 s) |

**Home upload.** Eight parallel downloads from the TV to beast over Tailscale
reached 335 Mbit/s. Eight streams of the heaviest file (10 Mbit/s) need ~80.
tailscaled used at most 0.3 core.

### What limits what

- **CPU**: never. Even 8 streams with every sound track re-encoded, or the TV
  decoding H.264 in software beside them, peaked at 3.3 of 4 cores, and only
  for the seconds in which streams open.
- **Memory**: ~4 GB of 8 stays free with segments on the card.
- **Upload**: at least 335 Mbit/s, four times what 8 heavy streams need.
- **SD card writes**: the limit. A stream of a film that isn't downloaded makes
  rclone fetch the first 256 MB at once (`--vfs-read-ahead`) and write it to
  the card. ffmpeg then reads at twice the playing speed, and its segments go
  to the card as well. With 2 or more such streams opening, the card sits at
  its write ceiling: the starts wander between 3 and 50 s, and from 6 on they
  stall. Moving the segments to RAM helps up to 6, but at 8 rclone's writes
  alone fill the card.
- **Downloaded films** don't touch pCloud. Their reads barely load the card,
  and 8 run as well as 1.

## 3. The limits

Hard limits in `src/stream/mod.rs`, checked by `room()` on every open:

1. `MOST_STREAMS = 8` phones and laptops at once. Measured clean at 8 with
   downloaded films, locally and remotely; more would put the CPU near its
   ceiling while streams open.
2. `MOST_FROM_PCLOUD = 3` of them on films that aren't downloaded. The TV
   counts as one of them while it plays a film that isn't downloaded. On
   semmeltv5 today that is "the TV plus two phones" or "three phones", the
   most the card carried without long starts or stalls. Downloaded films don't
   count against this one.

A place won in a bake-off counts as taken until its winner opens their stream
(at most 90 s). A device that opens a new stream replaces its own old one
first. The TV-set hand-off (the Samsung's own player, or semmeltv3) is not a
phone stream and never counts.

Before 2026-11-22 the limit was 3 streams, and 1 while the TV played anything.

## 4. The bake-off

### The game: Semmelstechen

The name lives in one place, `GAME` in `src/challenge/bake.rs`. The API
sends it with every answer (`game`), and the phone and the TV show it from
there.

A one-button game of about 10 seconds: a tap on a phone, the space bar or a
click on a laptop, OK on the TV remote.

1. Three Semmeln go into the oven one after the other. Each one browns from
   pale dough through golden to burnt, along a bar with a golden band. Each
   turns golden after a random 1.6–2.6 s; the oven, drawn from one seed, is
   the same for both players. A Semmel burns 0.6 s after its golden moment and
   comes out by itself. After it there is a 0.5 s rest.
2. The player pulls each Semmel out with their one button. **Crust** (skill)
   = 100 × (1 − (error / 250 ms)²), at least 0, where the error is how far the
   press was from the golden moment. A burnt Semmel has no crust.
3. **Rise** (luck): the yeast. The server draws 0–80 per Semmel for each
   player. The result screen shows it as how much each Semmel rose.
4. Score = crust + rise over the three Semmeln. The higher score wins; a tie
   goes to the person challenged.

Both players bake at the same time, each on their own device, after a 3 s
countdown. The game runs on each device's own clock. The device measures each
press against its own animation and sends the press times in milliseconds
after each golden moment. The network's delay therefore never enters a score;
it only shifts when the bake starts on each device. The server keeps the oven
and the yeast, scores, and decides (`src/challenge/`).

### Half skill, half luck

"50% skill" means: over typical pairs of players, the score differences that
come from pressing (the crust) and those that come from the yeast (the rise)
vary by the same amount. The rise's range is set to make those two variances
equal. The simulated players differ in how much their presses scatter
around the moment they aim for: lognormal, median 70 ms, 5–95% between 34 and
150 ms. That is the variable error usually reported for timing a predictable
moving target, wider for untrained players.

The simulation is a test in `src/challenge/bake.rs`, with hard bounds over
40,000 games:

| Assertion | Bound | Value |
|---|---|---|
| skill's share of the score variance | 0.45–0.55 | 0.50 |
| a 40 ms player beats an 80 ms player | 0.58–0.70 | ~0.64 |
| a 35 ms player beats a 150 ms player | 0.78–0.92 | ~0.85 |
| the same player, remote against touch, at 40/70/100 ms | 0.48–0.52 | 0.50–0.51 |
| the same player, keys against touch | 0.48–0.52 | ~0.50 |

### Fair on every device

An input is unfair in two ways, and the game corrects both:

1. **Lateness.** Between the golden frame being shown and the press being
   registered sits the display's lag plus the input's lag: on a phone about
   50 ms (touch sampling, a frame or two). On the TV, the remote's IR goes to
   the Samsung, which sends a CEC "User Control Pressed" frame. That frame is
   3 blocks of 10 bits at 2.4 ms each plus a 4.5 ms start bit, about 77 ms on
   the bus alone. Add the TV's own processing and its display lag: an
   estimated ~220 ms. Without correction, a player on the remote would press
   that much late every time; the simulation shows the remote winning under
   12% for the same player. So the oven subtracts each input's lateness from
   its presses. It starts from those estimates and **learns** them: the median
   signed error of the last 60 presses per input kind, once it has 15
   (`Calibration`), clamped to −100…400 ms and kept in the `settings` table
   (`bake-off calibration`). A player who adapts to the lag and presses early
   just moves the median.
2. **Wobble.** An input whose lateness varies from press to press scatters a
   player's presses more. Because the crust falls with the square of the error,
   a wobble with standard deviation σ costs on average exactly 100 × σ² /
   250² points per Semmel. Each input gets exactly that back (`wobble()`):
   ~2 points per Semmel for the remote's estimated 35 ms, 0.1 for touch's 8 ms.

The remote's numbers are estimates until the TV side is built. Then a
two-minute measurement with Frederik replaces them:
- he taps a steady beat on the phone and then on the remote. The difference
  in the variance of the intervals between presses gives the remote's wobble;
- he presses the phone and the remote at the same moment ten times. The mean
  offset between their arrival gives the remote's extra lateness against
  touch.
The learning in (1) corrects lateness either way. Wobble isn't learnable from
games, so it stays as measured.

### Flow

1. **Full.** `POST /api/stream/open` answers 429 with `full`: a heading ("Der
   Ofen ist voll"), the message naming the limit, the people watching (person,
   device, title, whether they're in a bake-off already), and how long the
   asker's dough still rests. When the limit hit is the pCloud one, only
   people watching films that aren't downloaded are listed, because beating
   anyone else wouldn't free a place.
2. **Challenge.** The phone shows "Who do you want to challenge for a place
   in the oven?" and the list. A pick calls `POST /api/challenges {seat,
   device}`. The challenger then waits ("Waiting for Ramu…", with *Give up*).
3. **Asked.** Every playing phone asks `GET /api/stream/{id}/challenge` every
   2 s. When it is challenged, the video pauses and it shows "Semmelstechen!
   Mami fordert dich zum Semmelstechen heraus…" with *Bake* or *Stop
   watching*, and the time left to answer. There is no decline.
4. **Bake.** *Bake* (`POST /api/stream/{id}/challenge/accept`) starts the
   oven 3 s later for both. Each device sends its presses
   (`/api/challenges/{id}/presses`, `/api/stream/{id}/challenge/presses`).
5. **Done.** Both see the result: who won, and each Semmel's crust and rise.
   A challenger who won gets *Watch*, which opens the film they asked for.
   A defender who won gets *Back to the film*, which plays on if it was
   playing. The loser gets *Close*; a defender who lost has their stream
   stopped by the server.

### Practice

Opening any page of the web app with `#semmelstechen` at the end of the
address (`https://semmeltv5.semmeltv.ch/#semmelstechen`) starts a practice
bake against the **Bäckermeister**. It is the real game on the real oven,
with the server's yeast. The Bäckermeister presses like a typical player (70
ms spread) on a touch screen. Nobody's place is at stake, nobody rests, and
the Bäckermeister's presses don't teach the oven's calibration. The result
offers *Bake again*. There is no button for it anywhere
(`POST /api/challenges/practice`).

### Edges

| Case | What happens |
|---|---|
| The person challenged doesn't answer in 60 s | They lose: stream stopped, 10-minute rest. They were paused anyway. |
| They press *Stop watching* | They lose the same way. |
| They stop watching some other way (close, another film, the stream times out) | The challenger gets the place; nobody rests. |
| The challenger gives up before the bake | Nothing happens to anyone. The defender sees "Nobody baked" and goes back to the film. |
| The challenger leaves during the bake | The challenger loses and rests. |
| Presses missing (the page closed, the phone slept) | Those Semmeln count as burnt once the oven plus 5 s is over. If both are missing, the tie goes to the defender. |
| Two people challenge the same person | The second gets "Ramu is already in a bake-off"; the list marks Ramu that way. |
| Someone challenges while already in a bake-off | "You are already in a bake-off". |
| Someone challenges while resting | The sheet lists who watches without the pick buttons, with "Your dough is still resting: you can challenge again in 7 minutes". |
| Resting | Per person (profile), not per device. Kept in memory: a restart of the core clears it. |
| The winner doesn't open their stream | Their place is kept for 90 s, then freed. |
| The TV | Listed as "Frederik · the TV · Dune" while it plays a film that isn't downloaded and the pCloud limit is the one hit. Whoever holds the remote plays. Losing stops the TV's film and saves its place. |
| The TV-set hand-off (Samsung, semmeltv3) | Never listed and never challenged: it isn't a stream and costs the Pi only serving a file. |

### The TV side

The TV is a seat (`Seat::Tv`) like a stream. It is listed while it plays a
film that isn't downloaded, but only when the pCloud limit is the one hit, and
never during a TV-set hand-off or while the tv process restarts. The core's
actor sets `Arena::tv_can_bake` on every 500 ms tick and ends a TV challenge
as "stopped watching" when the TV's film goes away.

Everything goes over the link, so it works the same with the tv in its own
process (`semmeltv tv`):

- The actor finds the TV's challenge in the arena on every tick. When one
  comes, it pauses the film and sends `ToTv::Bakeoff(Some(view))`, and again
  at each new stage.
- While the bake-off shows, every remote key goes to the tv as
  `ToTv::BakeKey`, never to the film or the menu.
- The tv (`challenge/screen.rs`, `Stand`) draws it as an mpv OSD overlay
  (`Player::draw_bake`, id 5, above the toast) over the paused film. While the
  answer countdown or the oven runs, the tv's link thread redraws it every
  33 ms.
- *Bake* / *Stop watching* are chosen with ◀ ▶ and OK. In the oven, OK pulls
  the Semmel out, timed on the tv's clock when the key arrives there. The tv
  answers with `FromTv::Bakeoff(Accept | DropOff | Presses | Dismiss)`, and
  the actor plays those into the arena with `Input::Remote`.
- The result shows until OK or for 10 s. Then the actor sends
  `ToTv::Bakeoff(None)`, and the film goes on where it paused, or stops
  (saving its place) when the TV lost.

A key pressed on the tv's own keyboard (only the VM and a laptop have one)
goes to mpv and doesn't reach the bake-off; the TV's remote comes through
the core over CEC.

The remote's lateness and wobble are still the estimates above (220 ms, 35
ms) until the two-minute measurement with Frederik.

### Code

- `src/challenge/bake.rs`: the oven, crust, rise, `Input`, calibration and
  wobble, and the simulation tests.
- `src/challenge/mod.rs`: `Arena`, the state machine of challenges, rests and
  kept places. Time comes in as an argument; it owns no threads.
- `src/stream/bakeoff.rs`: the HTTP side, and carrying out the arena's
  effects on the streams.
- `web/src/phone.js` (`class Bakeoff`) and `web/src/phone.css` (`.pp-bake`):
  the phone and laptop UI.
- VM group `bakeoff`: a full oven over the API with a drop-off, a phone that
  challenges and loses and then rests, a phone that is challenged while
  playing and loses its place, a TV that is challenged pauses, shows the bake-off and plays on when the challenger goes.

## 5. Next

1. Segments in `/dev/shm`, keeping only a window behind the playhead, so the
   card no longer carries them. Measured: segments in RAM carried 6 streams of
   films not downloaded cleanly, against ~3 on the card.
2. A smaller rclone `--vfs-read-ahead` (256 MB today), so a stream opening
   doesn't write a quarter gigabyte to the card at once.
3. Re-measure with both, and raise `MOST_FROM_PCLOUD` if the data allows.
