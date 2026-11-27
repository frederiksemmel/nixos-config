"""Serve a built static site with feedback mode, and keep each batch of
comments as a report with a screenshot of every marked part.

    serve.py --site DIR --out DIR --client DIR --shim FILE [--port N] [--rev SHA] [--repo URL]

A report is a folder in --out: batch.json, report.md with NN.png (the marked
part) and NN-view.png (the screen around it), and report.html, the same in one
file with the pictures inside, to send as it is. /feedback lists them and
sends one as an issue on --repo, its pictures on the repo's feedback branch.
"""

import argparse
import base64
import html
import json
import mimetypes
import re
import subprocess
import threading
import traceback
from datetime import datetime
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlsplit

PLAIN = "ramura-plain"
PROJECT = "ramura-site"
SCREENSHOTS = "feedback-screenshots"
shooting = threading.Lock()

APPLY = """(ctx) => {
  if (ctx.filter && ctx.filter !== "*")
    document.querySelector(`.chip[data-filter="${CSS.escape(ctx.filter)}"]`)?.click();
  if (Array.isArray(ctx.open))
    document.querySelectorAll("details[id]").forEach((d) => {
      d.open = ctx.open.includes(d.id);
      d.hidden = (ctx.hidden || []).includes(d.id);
    });
  document.querySelectorAll("img[loading=lazy]").forEach((i) => (i.loading = "eager"));
  return Promise.all([...document.images].map((i) => i.decode().catch(() => {})));
}"""

MARK = """(item) => {
  let rect = item.rect;
  const found = item.kind !== "box" && item.target?.selector && document.querySelector(item.target.selector);
  if (found) {
    const r = found.getBoundingClientRect();
    if (r.width || r.height) rect = { x: r.x + scrollX, y: r.y + scrollY, w: r.width, h: r.height };
  }
  const box = document.createElement("div");
  box.style.cssText = `position:absolute;left:${rect.x - 4}px;top:${rect.y - 4}px;width:${rect.w + 8}px;height:${rect.h + 8}px;
    border:3px solid #e5484d;border-radius:6px;pointer-events:none;z-index:2147483647;box-sizing:border-box`;
  const pin = document.createElement("div");
  pin.textContent = item.n;
  pin.style.cssText = `position:absolute;left:-14px;top:-14px;min-width:24px;height:24px;padding:0 6px;border-radius:12px;
    background:#e5484d;color:#fff;font:600 14px/24px system-ui,sans-serif;text-align:center;box-sizing:border-box`;
  box.append(pin);
  document.body.append(box);
  return { rect, size: [document.documentElement.scrollWidth, document.documentElement.scrollHeight] };
}"""


def around(rect, size, pad=40, min_w=480, min_h=240, max_h=4000):
    x0, y0 = rect["x"] - pad, rect["y"] - pad
    x1, y1 = rect["x"] + rect["w"] + pad, rect["y"] + rect["h"] + pad
    if x1 - x0 < min_w:
        grow = (min_w - (x1 - x0)) / 2
        x0, x1 = x0 - grow, x1 + grow
    if y1 - y0 < min_h:
        grow = (min_h - (y1 - y0)) / 2
        y0, y1 = y0 - grow, y1 + grow
    x0, y0 = max(0, x0), max(0, y0)
    x1, y1 = min(size[0], x1), min(size[1], y1, y0 + max_h)
    return {"x": x0, "y": y0, "width": max(1, x1 - x0), "height": max(1, y1 - y0)}


def shoot(port, batch, folder):
    from playwright.sync_api import sync_playwright

    viewport = batch.get("viewport") or {}
    with shooting, sync_playwright() as p:
        browser = p.chromium.launch()
        try:
            for item in batch.get("items") or []:
                context = item.get("context") or {}
                width = int(context.get("width") or viewport.get("width") or 1280)
                height = int(viewport.get("height") or 800)
                session = browser.new_context(
                    viewport={"width": width, "height": height},
                    device_scale_factor=min(float(viewport.get("scale") or 1), 2),
                )
                base = f"http://127.0.0.1:{port}"
                session.add_cookies([{"name": PLAIN, "value": "1", "url": base}])
                page = session.new_page()
                page.goto(base + (item.get("page") or "/"), wait_until="load")
                page.evaluate(APPLY, context)
                try:
                    page.wait_for_load_state("networkidle", timeout=5000)
                except Exception:
                    pass
                page.evaluate("document.fonts.ready.then(() => true)")
                marked = page.evaluate(MARK, item)
                size = marked["size"]
                n = int(item["n"])
                page.screenshot(path=folder / f"{n:02}.png", clip=around(marked["rect"], size, max_h=4000), full_page=True)
                scroll = item.get("scroll") or {"x": 0, "y": 0}
                view = {"x": scroll["x"], "y": scroll["y"], "w": width, "h": height}
                page.screenshot(path=folder / f"{n:02}-view.png", clip=around(view, size, pad=0), full_page=True)
                session.close()
        finally:
            browser.close()


