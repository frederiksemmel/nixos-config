---
name: semmeltv-incoming
description: Sort new videos from pcloud:Incoming into the SemmelTV library, update library.toml and fetch artwork and ratings.
disable-model-invocation: true
---

# Sort pCloud Incoming into SemmelTV

Run on beast, with the user present. The library lives in `pcloud:SemmelTV`: `library.toml`, `Movies/`, `Series/` and `Artwork/`. Every write to pCloud in this skill goes through one of three commands: `rclone moveto` for a video, `rclone copy` for new images in `Artwork/`, and `rclone copyto` for `library.toml`, and each of those needs the user's go-ahead from step 3 or 6. Leave everything else on pCloud as it is. Nothing here deletes.

`pcloud:Incoming/` may not exist yet. On the first run, create it with `rclone mkdir pcloud:Incoming`, tell the user to put files there, and stop.

Run the app as `nix run ~/repos/nixos-config#semmeltv --`, called `semmeltv` below, unless `semmeltv` is on `PATH`. `TMDB_TOKEN_FILE` and `MDBLIST_API_KEY_FILE` are set in the user's session; keep both values out of your output.

## 1. List

    rclone lsf -R --files-only --format sp pcloud:Incoming

Done when every entry is sorted into one of: film, episode, junk. Junk stays where it is and goes in the final report: `RARBG*.txt`, `*.exe`, `*.nfo`, samples, trailers. Subtitles beside a film or episode travel with it.

Read names only. Video content stays unopened.

## 2. Identify

For each film and show, find the TMDB record:

    curl -s -H "Authorization: Bearer $(cat "$TMDB_TOKEN_FILE")" \
      --get --data-urlencode "query=TITLE" https://api.themoviedb.org/3/search/movie

Use `search/tv` for a show. Done when each item has one confident match: same title, and same year give or take one. When there are several candidates, a different title, or no result, it is uncertain: ask the user with the candidates listed and wait for the answer. An item that stays uncertain stays in Incoming.

Keep the token out of your output.

## 3. Propose

Show a table of source and destination, and wait for the user's go-ahead. Canonical layout:

    SemmelTV/Movies/Title (Year)/Title (Year).ext
    SemmelTV/Series/Show/Season NN/Show - SNNENN - Episode Name.ext

Use the TMDB title with characters unsafe in a file name replaced. Check the destination first with `rclone lsf`; if it already exists, report it instead of moving onto it.

## 4. Move

    rclone moveto "pcloud:Incoming/SOURCE" "pcloud:DESTINATION"

Done when `rclone lsf -R --files-only pcloud:Incoming` shows only junk and uncertain items.

## 5. Import, artwork and ratings

Take a listing, the current manifest and the artwork, then import against the local copy. A listing is used instead of the `~/pcloud` mount because the mount can lag behind the moves.

    work=$(mktemp -d)
    rclone copyto pcloud:SemmelTV/library.toml "$work/library.toml"
    cp "$work/library.toml" "$work/before.toml"
    rclone copy pcloud:SemmelTV/Artwork "$work/Artwork"
    rclone lsf -R --files-only --format tsp --include "/Series/**" --include "/Movies/**" pcloud:SemmelTV > "$work/listing.txt"
    semmeltv import --listing "$work/listing.txt" --manifest "$work/library.toml"
    semmeltv artwork --manifest "$work/library.toml"

A missing `pcloud:SemmelTV/library.toml` means something is wrong: stop and tell the user, since a first import dates everything by file and is not this skill's job.

Done when every moved video appears as `added:` in the import output. A moved file that shows up as `video not imported` lost out to a larger copy of the same title; tell the user. The artwork step only fetches what `$work/Artwork` and the manifest lack, writes backdrops, posters and episode stills into `$work/Artwork` and episode names and runtimes into `$work/library.toml`, and lists titles without a TMDB id. It also stores IMDb, Rotten Tomatoes critics and audience, and Metacritic scores from MDBList for titles not rated yet, and prints the MDBList requests it used. MDBList allows 1000 a day: when a run reports titles left for the next run, run `semmeltv artwork` again the next day before uploading, or upload now and let a later run fill them in.

## 6. Review and upload

Show the user `diff "$work/before.toml" "$work/library.toml"`, the import's `review:` lines and the artwork step's summary. For a `review = true` entry the user can settle, edit the local file and drop the flag; a title given a `tmdb` id that way needs `semmeltv artwork` again. When the user agrees, upload the images first and the manifest last, so the TV never lists a title before its art:

    rclone copy "$work/Artwork" pcloud:SemmelTV/Artwork
    rclone copyto "$work/library.toml" pcloud:SemmelTV/library.toml

The TV picks it up by itself within a few minutes.

## 7. Report

List what moved, what is marked for review, the titles without art, and the junk and uncertain items left in Incoming.

## Refreshing artwork and ratings

TMDB's terms allow keeping images for six months, and ratings drift. When the user asks for a refresh, do step 5 without the listing and the import, with `semmeltv artwork --refresh --manifest "$work/library.toml"`, which downloads again every image older than five months and asks MDBList again for ratings 30 days old or older. An image it cannot replace is deleted locally and printed as a `problem:`; `rclone copy` leaves the pCloud copy in place, so name those to the user. Then upload as in step 6, after the user's go-ahead.
