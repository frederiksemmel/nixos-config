# Feedback framework

Comment on a web app from the app itself, and have Claude on beast change the
app, deploy it and answer. Two apps use it: SemmelTV (`semmeltv/`) and Courts
(`courts/`). Adding a third takes the four pieces below.

```
phone or laptop                    the app's server                  beast
feedback mode ──batch (JSON)──▶ report store ◀──adapter CLI── feedback-hub-watch
(client/)                       (the app's own)                  └─ one Claude session per
                                                                    project, for all open reports
```

## 1. Feedback mode in the page (`client/`)

`feedback-mode.js` and `feedback-mode.css` are plain files with no build. The
app serves them (a Topcoat app: `asset!("../../feedback/client/feedback-mode.js")`,
and its Nix source then has to include `feedback/client`) and turns them on
with attributes on `<html>`:

| attribute | meaning |
|---|---|
| `data-feedback-project="courts"` | the project; also keys the draft in localStorage |
| `data-feedback-send="/api/feedback"` | where the batch is POSTed |
| `data-feedback-list="/feedback"` | optional: the page that lists what was sent |
| `data-feedback-launcher="off"` | optional: no floating button; any `[data-feedback-open]` element toggles the mode |

An app that sends differently sets `window.feedbackMode.send = async (batch) => message`
(SemmelTV does: its reports carry the phone's trail and the TV's logs).

In feedback mode a layer over the page takes the gestures:

- **Tap or click:** marks the point and selects what is under it (the nearest
  link, button, paragraph, row or block). The `−` and `+` buttons select less
  or more: the next smaller or larger element.
- **Drag with one finger or the mouse:** selects the smallest element that
  holds everything the drag crossed (a paragraph, a button, a whole bar).
- **Two fingers, or Shift and drag:** a box; the comment keeps the box and what
  lies inside it.
- **Mark / Scroll:** the toolbar switches between marking and scrolling the page.

Each selection gets a comment and a numbered pin. Comments persist in
localStorage until sent, across pages, and the list sends them all as one
batch, with an optional note for the page as a whole and a photo.

## 2. The batch

```json
{
  "project": "courts",
  "at": "2026-11-27T08:10:00.000Z",
  "page": "/?date=2026-11-28",
  "title": "Courts",
  "note": "optional, for the batch as a whole",
  "photo": "data:image/jpeg;base64,… (optional)",
  "viewport": { "width": 393, "height": 659, "scale": 3, "coarse": true },
  "agent": "Mozilla/5.0 (iPhone; …)",
  "items": [
    {
      "n": 1,
      "kind": "point | drag | box",
      "comment": "Make the times bigger",
      "what": "Link “08:00”",
      "page": "/?date=2026-11-28",
      "point": { "x": 130, "y": 812 },
      "rect": { "x": 32, "y": 790, "w": 74, "h": 44 },
      "scroll": { "x": 0, "y": 640 },
      "target": { "selector": "section#club-6 > ul.courts > li > span.times > a.time:nth-of-type(1)",
                  "tag": "a", "text": "08:00", "label": null, "html": "<a class=\"time\" …>08:00</a>" },
      "inside": [ { "selector": "…", "tag": "p", "text": "…" } ]
    }
  ]
}
```

Coordinates are CSS pixels from the top left of the page (not the viewport).
`inside` is only on boxes. `target` on a box is the smallest element around it.

## 3. The report store (the app)

The app keeps each batch as a **report** with a status and a thread. States:
`new` → (`needs_review`) → `done`, with `outcome` `fixed` or `no_change`. A
person's comment on a `done` or `needs_review` report sets it to `reopened`;
Claude's comments never do. The app shows its reports to people (Courts:
`/feedback`; SemmelTV: the admin Feedback page) so they see what Claude did and
can answer.

Courts' HTTP API, which a new app can copy (`courts/src/feedback.rs`):

| | |
|---|---|
| `POST /api/feedback` | a batch from the page → `{ok, id, message}` |
| `GET /api/feedback?state=new` | report summaries |
| `GET /api/feedback/ID` | one report, with its batch and thread |
| `POST /api/feedback/ID/status` | `{state, outcome?, answer?, commit?}` |
| `POST /api/feedback/ID/comments` | `{text, claude?}`; `claude` only with the bearer key |

## 4. The adapter (a CLI on beast) and the hub

`hub/` is a home-manager module, `services.feedback-hub`. Each project names an
adapter, a command on beast with this interface:

```
ADAPTER open                     open reports (new, reopened), one JSON object per line:
                                 {"id", "state", "marked_at": epoch seconds, …}
ADAPTER brief ID                 the report as Markdown, for the session's prompt
ADAPTER show ID                  the whole report as JSON
ADAPTER comment ID TEXT          a comment signed Claude
ADAPTER mark ID fixed COMMIT WHAT
ADAPTER mark ID no-change WHY
ADAPTER mark ID needs-review WHY DECIDE [--ask Q --recommend A --option B …]
ADAPTER deploy                   deploys the newest main to where the app runs
```

`feedback-hub-watch` runs every two minutes. For each project with open
reports and no session of its own still at work, it waits until the newest
report is `quiet` seconds old (so a few batches sent close together go
together), then starts **one** Claude session, `feedback-PROJECT-MMDD-HHMM`,
with `hub/common.md`, the project's prompt and every open report. That session
fixes them all, lands on main, deploys once, and marks every report. A report
is handed over at most twice per status change, so a session that dies doesn't
loop.

Adapters: `courts/feedback.sh` (Courts on semmeltv5) and
`home/hosts/beast-semmeltv/adapter.sh` (SemmelTV, around `semmeltv-feedback`).

## Adding a project

1. Serve `client/` and set the `<html>` attributes.
2. Store reports and expose them (copy Courts' routes, or give the adapter
   another way in).
3. Write an adapter with the commands above.
4. Register it on beast:

   ```nix
   services.feedback-hub.projects.NAME = {
     adapter = pkgs.writeShellApplication { name = "feedback-NAME"; … };
     prompt = ./prompt.md;   # what the project is, how to test and land it
   };
   ```