def data_url(path):
    kind = mimetypes.guess_type(path.name)[0] or "image/png"
    return f"data:{kind};base64,{base64.b64encode(path.read_bytes()).decode()}"


def write_reports(folder, batch, rev, repo, problem):
    when = datetime.fromisoformat(batch["at"].replace("Z", "+00:00")).astimezone()
    stamp = when.strftime("%-d %B %Y, %H:%M")
    items = batch.get("items") or []
    viewport = batch.get("viewport") or {}
    site = f"{repo}/commit/{rev}" if rev and repo else None
    screen = f"{viewport.get('width')}×{viewport.get('height')} screen" if viewport else ""

    md = [f"# Comments on {batch.get('title') or 'the site'}", ""]
    md.append(" · ".join(filter(None, [stamp, f"site at [{rev}]({site})" if site else None, screen])))
    md.append("")
    if batch.get("note"):
        md += [batch["note"], ""]
    if (folder / "photo.jpg").exists():
        md += ["![Photo](photo.jpg)", ""]
    if problem:
        md += [f"_Screenshots failed: {problem}_", ""]
    for item in items:
        n = int(item["n"])
        target = item.get("target") or {}
        md += [f"## {n}. {item.get('comment', '').strip()}", ""]
        on = item.get("what") or target.get("tag") or "the page"
        md += [f"On {on}" + (f" (`{target['selector']}`)" if target.get("selector") else "") + f", page `{item.get('page') or '/'}`", ""]
        if (folder / f"{n:02}.png").exists():
            md += [f"![{n}]({n:02}.png)", "", f"Where it is on the screen: [{n:02}-view.png]({n:02}-view.png)", ""]
    (folder / "report.md").write_text("\n".join(md))

    def img(name, alt, cls):
        path = folder / name
        return f'<img class="{cls}" src="{data_url(path)}" alt="{html.escape(alt)}">' if path.exists() else ""

    parts = []
    for item in items:
        n = int(item["n"])
        target = item.get("target") or {}
        on = item.get("what") or target.get("tag") or "the page"
        parts.append(
            f"""<section><h2><span class="n">{n}</span>{html.escape(item.get('comment', '').strip())}</h2>
<p class="on">On {html.escape(on)}{f' · <code>{html.escape(target["selector"])}</code>' if target.get("selector") else ''}</p>
{img(f'{n:02}.png', f'The part comment {n} is about', 'detail')}
<details><summary>Where it is on the screen</summary>{img(f'{n:02}-view.png', f'The screen around comment {n}', 'view')}</details>
</section>"""
        )
    meta = " · ".join(
        filter(None, [stamp, f'site at <a href="{html.escape(site)}">{html.escape(rev)}</a>' if site else None, screen])
    )
    note = f"<p class=note>{html.escape(batch['note'])}</p>" if batch.get("note") else ""
    photo = img("photo.jpg", "Photo", "detail")
    failed = f"<p class=note>Screenshots failed: {html.escape(problem)}</p>" if problem else ""
    (folder / "report.html").write_text(
        f"""<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Comments on {html.escape(batch.get('title') or 'the site')}, {html.escape(stamp)}</title>
<style>
body{{font:16px/1.5 system-ui,sans-serif;color:#1c1c1e;background:#f6f5f2;margin:0}}
main{{max-width:880px;margin:0 auto;padding:32px 20px 64px}}
h1{{font-size:1.6rem;margin:0 0 4px}} .meta{{color:#666;margin:0 0 24px}}
section{{background:#fff;border-radius:12px;padding:20px;margin:0 0 20px;box-shadow:0 1px 3px #0001}}
h2{{font-size:1.15rem;margin:0 0 4px;display:flex;gap:10px;align-items:baseline}}
.n{{flex:none;background:#e5484d;color:#fff;border-radius:12px;min-width:24px;height:24px;font-size:14px;line-height:24px;text-align:center}}
.on{{color:#666;margin:0 0 12px;font-size:.9rem}} code{{font-size:.8rem;word-break:break-all}}
img{{display:block;max-width:100%;border:1px solid #ddd;border-radius:8px}} .view{{max-width:420px;margin-top:8px}}
summary{{cursor:pointer;color:#555;margin-top:12px;font-size:.9rem}} .note{{white-space:pre-wrap;background:#fff;border-radius:12px;padding:16px 20px}}
</style>
<main><h1>Comments on {html.escape(batch.get('title') or 'the site')}</h1><p class="meta">{meta}</p>{note}{photo}{failed}{''.join(parts)}</main></html>"""
    )


