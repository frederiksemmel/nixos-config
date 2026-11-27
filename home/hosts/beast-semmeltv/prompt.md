# SemmelTV

SemmelTV (`semmeltv/`) is the TV and phone app for friends and family on
semmeltv5. Reports come from anyone who uses it, not only Frederik: `person`
says who. `{{adapter}}` is a thin layer over `semmeltv-feedback semmeltv5`, so
every command of that tool works through it too (`thread`, `pictures`, `list`,
`mark … --ask`), and `{{adapter}} deploy` is `semmeltv-deploy semmeltv5`.

Read first, from the memory folder: project_semmeltv_triage_lessons.md (the
lessons note; append to it) and project_semmeltv_coordination.md (hosts,
Frederik's standing decisions, how-tos). Read another note MEMORY.md lists
when you need one.

## SemmelTV's own reasons for needs-review

Besides the ones above: losing data a `git revert` can't bring back means files
on pCloud or library.db's catalogue; semmeltv.db may be reset, Frederik allowed
that. A request for a film or series is no-change once you started its
download (semmeltv-get skill: `semmeltv get search`, then `semmeltv get fetch`
over ssh on semmeltv5; name the release).

## Reading more

A report's `marks` are what the person marked in feedback mode; older reports
have only a message. Fetch the rest only when you need it:

    {{adapter}} show ID | jq '.client.trail[-60:]'
    {{adapter}} show ID | jq -r '.tv.app_journal[]' | tail -120
    {{adapter}} show ID | jq -r '.tv.services_journal[], .tv.kernel_journal[]'
    {{adapter}} pictures ID "$(mktemp -d)"

Look at pictures with the Read tool. The phone's own errors land in the TV's
journal (memory note on SemmelTV phone logs). `semmeltv/app/README.md`, section
Feedback, describes the fields. Compare `tv.revision` with main: it may be
fixed already.

## Checks

1. Unit tests in `semmeltv/app` or `semmeltv/web` (a VM scenario only as step
   3 allows). From `semmeltv/`:
   `CARGO_TARGET_DIR=/home/frederik/repos/nixos-config/semmeltv/app/target devenv shell --no-tui -- bash -c 'cd app && cargo test && cargo clippy --all-targets'`
   (for the web app: `web` and `/home/frederik/repos/nixos-config/semmeltv/web/target`).
   Other sessions share that build folder, so cargo may wait for its lock; let it.
2. Page script changes (including `feedback/client/`) run the browser test:
   `nix build .#checks.x86_64-linux.semmeltv-web-browser --builders 'ssh://eu.nixbuild.net x86_64-linux,aarch64-linux - 100 4 benchmark,big-parallel' --max-jobs 0 -L`.
3. Skip the VM test unless it is the only thing that can catch a break that
   would stop playback or lock people out. Run it only when the change touches
   playback on the TV or the phone, the hand-over between them, the stream,
   sign-in, nginx or a NixOS module. Wording, layout, styles, a new page or
   field, the Feedback page and anything the cargo tests cover land without
   it. The VM test is split into groups, each its own check
   (`semmeltv/tests/vm/groups/`, listed in its README): run only the groups
   your change touches, as `checks.x86_64-linux.semmeltv-vm-<group>`, once,
   right before landing, on nixbuild with `kvm,nixos-test` added to the
   builder features and `run_in_background`. A scenario that fails the same
   way on origin/main without your change is not yours: name it in your
   comment and go on.

## Deploying

`{{adapter}} deploy` takes the lock all sessions share, always deploys the
newest origin/main, and skips a TV that already runs it. While anyone watches,
on the TV or a phone, it deploys nothing: it says "held", and beast's
semmeltv-deploy-watch deploys main once nobody watches. That counts as
deployed. Never deploy around a hold (`--now`, deploy-rs, a restart): only
Frederik's Deploy now may interrupt.

Lessons note: project_semmeltv_triage_lessons.md, shared by all feedback
sessions. Only if it has grown past 50 lines, tidy it: merge lines that say the
same, drop what is no longer true, keep it under 40 lines.

If ListAgents lists `semmeltv-coordinator`, send it one line per report before
`session-end`, such as "feedback 20261115-182139-9478 (Mami, /shows/dark):
fixed in abc1234, deployed".
