# Toasty in SemmelTV

How the TV app uses [Toasty](https://github.com/tokio-rs/toasty), Tokio's ORM,
what it was like to move a real store onto it, and what that says about using
it in production. Written 2026-11-14 against Toasty `main` at `45ac82a`
(2026-11-09). The newest release is 0.11.0 (2026-09-25), and a few things here
exist only on main.

The store went from Diesel to Toasty in one day:
- **The code:** `semmeltv/app/src/db/`, five files.
- **The migrations:** the `app/toasty/` directory.
- **The rest of the app:** unchanged, because everything goes through one
  `crate::db::Db` type, and its API stayed the same.

The Diesel version is in git history (`fb16ad4`, `70d6f69`) for comparison.

## Contents

1. [What Toasty is](#what-toasty-is)
2. [Setup](#setup)
3. [Models](#models)
4. [Reading and writing](#reading-and-writing)
5. [Transactions](#transactions)
6. [Relations and cascades](#relations-and-cascades)
7. [Migrations](#migrations)
8. [Raw SQL](#raw-sql)
9. [Async inside a sync app](#async-inside-a-sync-app)
10. [Errors](#errors)
11. [What is nice](#what-is-nice)
12. [What is not](#what-is-not)
13. [For a production system](#for-a-production-system)

## What Toasty is

You describe your data as Rust structs with `#[derive(toasty::Model)]`. From
those, Toasty derives three things:
- **A database schema**, which its CLI turns into SQL migrations.
- **A typed query API**: `get_by_*`, `filter_by_*`, `upsert_by_*` and so on,
  named after your keys and indexes.
- **A query engine** that turns those calls into SQL. The engine also does
  what the database would otherwise do, such as cascading deletes along
  relations.

The drivers are SQLite, PostgreSQL, MySQL, Turso and DynamoDB. The models
are written to be the same across all of them, which is why some things live
in Toasty's engine rather than in SQL.

Diesel is the opposite design. It is a type-checked SQL builder: you write the
schema in SQL, mirror it in `table!` macros, and write queries that look like
SQL. Toasty is closer to ActiveRecord or Prisma: models first, SQL generated.

## Setup

`app/Cargo.toml`:

```toml
toasty = { git = "https://github.com/tokio-rs/toasty", rev = "45ac82a…", default-features = false, features = ["sqlite", "serde", "migration"] }
toasty-cli = { git = "…", rev = "45ac82a…" }           # the migration CLI, see below
toasty-core = { git = "…", rev = "45ac82a…" }          # TransactionMode is not re-exported
toasty-driver-sqlite = { git = "…", rev = "45ac82a…" } # typed pragmas
rusqlite = "0.40"                                    # only to recognise unique violations
```

- **Why main and not a release:** the pragma setters landed after 0.11.0.
- **Pinned by `rev`:** the API changes between minors, so pinning keeps the
  build reproducible. Crane vendors the git dependency from `Cargo.lock`,
  and the Nix build needed no other changes.
- **SQLite is bundled:** it's compiled from C through rusqlite's `bundled`
  feature, so there is no system `libsqlite3` to match.

Opening the database (`src/db/mod.rs`):

```rust
let driver = Sqlite::open(file)
    .journal_mode(JournalMode::Wal)
    .synchronous(Synchronous::Normal)
    .busy_timeout(Duration::from_secs(5));          // foreign_keys is ON by default
let db = toasty::Db::builder().models(models::all()).build(driver).await?;
MIGRATIONS.apply(&db).await?;
```

Every pooled connection gets these pragmas. Released 0.11.0 sets none at all,
which was the main reason [ADR 1](adr/0001-database.md) said no.

## Models

All models are in `src/db/models.rs`. Here are the features they use, with
real examples.

**A model with a generated key, a unique field, an embedded enum and
relations:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, toasty::Embed)]
pub enum Role { Admin, Viewer }        // stored as TEXT with CHECK (role IN ('admin', 'viewer'))

#[derive(Debug, toasty::Model)]        // table "profiles": the struct name, pluralised
pub struct Profile {
    #[key] #[auto]
    pub id: i64,                       // INTEGER PRIMARY KEY AUTOINCREMENT
    #[unique]
    pub name: String,                  // unique index, plus get_by_name / upsert_by_name / …
    pub role: Role,
    pub password_hash: Option<String>, // Option means a nullable column
    pub color: Option<String>,
    pub created_at: i64,

    #[has_many]                        // no column: the child holds the key
    pub passkeys: Deferred<Vec<Passkey>>,
    #[has_many(pair = profile)]        // two relations to Invite, so name the pairing
    pub invites: Deferred<Vec<Invite>>,
    #[has_many(pair = creator)]
    pub invites_sent: Deferred<Vec<Invite>>,
    #[has_one]
    pub audio_prefs: Deferred<Option<AudioPrefs>>,
    // … sessions, file_places, show_places, watching, dismissals, subtitle_settings
}
```

`Deferred<T>` means the relation is loaded only when asked for:
`profile.passkeys().exec(&mut db)`, or `.include(...)` on a query. A plain
`Vec<Passkey>` would load it with every profile.

**The child side:**

```rust
#[derive(Debug, toasty::Model)]
pub struct Passkey {
    #[key] #[auto] pub id: i64,
    #[index]
    pub profile_id: i64,                               // the foreign key column
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,                    // required, so deleting the parent deletes this
    #[unique]
    pub credential_id: Vec<u8>,                        // BLOB, unique
    // …
}
```

An optional parent, `created_by: Option<i64>` with
`creator: Deferred<Option<Profile>>`, is set to NULL when the parent goes
instead of deleted.

**Composite keys:**

```rust
#[derive(Debug, toasty::Model)]
#[key(profile_id, path)]               // PRIMARY KEY (profile_id, path)
pub struct FilePlace {
    pub profile_id: i64,
    pub path: String,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    pub stopped_at: Option<f64>,
    pub finished: bool,                // BOOLEAN, stored as 0/1
    // …
}
```

This generates `FilePlace::upsert_by_profile_id_and_path(profile, path)`. An
embedded enum can be part of the key too: `Watching` is keyed by
`(profile_id, kind, title_key)`.

**JSON, defaults and table names:**

```rust
#[derive(Debug, toasty::Model)]
#[table = "subtitle_settings"]         // override the pluralised name
pub struct SubtitleSettings {
    #[key] pub profile_id: i64,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    #[column(type = text)]
    pub settings: Json<subtitles::Settings>,  // any serde type; setters take the inner type
}

pub struct File {
    #[key] pub path: String,
    #[default(0.0)]
    pub subtitle_delay: f64,           // applied by Toasty on create, not a SQL DEFAULT
    // …
}
```

**Registering the models:**

```rust
pub fn all() -> toasty::ModelSet {
    toasty::models!(Profile, Passkey, Session, Invite, File, FilePlace, ShowPlace,
                    Watching, Dismissal, SubtitleSettings, AudioPrefs, Setting)
}
```

`toasty::models!(crate::*)` would find every model in the crate. Listing them
explicitly keeps the set obvious.

## Reading and writing

Every call takes an executor (`&mut Db`, a `Connection` or a `Transaction`)
and is `async`. The calls used in the store:

```rust
// by key or index: generated per field
StoredSetting::filter_by_name(name).first().exec(&mut db).await?        // Option<Setting>
Passkey::filter_by_profile_id(id).order_by(Passkey::fields().id().asc()).exec(&mut db).await?

// expressions: typed field paths, and/or, null checks, IN
let file = File::fields();
File::filter(file.duration().is_some().or(file.credits().is_some())).exec(&mut db).await?;
FilePlace::filter(fields.profile_id().eq(p).and(fields.path().in_list(gone))).delete().exec(&mut db).await?;

// ordering by several fields
StoredWatching::filter(…).order_by((fields.last_played().desc(), fields.title_key().asc()))

// create: a macro with struct-literal syntax; missing Option fields are NULL
toasty::create!(Passkey { profile_id: 1, credential_id: id, rp_id, name, passkey, created_at: now })
    .exec(&mut db).await?;

// upsert: one atomic INSERT … ON CONFLICT
FilePlace::upsert_by_profile_id_and_path(profile, path)
    .stopped_at(row.stopped_at)
    .finished(row.finished)
    .exec(&mut db).await?;

// update one loaded record
toasty::update!(found { password_hash: hash }).exec(&mut tx).await?;
```

Upserts carry most of the writes. The progress store loads what is stored,
compares it with what the app has in memory, and upserts or deletes only
the rows that changed (`src/db/progress.rs`).

## Transactions

```rust
let mut tx = db.transaction_builder().mode(TransactionMode::Immediate).begin().await?;
let unused = StoredInvite::filter(f.token_hash().eq(token).and(f.used_at().is_none()))
    .first().exec(&mut tx).await?;
let Some(mut unused) = unused else { return Ok(false) };   // dropping tx rolls back
toasty::update!(unused { used_at: Some(now) }).exec(&mut tx).await?;
tx.commit().await?;
```

- **Exclusive borrow:** a transaction holds `&mut db` for its lifetime, so a
  statement can't slip past it on another pooled connection.
- **Rollback by dropping:** dropping `tx` rolls it back.
- **`Immediate` mode:** this is SQLite's `BEGIN IMMEDIATE`, which takes the
  write lock up front, so two people tapping the same invite are served one
  after the other.
- **Why a transaction here:** a query-level `update!` returns `()` rather
  than the rows it changed, so "update if still unused" can't be one
  statement in the typed API.

## Relations and cascades

Deleting a profile is one call:

```rust
let Some(found) = StoredProfile::filter_by_id(id).first().exec(&mut tx).await? else { … };
found.delete().exec(&mut tx).await?;
```

Toasty's engine reads the relations and also deletes or nulls the related rows:
- **Deleted:** the profile's passkeys, sessions and invites, its places, show
  places, Currently watching, dismissals, subtitle settings and audio prefs.
- **Nulled:** `created_by` on invites that profile sent.

The test `deleting_a_profile_takes_its_state_along_and_leaves_the_others`
checks all of it. This is application-level: the SQL schema has no
`REFERENCES` clauses at all (see [What is not](#what-is-not)).

## Migrations

The workflow:

1. **Change a model** in `src/db/models.rs`.
2. **Generate** from `semmeltv/app`:
   `cargo run -- --db /tmp/scratch.db toasty migration generate --name what-changed`.
   That writes three files:
   - `toasty/migrations/0001_what-changed.sql`;
   - `toasty/snapshots/0001_snapshot.toml`: the whole schema as TOML, which
     the next generate diffs against;
   - an entry in `toasty/history.toml`.

   For a dropped and added table or column, the CLI asks interactively
   whether it was a rename.
3. **Review the SQL and commit all three.** On SQLite, a column type or
   nullability change becomes the usual six-step table rebuild.
4. **Applied at startup:** `embed_migrations!("toasty")` compiles the history
   and the SQL into the binary, and `MIGRATIONS.apply(&db)` runs the pending
   ones. It records each one in `__toasty_migrations`, each inside its own
   transaction.

Toasty's CLI has to link your models, so it is not a standalone tool. You
build it into a binary of your own with `toasty_cli::ToastyCli`. SemmelTV
puts it behind a mode of the app (`semmeltv toasty …`, in `db::toasty_cli`)
rather than a second binary, because the app crate is a single `main.rs`
with modules.

The household seed (Frederik, Mami, Miriam, Ramu) is four `INSERT` rows
appended by hand to `0000_initial.sql`. Toasty has no seed concept. Since
`checksums = false` in `Toasty.toml`, editing a generated file is allowed.

The generated SQL for one table:

```sql
CREATE TABLE "watching" (
    "profile_id" BIGINT NOT NULL,
    "kind" TEXT NOT NULL CHECK ("kind" IN ('series', 'movie')),
    "title_key" TEXT NOT NULL,
    "last_played" BIGINT NOT NULL,
    PRIMARY KEY ("profile_id", "kind", "title_key")
);
```

## Raw SQL

Moving a Diesel-era database across (`src/db/diesel.rs`) is the one place
the store writes SQL by hand. It attaches the old file on a dedicated
connection and copies every table in one transaction:

```rust
let mut connection = db.connection().await?;
statement("ATTACH DATABASE ?1 AS earlier").bind(path).exec(&mut connection).await?;
let mut tx = connection.transaction().await?;
for sql in COPY { statement(*sql).exec(&mut tx).await?; }  // INSERT INTO … SELECT … FROM earlier.…
tx.commit().await?;
```

- **Return values:** `toasty::sql::statement` returns the number of affected
  rows, and `toasty::sql::query` returns dynamic `Value::Record` rows.
- **Placeholders:** they are the backend's own (`?1` on SQLite). Toasty
  doesn't rewrite them.
- **Why a dedicated connection:** `ATTACH` is per connection, and the pool
  could otherwise hand the next statement to a different one.

Detecting the old database, checkpointing it and renaming it uses rusqlite
directly. That needs a connection closed at a known moment, and Toasty's
pool closes connections in background tasks.

## Async inside a sync app

Toasty is async only and needs Tokio. The TV app runs on Slint's event loop,
and the HTTP API calls the store from inside its own Tokio runtime. So `Db`
owns a one-worker runtime and turns every call into a blocking one:

```rust
fn wait<T>(runtime: &Runtime, work: impl Future<Output = toasty::Result<T>> + Send + 'static) -> Result<T> {
    let (done, result) = mpsc::sync_channel(1);
    runtime.spawn(async move { let _ = done.send(work.await); });
    Ok(result.recv()??)
}

pub fn setting(&self, name: &str) -> Result<Option<String>> {
    let name = name.to_string();                       // the future must be 'static
    self.wait(|mut db| async move {
        Ok(StoredSetting::filter_by_name(name).first().exec(&mut db).await?.map(|s| s.value))
    })
}
```

- **Why spawn and a channel:** `Runtime::block_on` panics when called from
  inside another runtime, and spawning onto the store's runtime doesn't.
- **Shutting down:** `Drop` calls `shutdown_background()`, because dropping
  a runtime from async context also panics.
- **Cloning the handle:** `toasty::Db` is a cheap clone of the pool, so each
  call clones it into its future.

In an async-native app (axum, a worker), none of this is needed: you call
`.exec(&mut db).await` directly.

## Errors

`toasty::Error` has kinds such as `record_not_found` and
`condition_failed`, but no "unique constraint violated". The login code needs
exactly that: a passkey that belongs to someone else. So the store walks the
error's `source()` chain down to rusqlite's error and checks the extended
code:

```rust
if let Some(rusqlite::Error::SqliteFailure(failure, _)) = error.downcast_ref::<rusqlite::Error>() {
    return matches!(failure.extended_code, SQLITE_CONSTRAINT_UNIQUE | SQLITE_CONSTRAINT_PRIMARYKEY);
}
```

## What is nice

- **The models are the schema.** Under Diesel the schema lived twice: once in
  `up.sql` and again by hand in `schema.rs`, plus row tuples or structs per
  query. Now `models.rs` is the single place, and migrations are generated
  from it. This was the biggest real gain.
- **The generated API reads like the schema.** Examples:
  `FilePlace::upsert_by_profile_id_and_path`,
  `Passkey::filter_by_credential_id`, `Invite::filter_by_profile_id`. Declare
  an `#[index]` and the lookup method appears; remove it and every caller
  stops compiling.
- **Upserts are first class.** You get atomic `INSERT … ON CONFLICT` per key
  or unique constraint, with `on_create` / `on_update` branches and
  `or_ignore`. Diesel has `on_conflict().do_update()`, but you name the
  columns twice.
- **Relations do the bookkeeping.** Deleting a profile cascades through ten
  tables from one `delete()`. Optional relations are nulled instead.
- **Enums and JSON are painless.**
  - A plain Rust enum becomes a `TEXT` column with a `CHECK`.
  - `Json<T>` stores any serde type and hands back `T`.

  Under Diesel these were strings, mapped by hand.
- **Transactions are safe by construction.** Borrowing `&mut db` means a
  statement can't accidentally run outside the transaction. `Immediate`
  mode is exposed for SQLite.
- **The connection setup is right on main.** Typed pragmas apply to every
  pooled connection, with foreign keys on and a busy timeout by default.
- **One model, several backends.** The same models run on PostgreSQL, Turso
  and DynamoDB, which matters for a production system that may outgrow
  SQLite.
- **Good documentation.** The guide (`docs/guide/` in the repo) is thorough
  and current, and `examples/` holds runnable programs per topic. It was
  enough to write this port without reading much of the source.

## What is not

- **Not stable.** It's pre-1.0, and every minor release from 0.7 to 0.11
  broke something. Features we rely on are only on main, so the app tracks
  a git commit. Expect to fix code at each bump.
- **No foreign keys in the database** (tokio-rs/toasty#366).
  - Cascades exist only inside Toasty. Raw SQL, the `sqlite3` shell, or
    another service writing the same database can leave orphans.
  - Inserting a row for a profile that doesn't exist succeeds, so
    `save_audio_prefs` checks first.
  - You could hand-add `REFERENCES` to a generated migration, but the next
    SQLite table rebuild Toasty generates would drop them silently.
- **Open panics.** A panic in a single-process app kills the app.
  - Deleting a model with a `has_many` through a query, rather than a
    loaded instance, hits a `todo!()` (#1200).
  - `update!` on a row deleted meanwhile panics too (#1298).

  The store avoids both, but you have to know about them.
- **Query-level `update!` and `delete` return nothing.** There is no
  affected-row count, so "set if still unset" needs a transaction (or raw
  SQL, whose `statement` does return a count).
- **No typed constraint errors.** Unique violations need the downcast above,
  which ties the store to the SQLite driver's error type.
- **Async only.** In a sync program you build the bridge shown above.
  - Every call clones its inputs (`to_vec()`, `to_string()`) into a
    `'static` future.
  - The store is noisier than the Diesel version for it: about the same
    number of lines overall, even though the schema is declared once.
- **Deferred relation fields make models awkward to compare.** The progress
  store's diff needed separate plain structs, where Diesel's row structs
  could derive `PartialEq` directly.
- **Rough migration tooling.**
  - The CLI must be compiled into your app.
  - Generating can prompt interactively.
  - There are no down migrations.
  - `#[default]` is applied by Toasty, not as a SQL `DEFAULT`.
  - Seeds are hand-edited SQL.
  - A 560-line snapshot TOML lives in the repo.
  - `reset_db` and `migration reset` delete the whole SQLite file.
- **Smaller gaps.**
  - There is no case-insensitive unique (no `COLLATE NOCASE`), so "mami"
    vs "Mami" is still checked in Rust.
  - Timestamps are `jiff` types stored as ISO text, so the store keeps
    `i64` Unix seconds as before.
  - Some error messages repeat their cause several times over.

## Is it SQLite's fault?

No. Every rough edge above is in Toasty itself, so it is the same on
PostgreSQL. Checked against `main` at `45ac82a` and the open issues on
2026-11-14:

| Rough edge | SQLite | PostgreSQL |
| --- | --- | --- |
| No `REFERENCES` in generated schema (#366) | yes | yes: `toasty-sql` emits none for any backend |
| Engine panics (#1200, #1298, #1313, #855, #717) | yes | yes: they are in the engine and macros, not a driver |
| No typed unique-violation error | yes | yes: the PostgreSQL driver maps only `40001` and `25006` and wraps `23505` like any other error |
| Query `update!`/`delete` return no count | yes | yes |
| Async only | yes | yes |
| Pragmas | fixed on main | not applicable |

- **Driver bugs:** the open driver-specific bugs are two on PostgreSQL (an
  enum type that must be preloaded, #1305; `citext`, #1187) and two on
  DynamoDB. None of the open bugs is specific to SQLite.
- **Test coverage:** the integration suite runs in CI against SQLite, MySQL
  and PostgreSQL 18 alike.

What PostgreSQL would add with Toasty:
- `jiff` timestamps as `timestamptz` instead of ISO text;
- `jsonb`;
- native arrays for `Vec<scalar>`;
- a server that several programs can share safely.

None of those touches the problems above. For SemmelTV it would also cost
a database server on the Pi, with its own memory, upgrades between major
versions and `pg_dump` backups. It would also cost a temporary server in the
Nix build for every test run, where now each test opens an in-memory SQLite
database in milliseconds. One app writing on one device is the case SQLite
is built for, so SemmelTV stays on SQLite.

## For a production system

Toasty can carry a real service today if you treat it like a young dependency:

- **Pin and test.** Pin a release or a commit, and keep a test suite over
  your store layer that runs against the real driver: the in-memory SQLite
  is fast. Here, all of the store's behaviour is tested,
  including the Diesel conversion, and each Toasty bump is a rebuild that
  tells you what changed.
- **Wrap it.** Keep Toasty behind your own store interface, as `crate::db::Db`
  does here. Swapping Diesel for Toasty touched no file outside `src/db/`.
  The same wall would let you go back.
- **Don't count on the database for integrity.** If other programs, scripts
  or people with `psql` write the same database, Toasty's missing foreign
  keys matter. Either keep every writer on Toasty, or wait for #366.
- **Avoid the known panics.** Load-then-delete instead of query-delete for
  parents with children, and catch panics at your request boundary if one
  slipping through would be worse than an error.
- **Prefer an async service.** Most of the friction here came from the sync
  UI; an axum service would use Toasty as intended.
- **PostgreSQL is a fine backend for it.** Its Toasty driver gets the same
  CI and has typed timestamps, `jsonb` and arrays. Just don't expect it to
  fix the points above, because they are Toasty's, not SQLite's.

What Toasty brings over Diesel is a higher level: models as the schema,
generated migrations, generated lookups named after your indexes, relations
that cascade, enums and JSON without mapping code, and one model across
several databases. What it costs today is stability and some guarantees you'd
normally get from the database itself.
