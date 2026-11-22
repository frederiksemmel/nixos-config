"""N phones watching at once, each played on hls.js's timeline.

Each viewer opens a stream, starts playing after its first segment, keeps
30 s ahead like phone.js's hls.js (maxBufferLength 30) and counts a stall
whenever the playhead reaches the end of what it has. It reads the EVENT
playlist again every 2 s while the next segment isn't listed yet.

App:  python3 viewers.py --app https://semmeltv5.semmeltv.ch --cookie jar --title KEY -n 3
Farm: python3 viewers.py --farm http://100.x.y.z:18555 --file PATH -n 8
"""

import argparse
import http.client
import http.cookiejar
import json
import random
import threading
import time
import urllib.parse

BUFFER = 30.0
PLAYLIST_POLL = 2.0


class Connection:
    def __init__(self, base, cookie):
        self.url = urllib.parse.urlsplit(base)
        self.cookie = cookie
        self.connection = None

    def request(self, method, path, body=None):
        headers = {"User-Agent": "semmeltv-loadtest"}
        if self.cookie:
            headers["Cookie"] = self.cookie
        if body is not None:
            body = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        for attempt in range(2):
            if self.connection is None:
                kind = http.client.HTTPSConnection if self.url.scheme == "https" else http.client.HTTPConnection
                self.connection = kind(self.url.netloc, timeout=60)
            try:
                self.connection.request(method, path, body=body, headers=headers)
                response = self.connection.getresponse()
                return response.status, response.read()
            except (http.client.HTTPException, OSError):
                self.connection.close()
                self.connection = None
                if attempt:
                    raise
        raise RuntimeError("unreachable")


def parse(playlist):
    listed, duration, ended = [], None, False
    for line in playlist.splitlines():
        if line.startswith("#EXTINF:"):
            duration = float(line[8:].split(",")[0])
        elif line.startswith("#EXT-X-ENDLIST"):
            ended = True
        elif line and not line.startswith("#"):
            listed.append((line, duration))
    return listed, ended


class Viewer:
    def __init__(self, number, args, title, start):
        self.number = number
        self.args = args
        self.title = title
        self.start = start
        self.result = {"viewer": number, "title": title, "start": start}

    def open(self, connection):
        if self.args.app:
            body = {
                "movie": self.title,
                "start": self.start,
                "client": {"hevc": True, "eac3": self.args.eac3, "ac3": self.args.eac3},
                "device": {"id": f"loadtest-{self.number}", "name": f"Loadtest {self.number}"},
            }
            status, answer = connection.request("POST", "/api/stream/open", body)
            if status != 200:
                raise RuntimeError(f"open: {status} {answer[:200]!r}")
            opened = json.loads(answer)
            base = f"/api/stream/{opened['id']}/"
            return opened, base, f"/api/stream/{opened['id']}"
        body = {"file": self.title, "start": self.start, "client": {"eac3": self.args.eac3, "ac3": self.args.eac3}}
        status, answer = connection.request("POST", "/open", body)
        opened = json.loads(answer)
        base = f"/s/{opened['id']}/"
        return opened, base, base

    def run(self, until):
        connection = Connection(self.args.app or self.args.farm, self.args.cookie)
        began = time.monotonic()
        try:
            opened, base, closing = self.open(connection)
        except Exception as error:
            self.result["error"] = str(error)
            return
        self.result["open_s"] = round(time.monotonic() - began, 2)
        self.result["sound"] = opened.get("sound") or ("converted" if opened.get("converted_audio") else None)
        if self.args.app:
            connection.request("GET", base + "main.m3u8")
        status, text = connection.request("GET", base + "video.m3u8")
        listed, ended = parse(text.decode())
        connection.request("GET", base + "init.mp4")
        position = buffered = 0.0
        stalls = stall_time = 0.0
        stalled = False
        playing_since = None
        last = time.monotonic()
        polled = last
        waits = fetched = 0
        bytes_total = 0
        slowest = 0.0

        def advance():
            nonlocal position, stall_time, stalls, stalled, last
            now = time.monotonic()
            delta, last = now - last, now
            if playing_since is None:
                return
            room = buffered - position
            if delta <= room:
                position += delta
                stalled = False
            else:
                position = buffered
                stall_time += delta - room
                if not stalled:
                    stalls += 1
                stalled = True

        while time.monotonic() < until:
            advance()
            if buffered - position >= BUFFER:
                time.sleep(min(1.0, buffered - position - BUFFER + 0.05))
                continue
            if fetched >= len(listed):
                if ended:
                    break
                if time.monotonic() - polled >= PLAYLIST_POLL:
                    status, text = connection.request("GET", base + "video.m3u8")
                    polled = time.monotonic()
                    listed, ended = parse(text.decode())
                    if fetched >= len(listed):
                        waits += 1
                else:
                    time.sleep(0.2)
                continue
            name, duration = listed[fetched]
            asked = time.monotonic()
            status, body = connection.request("GET", base + name)
            took = time.monotonic() - asked
            if status != 200:
                self.result["error"] = f"{name}: {status}"
                break
            slowest = max(slowest, took / max(duration, 0.1))
            bytes_total += len(body)
            fetched += 1
            advance()
            buffered += duration
            if playing_since is None:
                playing_since = time.monotonic()
                self.result["startup_s"] = round(playing_since - began, 2)
        advance()
        connection.request("DELETE", closing)
        watched = time.monotonic() - (playing_since or time.monotonic())
        self.result.update(
            {
                "watched_s": round(watched, 1),
                "played_s": round(position, 1),
                "stalls": int(stalls),
                "stall_s": round(stall_time, 1),
                "playlist_waits": waits,
                "segments": fetched,
                "mbit": round(bytes_total * 8 / max(watched, 1) / 1e6, 2),
                "slowest_segment_ratio": round(slowest, 2),
            }
        )