def gh(path, body=None, method=None):
    command = ["gh", "api", path]
    if method:
        command += ["--method", method]
    if body is not None:
        command += ["--input", "-"]
    done = subprocess.run(command, input=json.dumps(body) if body is not None else None, capture_output=True, text=True)
    if done.returncode:
        raise RuntimeError(f"gh api {path}: {(done.stderr or done.stdout).strip()}")
    return json.loads(done.stdout) if done.stdout.strip() else {}


def to_github(folder, slug, branch=SCREENSHOTS):
    """Push the report's pictures and batch to the branch and open an issue that shows them."""
    batch = json.loads((folder / "batch.json").read_text())
    files = sorted(p for p in folder.iterdir() if p.suffix in (".png", ".jpg") or p.name == "batch.json")
    entries = []
    for file in files:
        blob = gh(f"repos/{slug}/git/blobs", {"content": base64.b64encode(file.read_bytes()).decode(), "encoding": "base64"})
        entries.append({"path": f"reports/{folder.name}/{file.name}", "mode": "100644", "type": "blob", "sha": blob["sha"]})
    try:
        head = gh(f"repos/{slug}/git/ref/heads/{branch}")["object"]["sha"]
    except RuntimeError:
        head = None
    tree = {"tree": entries}
    if head:
        tree["base_tree"] = gh(f"repos/{slug}/git/commits/{head}")["tree"]["sha"]
    tree = gh(f"repos/{slug}/git/trees", tree)["sha"]
    commit = gh(
        f"repos/{slug}/git/commits",
        {"message": f"Screenshots for the comments of {folder.name}", "tree": tree, "parents": [head] if head else []},
    )["sha"]
    if head:
        gh(f"repos/{slug}/git/refs/heads/{branch}", {"sha": commit}, method="PATCH")
    else:
        gh(f"repos/{slug}/git/refs", {"ref": f"refs/heads/{branch}", "sha": commit})

    def url(name):
        return f"https://raw.githubusercontent.com/{slug}/{commit}/reports/{folder.name}/{name}"

    items = batch.get("items") or []
    viewport = batch.get("viewport") or {}
    rev = (batch.get("site") or {}).get("rev")
    lines = [
        "Comments on the site"
        + (f" as of {rev}" if rev else "")
        + (f", seen on a {viewport.get('width')}×{viewport.get('height')} screen" if viewport else "")
        + ".",
        "",
    ]
    if batch.get("note"):
        lines += [batch["note"], ""]
    if (folder / "photo.jpg").exists():
        lines += [f"![Photo]({url('photo.jpg')})", ""]
    for item in items:
        n = int(item["n"])
        target = item.get("target") or {}
        on = item.get("what") or target.get("tag") or "the page"
        lines += [f"### {n}. {item.get('comment', '').strip()}", ""]
        lines += [f"On {on}" + (f" · `{target['selector']}`" if target.get("selector") else ""), ""]
        if (folder / f"{n:02}.png").exists():
            lines += [f"![Comment {n}]({url(f'{n:02}.png')})", ""]
        if (folder / f"{n:02}-view.png").exists():
            lines += ["<details><summary>Where it is on the screen</summary>", "", f"![Screen around comment {n}]({url(f'{n:02}-view.png')})", "", "</details>", ""]
    lines += [
        "Claude takes these comments up, pushes the changes to a branch of its own and opens a draft pull request that closes this issue. Nothing reaches `main` until it is merged.",
        "",
        f"<!-- ramura-site report={folder.name} -->",
    ]
    first = ((items[0]["comment"] if items else batch.get("note", "")).strip().splitlines() or ["Comments"])[0]
    title = (first[:70] + "…" if len(first) > 70 else first) + (f" (+{len(items) - 1} more)" if len(items) > 1 else "")
    try:
        gh(f"repos/{slug}/labels/feedback")
    except RuntimeError:
        gh(f"repos/{slug}/labels", {"name": "feedback", "color": "e5484d", "description": "Comments sent with ramura-site"})
    issue = gh(f"repos/{slug}/issues", {"title": title, "body": "\n".join(lines), "labels": ["feedback"]})
    (folder / "issue.txt").write_text(issue["html_url"] + "\n")
    return issue["html_url"]


