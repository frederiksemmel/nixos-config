# Ramura's site

Ramura Gassler's portfolio, https://ragassler.github.io, built from the repo
ragassler/ragassler.github.io (its README explains the layout: `site.toml`,
`projects/*.md`, `assets/`, `build.py`). It is Ramura's site, not Frederik's:
Frederik reviews it with `ramura-site` (a local copy with feedback mode,
`ramura-site/` in nixos-config) and Ramura decides what goes live. Each report
is an issue labelled `feedback` on that repo, with a screenshot of every marked
part. Ramura reads the issues and your answers there: write for both of them,
in English.

Lessons note: `project_ramura_site_lessons.md` in the memory folder (create it
on first use, and add it to MEMORY.md then).

## Where you work: this replaces the common Landing steps

Your worktree is a worktree of Ramura's repo (`~/repos/ragassler.github.io`),
not of nixos-config. Nothing goes to `main`, and nothing is deployed: GitHub
Pages publishes `main` when Ramura merges a pull request.

- One branch and one draft pull request per report: `feedback/issue-ID`.
  Start it from origin/main (`git fetch origin && git switch -c feedback/issue-ID origin/main`).
  For a reopened report whose pull request is still open, continue on its
  branch (`git switch -c feedback/issue-ID origin/feedback/issue-ID`) and add
  commits. If that pull request was merged or closed, start
  `feedback/issue-ID-2` from origin/main.
- Commit by path, never `git add -A`, never `--no-verify`. Follow the repo's
  commit style (`git log`): a short imperative subject, then the trailer lines.
  Never commit `.claude/` or `_site/`.
- Check: `python3 build.py` must succeed. Look at the result with
  `python3 -m http.server -d _site PORT` (pick a free port) and the Playwright
  tools, at the report's viewport and at an iPhone size, before and after.
- Push with `git push -u origin feedback/issue-ID`, then open the pull request
  as a draft, once per branch:
  `gh pr create --repo ragassler/ragassler.github.io --draft --base main --head feedback/issue-ID --title "…" --body "…"`.
  The body says, per comment, what changed and why, names the main alternative
  you didn't take, and ends with `Closes #ID`. Never merge, never mark a pull
  request ready, never push to `main`, never force-push.
- `{{adapter}} deploy` does nothing here; skip it.
- Every pull request has a preview: `{{adapter}} preview ID` builds the
  pushed branch and publishes it at https://ramura.semmeltv.ch/issue-ID/.
  Run it after each push and look at the preview with the Playwright tools.
- `{{adapter}} mark ID fixed COMMIT "WHAT"` publishes the preview again, puts
  its link at the top of the pull request's description and posts WHAT on the
  issue with the links: don't post a second comment for the same thing.

## The content is Ramura's

- Wording, order, layout, styling and fixes: decide and build, as the common
  rules say.
- Facts about Ramura's work (results, numbers, dates, people, institutions,
  what a project did): only from the repo itself (`projects/*.md`, `site.toml`,
  the PDFs in `assets/pdf/`, the linked public repos). Never invent one. When a
  comment needs a fact you can't find, mark needs-review and ask Ramura.
- New pictures or videos come from Ramura's own project folders through
  `tools/sync_media.py`, which you don't have: reuse what is in `assets/`, or
  mark needs-review and say which media is missing.
- `build.py` uses the standard library only, and the site has no build step
  besides it and no JavaScript framework. Keep it that way.
