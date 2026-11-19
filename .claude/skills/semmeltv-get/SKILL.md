---
name: semmeltv-get
description: Find and download a specific film or series for SemmelTV to exact quality wishes (4K, HDR, remux, codec, size, release group), or upgrade a title already in the library to a better copy.
---

# Get a title for SemmelTV

Run on beast, with Frederik present. The search and the download both run on **semmeltv5**: Jackett and rqbit live in its Mullvad namespace, and its downloads service uploads to pCloud and adds the title to `library.db`. rqbit there is built without upload, so a download only leeches.

Every TV command below runs as the downloads service's user with its environment. Call this `get`:

    ssh semmeltv5 'bin=$(systemctl cat semmeltv-downloads | grep -o "/nix/store/[^ ]*/bin/semmeltv" | head -1); sudo -u semmeltv env $(systemctl show semmeltv-downloads -p Environment --value) "$bin" get ARGS'

`get` with no arguments prints its usage. Quote a JSON argument for both shells.

## 1. Baseline

Find what the library has. `library.db` is the catalogue: on semmeltv5 it is `/var/lib/semmeltv/meta/library.db` (read it with `sudo sqlite3 -readonly`, sqlite3 from `/nix/store/*-sqlite-*-bin/bin`). Look the title up in `movies`, or in `shows` and `episodes`, by title or `tmdb`. A hit gives the TMDB id and the current `path`. Then probe that file's headers on the mount, `/var/lib/semmeltv/library/<path>`, with ffprobe from `/nix/store/*-ffmpeg*-bin/bin`, run with `sudo -u semmeltv` from `/tmp`: resolution, codec and profile, HDR transfer, bitrate, audio channels, size. A title missing from the library needs its TMDB id from the TMDB API (token in `$TMDB_TOKEN_FILE`; keep it out of your output).

Done when you can state the TMDB id and either the current copy's facts or "not in the library".

## 2. Search

    get search 'TITLE' --year YEAR --imdb ttID
    get search 'TITLE' --season N [--episode N]

Each output line is one release as JSON: `name`, `size`, `seeders`, `peers`, `indexer`, `magnet`, plus `resolution`, `source`, `video`, `hdr`, `audio`, `group` and `warnings`, all read from the name. `problem:` lines on stderr are indexers that failed. Search again with the original title and with a shorter title when the first search is thin. Indexers are the public ones in `semmeltv/modules/downloads.nix`.

Done when every search variant has run and its releases are pooled.

## 3. Rank

Filter by what Frederik asked for, then rank by these rules, in this order:

- **Plays everywhere.** The TV is a Pi 5 running mpv with `--hwdec=drm`: HEVC, including Main10 and 2160p, decodes in hardware. H.264 decodes in software, fine up to 1080p. AV1 has no hardware decoder on the Pi 5; reject it at 2160p. The phone stream copies the video as it is, so the same codec goes to the phone. For HDR, `HDR10` and `DV HDR10` (a Dolby Vision layer over an HDR10 base) are safe. A release tagged only `DV`, often a WEB-DL, may be Dolby Vision profile 5 with no HDR10 base, so its colours may be wrong. Say so, and pick it only if Frederik accepts that risk after a test.
- **Alive.** Seeder counts are what the indexer last saw; an old upload with a few seeders may be dead. A newer upload of similar quality is the safer pick.
- **Source and bitrate.** A remux beats an encode of the same disc, a BluRay encode beats a WEB-DL at the same resolution, and a bigger file at the same resolution and codec usually means a higher bitrate. Work out MB per minute from size and runtime, and compare it with the baseline from step 1.
- **Fits.** The service refuses a release bigger than its free room. `get jobs` prints `room` in bytes on its last line.

Show Frederik the top three as a table: name, resolution, source, codec, HDR, audio, size, seeders, MB per minute, with the baseline as the first row and a recommendation. When nothing beats the baseline, or nothing meets the ask (no 4K release exists, for example), say that plainly. Then wait for his pick.

Done when Frederik has named one release, or decided to keep what he has.

## 4. Download

    get fetch movie TMDB_ID [--replace] [--by NAME] 'JSON_LINE'
    get fetch show TMDB_ID --season N [--episode N] [--replace] [--by NAME] 'JSON_LINE'

`JSON_LINE` is the release's line from step 2, unchanged. Use `--replace` when the title, or that episode, is already in the library: the job then takes over the existing library entry and its id, so watch progress and resume points carry over to the new file. The old video and the subtitles named after it move to `pcloud:SemmelTV/Replaced/<old path>`. Without `--replace`, a title already in the library is refused as "already in the library". `--by` names the SemmelTV profile of whoever asked for it (Frederik, or a feedback report's author); the title page then says "Downloaded by NAME".

Follow the job with `get jobs` (each line has the stage, a message and the last log lines) until its stage is `done` or `failed`. A season comes one episode at a time, in order, and each episode is in the library (`placed`, and `in the library:` in the log) while the next one downloads, so Frederik can start watching before the job is done. A job that cannot read the torrent after 5 minutes found no peers; offer Frederik the next release from step 3.

Done when the job is `done`, and for a replacement its log shows `replaces` and `moved the replaced`.

## 5. Check and clean up

Probe the new file as in step 1 and compare it with the baseline. Ask Frederik to play it, on the TV or the phone, and to say whether picture and sound are right. For a replacement, once he confirms, delete the old copy from beast:

    rclone lsf -R pcloud:SemmelTV/Replaced
    rclone purge "pcloud:SemmelTV/Replaced/<old folder path>"

List the folder first and name every file you will delete. The subtitles that moved aside were made for the old copy; the TV fetches fresh ones when Frederik picks subtitles.

Done when Frederik has confirmed the new copy plays, `Replaced/` has no leftover from this title, and you have reported the before and after.