def list_page(out, problem=None):
    rows = []
    for folder in sorted((d for d in out.iterdir() if (d / "batch.json").exists()), reverse=True):
        batch = json.loads((folder / "batch.json").read_text())
        items = batch.get("items") or []
        first = items[0]["comment"] if items else batch.get("note", "")
        issue = (folder / "issue.txt").read_text().strip() if (folder / "issue.txt").exists() else None
        if issue:
            sent = f'<a class="sent" href="{html.escape(issue)}">Sent to Ramura: issue #{html.escape(issue.rsplit("/", 1)[-1])}</a>'
        else:
            sent = f'<form method="post" action="/api/reports/{folder.name}/github"><button>Send to Ramura as a GitHub issue</button></form>'
        rows.append(
            f"""<li><a href="/reports/{folder.name}/report.html">{html.escape(folder.name)}</a>
<span>{len(items)} comment{'' if len(items) == 1 else 's'}: {html.escape(first[:120])}</span>
{sent}<code>{html.escape(str(folder))}</code></li>"""
        )
    body = "".join(rows) or "<li>No comments sent yet. Open the site and press the feedback button.</li>"
    failed = f'<p class="failed">Not sent: {html.escape(problem)}</p>' if problem else ""
    return f"""<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Comments for Ramura</title>
<style>body{{font:16px/1.5 system-ui,sans-serif;max-width:880px;margin:0 auto;padding:32px 20px}}
li{{margin:0 0 20px}} li span{{display:block;color:#444}} code{{display:block;font-size:.8rem;color:#777}}
form{{margin:6px 0}} button{{font:inherit;padding:8px 14px;border-radius:8px;border:1px solid #bbb;background:#fff;cursor:pointer}}
.sent{{display:inline-block;margin:6px 0;color:#1a7f37}} .failed{{background:#fde8e8;padding:12px 16px;border-radius:8px}}</style>
<h1>Comments for Ramura</h1>{failed}
<p>Each report goes to Ramura as an issue on <a href="https://github.com/ragassler/ragassler.github.io/issues?q=label%3Afeedback">the site's repo</a>,
with the screenshots in it (kept on the repo's <code>{SCREENSHOTS}</code> branch). Claude on beast takes it up and opens a draft
pull request. <code>report.html</code> in a report's folder is the same as one file. <a href="/">Back to the site</a></p>
<ul>{body}</ul></html>"""


