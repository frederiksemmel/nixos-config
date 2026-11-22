# SemmelTV VM test

The VM test boots a NixOS VM with the real kiosk module: cage on tty1 with the
Slint app and mpv, the API, and the web app behind nginx on port 8080. A
generated library holds a short series (three one-minute episodes with sidecar
subtitles) and films with embedded subtitle tracks. The test drives the TV with
its keyboard, reads its screen with OCR and asks mpv over its IPC socket where
it is. The phone is Playwright in the test driver: Chromium as an Android phone
and a laptop, and WebKit as an iPhone where no video has to play, since
Playwright's WebKit build has no H.264 or AAC decoder. WebKit drops `Secure`
cookies on plain HTTP, so a phone is signed in by redeeming an invite and
handing the browser the session cookie.

The scenarios are split into groups. Each group is its own check with its own
VM, so nixbuild runs them side by side, and a change can run just the group it
touches:

| Group | Check | What it covers |
| --- | --- | --- |
| `tv-playback` | `semmeltv-vm-tv-playback` | the TV boots into the library, no pointer, Continue Watching, the TV's profile picker, the HEVC hand-off to the TV set |
| `tv-remote` | `semmeltv-vm-tv-remote` | the phone's remote steering the TV: skips keep the sound running, subtitle delay, the fade on pause, an iPhone back from the background |
| `up-next` | `semmeltv-vm-up-next` | an episode that ends on the TV: the countdown, OK, nothing after the last |
| `resume` | `semmeltv-vm-resume` | where the TV resumes: a film after the app restarts, a finished episode starting over, Back cancelling the countdown |
| `segments` | `semmeltv-vm-segments` | Skip intro and Play next from theintrodb segments |
| `streams` | `semmeltv-vm-streams` | the phone's HLS stream: pace and subtitles, ffmpeg's muxer, a seek outside the stream, `semmeltv-stats` |
| `phone-playback` | `semmeltv-vm-phone-playback` | the phone plays the next episode in place, Apple's native HLS path |
| `laptop` | `semmeltv-vm-laptop` | the laptop player's controls, Back after a reload, a laptop that goes quiet, rows and scroll bars |
| `handover` | `semmeltv-vm-handover` | a film moving between phone and TV, a phone stream surviving a TV app restart, the TV/phone switch |
| `live` | `semmeltv-vm-live` | a page whose live connection died catching up (Chromium and WebKit) |
| `pages` | `semmeltv-vm-pages` | pages answer, scroll and stay centred, Settings, details, a missing title, marking watched, watching a series together, a title tapped while the web app restarts |
| `phone-ui` | `semmeltv-vm-phone-ui` | the remote sheet, toasts, pages after a deploy |
| `feedback` | `semmeltv-vm-feedback` | reports with screenshots, statuses, threads, drafts, review questions |
| `downloads` | `semmeltv-vm-downloads` | season downloads, the search box's offers, TMDB and people search, pCloud storage |
| `sign-in` | `semmeltv-vm-sign-in` | invites, the profile picker with passwords, profiles on one phone and their limits |
| `tv-restart` | `semmeltv-vm-tv-restart` | **expected to fail** until step 4 of [two processes](../../docs/two-processes.md): a killed or frozen tv process comes back with the film while the API answers |
| `maintenance` | `semmeltv-vm-maintenance` | the bakery's closed screen from the front: planned work with `semmeltv-maintenance`, a stopped core, the guess growing once it runs over, the page coming back by itself |

One group, on nixbuild:

    nix build -L .#checks.x86_64-linux.semmeltv-vm-up-next \
      --builders 'ssh://eu.nixbuild.net x86_64-linux,aarch64-linux - 100 4 benchmark,big-parallel,kvm,nixos-test' --max-jobs 0

All groups at once: the same with `.#checks.x86_64-linux.semmeltv-vm`, which
links every group's result; add `--keep-going` to see every failing group.

`common.py` holds the helpers and the boot every group shares, then comes the
group's file in `groups/`, then `finish.py`, which fails the check if any
scenario failed. A new scenario goes into the group of its area; a group that
needs state another group leaves (a film in Continue Watching, a download job)
sets it up itself, as `pages` and `sign-in` do with `watched_a_while`, which
puts a title in Continue Watching through a phone stream's report.

A group listed in `expectedToFail` in `default.nix` tests a layout that isn't
built yet. Its check passes only while at least one scenario fails, and every
failure names the group's `EXPECTED_FAILURE`; a scenario failing for any other
reason, or every scenario passing, fails the check. Once the layout lands, take
the group out of `expectedToFail` and delete its `EXPECTED_FAILURE` line.
`tv-restart` fails today because `semmeltv.service` and a `semmeltv tv` under
cage don't exist yet.

The VM shortens the app's fixed waits through its environment, set in
`default.nix`: `SEMMELTV_UP_NEXT_SECONDS` for the Up next countdown,
`SEMMELTV_QUIET_SECONDS` for how long a quiet phone keeps the screen, and
`SEMMELTV_WATCHING_SECONDS` for how much of a title puts it in Continue
Watching. The scenarios read the same numbers as `UP_NEXT_SECONDS`,
`QUIET_SECONDS` and `WATCHING_SECONDS`. Production keeps 10, 45 and 60 s.

Screenshots of the TV and the phone land in each group's result.

Left to the cargo tests, because they need the Pi's hardware or the internet:
hardware decoding (drm, v4l2m2m), HDMI-CEC power, input and remote, HDMI display
modes, Bluetooth headphones, the Samsung UPnP volume, the pCloud mount and the
offline cache, downloads (Jackett, rqbit, Mullvad), and Tailscale, TLS and
deSEC.

`checks.x86_64-linux.semmeltv-no-upload` runs the TV's rqbit, built and
configured by `modules/rqbit.nix`, between a seeder and a third peer that asks
it for pieces mid-download, and fails if the TV uploads a single byte. The
seeder runs the same build without `RQBIT_DISABLE_UPLOAD` and must upload.
