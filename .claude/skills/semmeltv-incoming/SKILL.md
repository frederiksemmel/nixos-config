---
name: semmeltv-incoming
description: Sort new videos from pcloud:Incoming into the SemmelTV library and update library.toml.
disable-model-invocation: true
---

# Sort pCloud Incoming into SemmelTV

Run on beast, with the user present. Every write to pCloud in this skill goes through one of two commands: `rclone moveto` for a video, and `rclone copyto` for `library.toml`, and each of those needs the user's go-ahead from step 3 or 6. Leave everything else on pCloud as it is. Nothing here deletes.

`pcloud:Incoming/` may not exist yet. On the first run, create it with `rclone mkdir pcloud:Incoming`, tell the user to put files there, and stop.

Run the app as `nix run ~/repos/nixos-config#semmeltv --`, called `semmeltv` below, unless `semmeltv` is on `PATH`. `TMDB_TOKEN_FILE` is set in the user's session.

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

    Movies/Title (Year)/Title (Year).ext
    Series/Show/Season NN/Show - SNNENN - Episode Name.ext

Use the TMDB title with characters unsafe in a file name replaced. Check the destination first with `rclone lsf`; if it already exists, report it instead of moving onto it.

## 4. Move

    rclone moveto "pcloud:Incoming/SOURCE" "pcloud:DESTINATION"

Done when `rclone lsf -R --files-only pcloud:Incoming` shows only junk and uncertain items.

## 5. Import

Take a listing and the current manifest, then import against the local copy. A listing is used instead of the `~/pcloud` mount because the mount can lag behind the moves.

    work=$(mktemp -d)
    rclone copyto pcloud:library.toml "$work/library.toml"
    cp "$work/library.toml" "$work/before.toml"
    rclone lsf -R --files-only --format tsp --include "/Series/**" --include "/Movies/**" pcloud: > "$work/listing.txt"
    semmeltv import --listing "$work/listing.txt" --manifest "$work/library.toml"

A missing `pcloud:library.toml` means something is wrong: stop and tell the user, since a first import dates everything by file and is not this skill's job.

Done when every moved video appears as `added:` in the output. A moved file that shows up as `video not imported` lost out to a larger copy of the same title; tell the user.

## 6. Review and upload

Show the user `diff "$work/before.toml" "$work/library.toml"` and the import's `review:` lines. For a `review = true` entry the user can settle, edit the local file and drop the flag. When the user agrees:

    rclone copyto "$work/library.toml" pcloud:library.toml

The TV picks it up by itself within a few minutes, and its artwork within the hour.

## 7. Report

List what moved, what is marked for review, and the junk and uncertain items left in Incoming.
