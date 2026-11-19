import json
import sys
import threading
from urllib.parse import parse_qs, urlsplit
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SHOW = int(sys.argv[1])
CHOICE = {
    "label": "Testserie.S02.1080p.WEB.x264-GRP",
    "facts": "1080p WEB x264 · 3 episodes",
    "why": ["the only release"],
    "score": 1.0,
    "size": 1,
    "seeders": 5,
    "parts": [],
}
PROPOSAL = {
    "id": "season-2",
    "asked": "Testserie season 2",
    "from": "web",
    "created": 0,
    "outcome": "searched",
    "label": "Testserie season 2",
    "target": {},
    "message": "Testserie season 2: 1080p WEB x264",
    "choices": [CHOICE],
    "dropped": 0,
    "problems": [],
}

JOB = {
    "id": "1",
    "label": "Testserie season 2",
    "stage": "downloading",
    "message": "Downloading",
    "parts": [],
    "log": [],
}

lock = threading.Lock()
TYPED = {**PROPOSAL, "id": "typed", "asked": "Testserie season 2"}
WHICH = {
    "id": "which",
    "asked": "Testserie",
    "from": "web",
    "created": 0,
    "outcome": "question",
    "text": "Which one: Testserie (2020, series) or Testserie (2001, film)?",
    "options": [
        {"kind": "show", "tmdb": SHOW, "label": "Season 2", "season": 2},
        {"kind": "show", "tmdb": SHOW, "label": "Season 4", "season": 4},
        {"kind": "movie", "tmdb": SHOW, "label": "Testserie (2001, film)"},
    ],
    "season": None,
    "episode": None,
}
MATCHES = [
    {"kind": "show", "tmdb": SHOW, "title": "Testserie", "year": 2020, "poster": None, "season": None, "owned": True},
    {"kind": "movie", "tmdb": SHOW + 1, "title": "Testserie", "year": 2001, "poster": None, "season": None, "owned": False},
]
DIRECTED = [
    {"kind": "movie", "tmdb": SHOW + 2, "title": "Regiefilm", "year": 2019, "poster": None, "season": None, "owned": False, "credit": "directed by Erika Regie"},
]
SEASONS = {
    **WHICH,
    "id": "seasons",
    "text": "Which season of Testserie?",
    "options": [
        {"kind": "show", "tmdb": SHOW, "label": "Season 2", "season": 2},
        {"kind": "show", "tmdb": SHOW, "label": "Season 4", "season": 4},
    ],
}
PICKED = {**PROPOSAL, "id": "season-4", "asked": "Testserie", "label": "Testserie season 4"}
HELD = [
    {"kind": "series", "key": "testserie", "name": "Testserie", "videos": 3, "bytes": 30_000_000_000},
    {"kind": "movie", "key": "kinofilm-2024", "name": "Kinofilm (2024)", "videos": 1, "bytes": 9_000_000_000},
] + [
    {"kind": "movie", "key": f"archiv-{number:02}", "name": f"Archiv {number:02}", "videos": 1, "bytes": 1_000_000_000 - number}
    for number in range(1, 22)
]
SPACE = {"total": 2_199_023_255_552, "used": 825_724_082_158, "free": 1_373_299_173_394}
deleted = []
state = {"searched": False, "confirmed": False, "typed": False, "which": False, "picked": False, "seasons": False}


class Downloads(BaseHTTPRequestHandler):
    def reply(self, body, status=200):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        url = urlsplit(self.path)
        with lock:
            if url.path == "/api/downloads/suggest":
                text = parse_qs(url.query).get("text", [""])[0]
                self.reply(MATCHES if text == "Testserie" else DIRECTED if text == "Regie" else [])
            elif self.path == f"/api/downloads/shows/{SHOW}/missing":
                self.reply([{"number": 2, "downloading": state["confirmed"]}, {"number": 3, "downloading": True}])
            elif self.path == "/api/downloads/pcloud":
                titles = [held for held in HELD if {"kind": held["kind"], "key": held["key"]} not in deleted]
                self.reply({"space": SPACE, "titles": titles})
            elif self.path == "/api/downloads/pcloud/deleted":
                self.reply(deleted)
            elif self.path == "/api/downloads":
                proposals = [PROPOSAL] if state["searched"] and not state["confirmed"] else []
                proposals += [TYPED] if state["typed"] else []
                proposals += [WHICH] if state["which"] else []
                proposals += [PICKED] if state["picked"] else []
                proposals += [SEASONS] if state["seasons"] else []
                jobs = [JOB] if state["confirmed"] else []
                self.reply({"proposals": proposals, "jobs": jobs, "room": 0, "quota": 0, "free": None})
            else:
                self.reply({"error": "no such path"}, 404)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        with lock:
            if self.path == "/api/downloads/search" and body == {"text": "Testserie season 2", "from": "web", "by": "Frederik"}:
                state["typed"] = True
                self.reply(TYPED)
            elif self.path == "/api/downloads/search" and body == {"text": "Testserie", "from": "web", "by": "Frederik"}:
                state["which"] = True
                self.reply(WHICH)
            elif self.path == "/api/downloads/search" and body == {"text": "Testserie", "from": "web", "by": "Frederik", "pick": {"kind": "show", "tmdb": SHOW, "season": None, "episode": None}}:
                state["seasons"] = True
                self.reply(SEASONS)
            elif self.path == "/api/downloads/search" and body.get("pick") == {"kind": "show", "tmdb": SHOW, "season": 4, "episode": None} and body.get("by") == "Frederik":
                state["picked"] = True
                self.reply(PICKED)
            elif self.path == "/api/downloads/search":
                if body.get("pick") != {"kind": "show", "tmdb": SHOW, "season": 2, "episode": None} or body.get("by") != "Frederik":
                    self.reply({"id": "", "asked": "", "from": "web", "outcome": "refused", "message": f"unexpected {body}"})
                    return
                state["searched"] = True
                self.reply(PROPOSAL)
            elif self.path == "/api/downloads/pcloud/delete":
                held = next((held for held in HELD if held["kind"] == body.get("kind") and held["key"] == body.get("key")), None)
                if held is None:
                    self.reply({"message": "That title is no longer in the library", "ok": False, "job": None})
                else:
                    deleted.append(body)
                    self.reply({"message": f"Deleted {held['name']} from the library; its 9.0 GB went to pCloud's trash", "ok": True, "job": None})
            elif self.path == "/api/downloads/confirm" and body == {"proposal": "season-2", "choice": 0, "by": "Frederik"}:
                state["confirmed"] = True
                self.reply({"message": "Downloading Testserie season 2", "ok": True, "job": "1"})
            else:
                self.reply({"message": f"unexpected {self.path} {body}", "ok": False, "job": None})

    def log_message(self, format, *args):
        sys.stderr.write(f"fake-downloads: {format % args}\n")


ThreadingHTTPServer(("127.0.0.1", 8092), Downloads).serve_forever()
