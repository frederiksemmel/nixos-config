# 1. SQLite through Diesel, not Toasty

Date: 2026-11-14. Status: superseded the same day by [2. Toasty after all](0002-toasty.md).

## Context

The TV app kept its state in JSON files in its cache: `progress.json`,
`subtitles.json`, `subtitle-files.json` and `tv-token`. mpv kept a second copy
of every stopping point in `watch-later/`. Users and profiles are arriving
(Frederik as admin; Mami, Miriam and Ramu as viewers), with passwords,
passkeys and week-long sessions. All user-facing state becomes per profile. That
calls for one database with foreign keys and migrations.

The database is SQLite: one file under `/var/lib/semmeltv`, owned by the app's
user, on a single Raspberry Pi with one process writing.

Frederik's preference for the ORM was Toasty, Tokio's ORM, with Diesel as the
fallback. Toasty was checked against its primary sources on 2026-11-14:
crates.io, docs.rs, `github.com/tokio-rs/toasty` at `45ac82a`, the
`toasty-driver-sqlite` 0.11.0 source, and the tokio.rs blog.

## Decision

Diesel 2.4 with bundled SQLite (`libsqlite3-sys` `bundled`), used
synchronously, with SQL migrations embedded by `diesel_migrations` and applied
at startup.

Toasty can't carry this safely yet. Toasty 0.11.0 (2026-09-25) is current, and:

- **Its SQLite driver sets no connection options.** The released
  `toasty-driver-sqlite` issues no PRAGMA at all. Toasty runs a pool of
  several connections, so there is no `busy_timeout` (concurrent writers get
  `SQLITE_BUSY`), no `foreign_keys = ON`, and no `journal_mode`/`synchronous`.
  Typed pragmas exist only on main (tokio-rs/toasty#1285, unreleased).
- **It has no database-level foreign keys.** Cascades and set-null are done by
  Toasty's engine (tokio-rs/toasty#366 is open). The auth tables (sessions,
  passkeys, invites) need the database itself to enforce that they belong to a
  profile.
- **Its API is unstable by its author's own account.** The release post (2026-04-03)
  calls it "an early release … without much real-world usage" and expects
  "breaking changes as 0.x releases pretty often". Every minor from 0.7 to 0.11
  has `[breaking]` entries.
- **It has open panics on ordinary paths:** `update!` on a deleted row (#1298)
  and deleting a model with `has_many` (#1200), among about 25 open bugs. A
  panic takes the whole TV app down.
- **Its migrations are young.** It needs a hand-written CLI binary that
  diffs Rust models against TOML snapshots, there are no down migrations, and
  there are open bugs on composite keys (#807). Its `reset_db` deletes the
  whole SQLite file.

Toasty does have the modelling features this schema needs: composite keys,
upsert, JSON, enums and embedded migrations. So revisit this once a release
ships the pragma API and FK constraints, and the minors stop breaking.

Diesel 2.4.0 (2026-11-13) is mature and sync, which suits an app whose UI
thread is Slint and not async. Its SQLite connection is one we open and
configure ourselves. `embed_migrations!` puts plain SQL files
into the binary.

## Consequences

- One connection behind a `Mutex`, opened with `journal_mode = WAL`,
  `synchronous = NORMAL`, `foreign_keys = ON` and `busy_timeout = 5000`. Calls
  are sub-millisecond, so async handlers call the store directly.
- The schema is SQL in `app/migrations/`, and `app/src/db/schema.rs` mirrors
  it by hand: there is no `diesel_cli` step in the build. A new migration is a
  new numbered directory with `up.sql` and `down.sql`, plus the matching change
  to `schema.rs`.
- The domain types stay plain structs (`Progress`, subtitle `Settings`). The
  store loads and saves them per profile. A save diffs against the stored rows
  and writes only what changed, in one transaction.
- The database owns resume positions. mpv starts with `--start`, `--aid`,
  `--sid` and `--sub-visibility` from the profile's row and with
  `--no-resume-playback`. Watch-later files had to go: they are keyed by file
  only, so two profiles watching the same film would share one position. They
  also duplicated `progress.json`'s stopping point, which the phone handoff
  had to keep in step by forging mpv's MD5-named files.
- Facts about a file (its real duration, where its credits start, its subtitle
  delay) are shared by all profiles. Places, tracks, Currently watching,
  dismissals and settings are per profile.
- The download service's `jobs.json` is not in this database. It belongs to a
  separate service with its own user and state directory.
