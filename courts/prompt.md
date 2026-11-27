# Courts

Courts (`courts/` in this repo) finds free tennis, padel and squash courts in
Zurich on GotCourts and notifies about the ones that free up. It runs on
semmeltv5 at https://courts.semmeltv.ch, beside SemmelTV, and only Frederik
uses it. Rust: Topcoat for the pages (`src/pages.rs`, `src/layout.rs`,
`src/style.css`), Toasty on SQLite for the store (`src/db.rs`, migrations in
`toasty/`), a poller that asks GotCourts every minute (`src/poll.rs`,
`src/gotcourts.rs`), Web Push (`src/push.rs`), and the feedback store
(`src/feedback.rs`). The NixOS module is `module.nix`; the clubs it asks about
are listed in `semmeltv/hosts/semmeltv5/default.nix`.

Lessons note: `project_courts_lessons.md` in the memory folder (create it on
first use, and add it to MEMORY.md then).

## Checks

From `courts/`: `cargo test` and `cargo clippy --all-targets`, both clean.
After a model change in `src/db.rs`, generate a migration:
`COURTS_DB=$(mktemp -d)/scratch.db cargo run -- toasty migration generate --name what-changed`,
then `touch src/db.rs` before building again (the migrations are compiled in).

To look at a change, run it locally against GotCourts with a few clubs:

    key=$(mktemp); head -c 24 /dev/urandom | base64 | tr -d '/+=' > "$key"
    HOST=127.0.0.1 PORT=3917 COURTS_DB=$(mktemp -d)/courts.db COURTS_KEY_FILE=$key \
      COURTS_DAYS=3 COURTS_CLUBS=6,19,20,21 cargo run

The sign-in cookie is `courts_key` (Secure, so over plain http set it in the
browser yourself). Look at it with the Playwright iPhone tools and at a laptop
width of 1280 px. Build the package for the Pi on nixbuild before landing:
`nix build .#nixosConfigurations.semmeltv5.config.services.courts.package --builders 'ssh://eu.nixbuild.net x86_64-linux,aarch64-linux - 100 4 benchmark,big-parallel' --max-jobs 0`.

## Rules of the house

- Topcoat: `$(…)` is compiled to JavaScript (no `&&`, `format!` or calls to
  plain functions in it); a page or route function's name must not equal a
  local variable's; a page that returns `Err(redirect(..))` renders the
  layout's error page instead, so redirect from a `#[route]`.
- GotCourts is unofficial: keep the request rate where it is, and don't add
  booking (people book on GotCourts through the links).
- `{{adapter}} deploy` deploys semmeltv5 as a whole, held while anyone watches
  SemmelTV.