def handler(args):
    client = Path(args.client)
    files = {
        "/__feedback/feedback-mode.js": client / "feedback-mode.js",
        "/__feedback/feedback-mode.css": client / "feedback-mode.css",
        "/__feedback/shim.js": Path(args.shim),
    }
    out = Path(args.out)
    head = (
        '<link rel="stylesheet" href="/__feedback/feedback-mode.css">'
        '<script src="/__feedback/shim.js"></script>'
        '<script type="module" src="/__feedback/feedback-mode.js"></script>'
    )
    attrs = f'<html data-feedback-project="{PROJECT}" data-feedback-send="/api/feedback"'

    class Handler(SimpleHTTPRequestHandler):
        def __init__(self, *a, **kw):
            super().__init__(*a, directory=args.site, **kw)

        def end_headers(self):
            self.send_header("Cache-Control", "no-store")
            super().end_headers()

        def log_message(self, *a):
            pass

        def reply(self, status, body, kind):
            data = body.encode() if isinstance(body, str) else body
            self.send_response(status)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            path = unquote(urlsplit(self.path).path)
            if path in files:
                kind = "text/css" if path.endswith(".css") else "text/javascript"
                return self.reply(200, files[path].read_bytes(), kind)
            if path == "/feedback":
                return self.reply(200, list_page(out), "text/html; charset=utf-8")
            if path.startswith("/reports/"):
                file = (out / path.removeprefix("/reports/")).resolve()
                if file.is_relative_to(out.resolve()) and file.is_file():
                    kind = mimetypes.guess_type(file.name)[0] or "application/octet-stream"
                    return self.reply(200, file.read_bytes(), kind)
                return self.reply(404, "Not found", "text/plain")
            file = Path(self.translate_path(self.path))
            if file.is_dir():
                file = file / "index.html"
            if file.suffix == ".html" and file.is_file() and f"{PLAIN}=1" not in (self.headers.get("Cookie") or ""):
                page = file.read_text()
                page = re.sub(r"<html\b", attrs, page, count=1)
                page = page.replace("</head>", head + "</head>", 1)
                return self.reply(200, page, "text/html; charset=utf-8")
            return super().do_GET()

        def do_POST(self):
            path = urlsplit(self.path).path
            sending = re.fullmatch(r"/api/reports/([\w-]+)/github", path)
            if sending:
                folder = out / sending.group(1)
                if not (folder / "batch.json").exists():
                    return self.reply(404, "Not found", "text/plain")
                problem = None
                if not (folder / "issue.txt").exists():
                    try:
                        print(f"sent to Ramura: {to_github(folder, args.slug)}", flush=True)
                    except Exception as error:
                        traceback.print_exc()
                        problem = str(error)
                if problem:
                    return self.reply(502, list_page(out, problem), "text/html; charset=utf-8")
                self.send_response(303)
                self.send_header("Location", "/feedback")
                self.send_header("Content-Length", "0")
                return self.end_headers()
            if path != "/api/feedback":
                return self.reply(404, "Not found", "text/plain")
            batch = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)))
            folder = out / datetime.now().strftime("%Y-%m-%d-%H%M%S")
            folder.mkdir(parents=True, exist_ok=True)
            photo = batch.pop("photo", None)
            if photo and photo.startswith("data:image/jpeg;base64,"):
                (folder / "photo.jpg").write_bytes(base64.b64decode(photo.split(",", 1)[1]))
            batch["site"] = {"rev": args.rev, "repo": args.repo}
            (folder / "batch.json").write_text(json.dumps(batch, indent=2, ensure_ascii=False))
            problem = None
            try:
                shoot(self.server.server_address[1], batch, folder)
            except Exception as error:
                traceback.print_exc()
                problem = str(error).splitlines()[0] if str(error) else type(error).__name__
            write_reports(folder, batch, args.rev, args.repo, problem)
            n = len(batch.get("items") or [])
            print(f"saved {n} comment(s): {folder / 'report.html'}", flush=True)
            said = f"{n} comment{'' if n == 1 else 's'}" + ("" if not problem else " (without screenshots: they failed)")
            if args.local:
                message = f"Saved {said}. They are listed at /feedback."
            else:
                try:
                    issue = to_github(folder, args.slug)
                    print(f"sent to Ramura: {issue}", flush=True)
                    message = f"Sent {said} to Ramura as issue #{issue.rsplit('/', 1)[-1]}. Claude takes them up in a few minutes and opens a draft PR."
                except Exception as error:
                    traceback.print_exc()
                    message = f"Saved {said}, but GitHub failed ({str(error)[:120]}). Send them again from /feedback."
            self.reply(200, json.dumps({"ok": True, "id": folder.name, "message": message}), "application/json")

    return Handler


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--site", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--client", required=True)
    parser.add_argument("--shim", required=True)
    parser.add_argument("--port", type=int, default=8077)
    parser.add_argument("--rev", default="")
    parser.add_argument("--repo", default="")
    parser.add_argument("--local", action="store_true", help="keep reports here; send them from /feedback")
    args = parser.parse_args()
    args.slug = urlsplit(args.repo).path.strip("/")
    Path(args.out).mkdir(parents=True, exist_ok=True)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), handler(args))
    print(f"Ramura's site with feedback mode: http://localhost:{args.port}/", flush=True)
    print(f"Comments sent so far:             http://localhost:{args.port}/feedback", flush=True)
    print(f"Reports are saved in {args.out}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