def main():
    parser = argparse.ArgumentParser()
    where = parser.add_mutually_exclusive_group(required=True)
    where.add_argument("--app")
    where.add_argument("--farm")
    parser.add_argument("--cookie-jar")
    parser.add_argument("--title", action="append", default=[], help="a movie key (app) or file (farm); viewers take them in turn")
    parser.add_argument("--file", action="append", default=[], dest="title")
    parser.add_argument("-n", "--viewers", type=int, default=1)
    parser.add_argument("--seconds", type=float, default=150)
    parser.add_argument("--files-from", help="a file with one title per line")
    parser.add_argument("--start", type=float, default=600, help="where the first viewer starts; each next one 300 s later")
    parser.add_argument("--anywhere", type=float, help="start each viewer at a random place before this many seconds instead")
    parser.add_argument("--ramp", type=float, default=2, help="seconds between two viewers opening")
    parser.add_argument("--eac3", action="store_true", help="the clients play E-AC-3 and AC-3, as Safari does")
    parser.add_argument("--label", default="")
    args = parser.parse_args()
    args.cookie = None
    if args.cookie_jar:
        jar = http.cookiejar.MozillaCookieJar(args.cookie_jar)
        jar.load(ignore_discard=True, ignore_expires=True)
        args.cookie = "; ".join(f"{c.name}={c.value}" for c in jar)
    if args.files_from:
        with open(args.files_from) as file:
            args.title += [line.strip() for line in file if line.strip()]
    pick = random.Random()
    starts = [pick.uniform(300, args.anywhere) if args.anywhere else args.start + 300 * i for i in range(args.viewers)]
    began = time.monotonic()
    viewers = [Viewer(i, args, args.title[i % len(args.title)], round(starts[i])) for i in range(args.viewers)]
    threads = []
    for i, viewer in enumerate(viewers):
        thread = threading.Thread(target=viewer.run, args=(began + i * args.ramp + args.seconds,))
        thread.start()
        threads.append(thread)
        time.sleep(args.ramp)
    for thread in threads:
        thread.join()
    for viewer in viewers:
        print(json.dumps({"label": args.label, **viewer.result}), flush=True)
    good = [v.result for v in viewers if "error" not in v.result]
    print(
        json.dumps(
            {
                "label": args.label,
                "summary": True,
                "viewers": args.viewers,
                "failed": args.viewers - len(good),
                "stalls": sum(v["stalls"] for v in good),
                "stall_s": round(sum(v["stall_s"] for v in good), 1),
                "worst_startup_s": max((v.get("startup_s", 0) for v in good), default=None),
                "worst_open_s": max((v.get("open_s", 0) for v in good), default=None),
                "mbit_total": round(sum(v["mbit"] for v in good), 1),
            }
        ),
        flush=True,
    )


if __name__ == "__main__":
    main()
