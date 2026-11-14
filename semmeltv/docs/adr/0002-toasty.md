# 2. Toasty after all

Date: 2026-11-14. Status: accepted. Supersedes [1](0001-database.md).

## Context

[ADR 1](0001-database.md) chose Diesel because Toasty 0.11.0, the latest
release, has four problems:
- its SQLite driver sets no connection pragmas;
- the database has no foreign keys;
- its API breaks in every minor release;
- it has open panics on delete paths.

Frederik wants to use Toasty anyway. He is fine with following a young
library, and he is weighing Toasty for a production system elsewhere, so
SemmelTV is where to learn it.

## Decision

All database access goes through Toasty, pinned to a commit on main
(`45ac82a`, 2026-11-09), not to 0.11.0. The app keeps the `crate::db::Db`
interface it had under Diesel, so nothing outside `src/db/` changed.

ADR 1's objections against main:

- **Pragmas: solved on main.** `toasty_driver_sqlite::Sqlite` has typed
  setters, applied to every pooled connection. `foreign_keys` is on and
  `busy_timeout` is 5 s by default. We add WAL and `synchronous = NORMAL`.
- **Foreign keys: still none in the schema** (tokio-rs/toasty#366).
  Toasty's engine does the cascades: deleting a profile deletes its
  passkeys, sessions, invites, places, settings and prefs, and nulls
  `invites.created_by`. A test checks this. Two consequences:
  - Writes that go around Toasty (raw SQL, `sqlite3`) can leave orphans.
  - Inserting a row for a profile that does not exist is not refused by the
    database, so `save_audio_prefs` checks first.
- **Panics: none hit.** Profile deletion uses an instance delete
  (`profile.delete()`), which cascades fine. The open panic is in
  query-based deletes of a model with a `has_many` (#1200), so we don't
  delete profiles by query.
- **Churn: accepted.** Upgrading means bumping the `rev`, rebuilding and
  fixing what broke. The tests cover the whole store, including the
  Diesel conversion.

## Consequences

- **Models.** They are Rust structs in `app/src/db/models.rs`. Migrations
  are generated from them with `semmeltv toasty migration generate`
  (Toasty's CLI, built into the app), checked in under `app/toasty/`, and
  applied at startup by `embed_migrations!`. The seed of the four profiles
  is hand-appended SQL in the first migration.
- **Async in a sync app.** Toasty is async only, and the TV app is
  synchronous (Slint). `Db` owns a one-worker Tokio runtime and blocks on a
  channel for each call, so it works from plain threads and from inside the
  API's runtime alike.
- **Unique violations.** Toasty has no typed error for them. The store walks
  the error's source chain to rusqlite's
  `SQLITE_CONSTRAINT_UNIQUE`/`PRIMARYKEY` codes. That is why `rusqlite` is a
  direct dependency.
- **Diesel-era databases.** A database with `__diesel_schema_migrations` is
  renamed to `semmeltv.diesel.db` on the first start, and its rows are
  copied into a fresh Toasty schema in one transaction. This was tested on
  a copy of semmeltv5's database: every row carried over.
- **Teaching notes.** [`docs/toasty.md`](../toasty.md) explains how the app
  uses Toasty and what is good and bad about it.
