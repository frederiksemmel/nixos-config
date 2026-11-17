import json
import sys
import threading
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
state = {"searched": False, "confirmed": False}


class Downloads(BaseHTTPRequestHandler):
    def reply(self, body, status=200):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        with lock:
            if self.path == f"/api/downloads/shows/{SHOW}/missing":
                self.reply([{"number": 2, "downloading": state["confirmed"]}, {"number": 3, "downloading": True}])
            elif self.path == "/api/downloads":
                proposals = [PROPOSAL] if state["searched"] and not state["confirmed"] else []
                jobs = [JOB] if state["confirmed"] else []
                self.reply({"proposals": proposals, "jobs": jobs, "room": 0, "quota": 0, "free": None})
            else:
                self.reply({"error": "no such path"}, 404)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        with lock:
            if self.path == "/api/downloads/search":
                if body.get("pick") != {"kind": "show", "tmdb": SHOW, "season": 2, "episode": None}:
                    self.reply({"id": "", "asked": "", "from": "web", "outcome": "refused", "message": f"unexpected {body}"})
                    return
                state["searched"] = True
                self.reply(PROPOSAL)
            elif self.path == "/api/downloads/confirm" and body == {"proposal": "season-2", "choice": 0}:
                state["confirmed"] = True
                self.reply({"message": "Downloading Testserie season 2", "ok": True, "job": "1"})
            else:
                self.reply({"message": f"unexpected {self.path} {body}", "ok": False, "job": None})

    def log_message(self, format, *args):
        sys.stderr.write(f"fake-downloads: {format % args}\n")


ThreadingHTTPServer(("127.0.0.1", 8092), Downloads).serve_forever()
