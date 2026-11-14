"""A phone that watches on SemmelTV the way phone.js and an iPhone's AVPlayer do.

It asks the TV to play a title here, opens the stream, fetches the playlists,
init and segments at playback speed while keeping a buffer ahead, reports
progress every 10 s and follows /api/events. Whenever the screen says
something that would make phone.js pause or close the player, it says so and
why. Every request is logged with its time, status, latency and size.

    phone_sim.py BASE --cookie FILE --movie KEY [--minutes 20] [--buffer 60]
        [--flip-at SECONDS --flip-back SECONDS] [--quiet-at SECONDS --quiet-for SECONDS]

--cookie is a curl cookie jar or a file holding `semmeltv_session=...`.
--quiet-at/--quiet-for drop the progress reports for a while, as an iPhone
whose requests fail with "Load failed" does.
"""

import argparse
import asyncio
import json
import os
import re
import sys
import time
from urllib.parse import urljoin, urlparse

import aiohttp

REPORT_EVERY = 10.0
CLIENT = {"hevc": True, "eac3": True, "ac3": True}
began = time.monotonic()


def log(kind, text):
    stamp = time.strftime("%H:%M:%S")
    print(f"{stamp} +{time.monotonic() - began:7.1f}s {kind:8} {text}", flush=True)


def cookie_of(path):
    text = open(path).read()
    for line in text.splitlines():
        fields = line.split("\t")
        if len(fields) >= 7 and fields[5] == "semmeltv_session":
            return f"semmeltv_session={fields[6].strip()}"
    found = re.search(r"semmeltv_session=([^;\s]+)", text)
    if not found:
        sys.exit(f"{path} holds no semmeltv_session cookie")
    return f"semmeltv_session={found.group(1)}"


class Phone:
    def __init__(self, args):
        self.args = args
        self.base = args.base.rstrip("/")
        self.origin = f"{urlparse(self.base).scheme}://{urlparse(self.base).netloc}"
        self.headers = {"Cookie": cookie_of(args.cookie), "Origin": self.origin,
                        "User-Agent": "SemmelTV phone simulator"}
        self.device = {"id": os.urandom(16).hex(), "name": "Simulator"}
        self.screen = None
        self.session = None
        self.segments = []
        self.next_segment = 0
        self.buffered_to = 0.0
        self.playhead = 0.0
        self.playing = False
        self.started = False
        self.played = 0.0
        self.stalled_since = None
        self.stops = []
        self.opening = False
        self.quiet = (args.quiet_at, args.quiet_at + args.quiet_for) if args.quiet_at is not None else None

    async def request(self, method, path, body=None, quiet=False):
        url = path if path.startswith("http") else self.base + path
        started = time.monotonic()
        try:
            async with self.http.request(method, url, json=body, headers=self.headers,
                                         timeout=aiohttp.ClientTimeout(total=30)) as response:
                data = await response.read()
                ms = (time.monotonic() - started) * 1000
                if not quiet or response.status >= 400 or ms > 2000:
                    detail = data[:160].decode(errors="replace") if response.status >= 400 else f"{len(data)} B"
                    log("http", f"{method} {urlparse(url).path} {response.status} {ms:.0f} ms {detail}")
                return response.status, data
        except Exception as error:
            ms = (time.monotonic() - started) * 1000
            log("http", f"{method} {urlparse(url).path} FAILED after {ms:.0f} ms: {type(error).__name__} {error}")
            return None, b""

    async def json(self, method, path, body=None, quiet=False):
        status, data = await self.request(method, path, body, quiet)
        try:
            return status, json.loads(data) if data else None
        except ValueError:
            return status, None

    def ours(self, view):
        return bool(view and view.get("device") and view["device"]["id"] == self.device["id"])

    def adopt(self, view, source):
        before = self.screen
        self.screen = view
        if before is None or (before.get("kind"), before.get("play") and before["play"]["state"],
                              before.get("notice")) != (view.get("kind"), view.get("play") and view["play"]["state"], view.get("notice")):
            play = view.get("play")
            log("screen", f"{source}: {view.get('kind')} on {view['device']['name'] if view.get('device') else '-'}"
                          f"{', ' + play['state'] + ' ' + play.get('name', '') + ' at ' + format(play.get('position', 0), '.0f') if play else ', no play'}"
                          f"{' notice: ' + view['notice']['text'] if view.get('notice') else ''}")
        self.reconcile()

    def reconcile(self):
        view = self.screen
        play = view.get("play")
        wanted = self.ours(view) and play and play["state"] in ("moving", "starting", "playing")
        if not wanted:
            if self.session and self.playing:
                if self.ours(view) and view.get("kind") == "phone" and not play:
                    why = f"phone.js idles here (pauses, 'Stopped at {self.playhead:.0f}'): the TV dropped the play"
                else:
                    why = f"phone.js closes the player here: the screen went to {view.get('kind')}"
                notice = view.get("notice")
                self.stop(why + (f"; last notice: {notice['text']}" if notice else ""))
            return
        if self.session and not self.opening:
            return
        if not self.opening:
            asyncio.ensure_future(self.open(play.get("position")))

    def stop(self, why):
        log("STOP", why)
        self.stops.append((time.monotonic() - began, why))
        self.playing = False
        self.started = False
        self.session = None

    async def open(self, start):
        self.opening = True
        body = {"movie": self.args.movie, "client": CLIENT, "device": self.device}
        if start:
            body["start"] = start
        status, session = await self.json("POST", "/api/stream/open", body)
        self.opening = False
        if status != 200 or not session:
            await self.json("POST", "/api/screen", {"action": "failed", "reason": f"open answered {status}", "device": self.device})
            self.stop(f"stream/open answered {status}")
            return
        self.session = session
        log("stream", f"opened {session['id'][:8]} at {session['start']:.0f} of {session['duration']:.0f} s, "
                      f"{session['bitrate'] / 1e6:.1f} Mbit/s, heavy {session['heavy']}, audio converted {session['converted_audio']}")
        _, master = await self.request("GET", session["playlist"])
        variant = next(line for line in master.decode().splitlines() if line and not line.startswith("#"))
        media_url = urljoin(self.base + session["playlist"], variant)
        _, media = await self.request("GET", media_url)
        self.segments = []
        at = 0.0
        init = None
        duration = None
        for line in media.decode().splitlines():
            if line.startswith("#EXT-X-MAP:"):
                init = urljoin(media_url, re.search(r'URI="([^"]+)"', line).group(1))
            elif line.startswith("#EXTINF:"):
                duration = float(line[8:].split(",")[0])
            elif line and not line.startswith("#"):
                self.segments.append((at, duration, urljoin(media_url, line)))
                at += duration
        log("stream", f"{len(self.segments)} segments over {at:.0f} s")
        if init:
            await self.request("GET", init)
        self.playhead = session["start"]
        self.next_segment = max(0, next((i for i, (s, d, _) in enumerate(self.segments) if s + d > self.playhead), 0))
        self.buffered_to = self.segments[self.next_segment][0]
        self.playing = True

    async def fetcher(self):
        while True:
            await asyncio.sleep(0.2)
            if not self.playing or not self.session or self.next_segment >= len(self.segments):
                continue
            if self.buffered_to - self.playhead >= self.args.buffer:
                continue
            session = self.session
            start, duration, url = self.segments[self.next_segment]
            status, data = await self.request("GET", url, quiet=True)
            if session is not self.session:
                continue
            if status == 200:
                self.next_segment += 1
                self.buffered_to = start + duration
            else:
                await asyncio.sleep(1)

    async def clock(self):
        last = time.monotonic()
        while True:
            await asyncio.sleep(0.25)
            now = time.monotonic()
            step = now - last
            last = now
            if not self.playing or not self.session:
                continue
            if self.playhead + step <= self.buffered_to:
                if self.stalled_since is not None:
                    log("stall", f"playing again after {now - self.stalled_since:.1f} s at {self.playhead:.0f}")
                    self.stalled_since = None
                self.playhead += step
                self.played += step
                if not self.started:
                    self.started = True
                    asyncio.ensure_future(self.report("start"))
            elif self.stalled_since is None and self.started:
                self.stalled_since = now
                log("stall", f"buffer ran out at {self.playhead:.0f}")

    async def report(self, state):
        if not self.session:
            return
        now = time.monotonic() - began
        if self.quiet and self.quiet[0] <= now < self.quiet[1]:
            log("progress", f"{state} at {self.playhead:.0f} dropped on purpose (quiet until +{self.quiet[1]:.0f}s)")
            return
        session = self.session["id"]
        status, answer = await self.json("POST", f"/api/stream/{session}/progress",
                                         {"position": self.playhead, "played": self.played, "state": state}, quiet=True)
        if status != 200:
            log("progress", f"{state} at {self.playhead:.0f} answered {status}: {answer}")

    async def reporter(self):
        while True:
            await asyncio.sleep(REPORT_EVERY)
            if self.started and self.session:
                await self.report("playing")

    async def events(self):
        url = self.base.replace("http", "ws", 1) + "/api/events"
        while True:
            try:
                async with self.http.ws_connect(url, headers=self.headers, heartbeat=30) as socket:
                    log("events", "connected")
                    async for message in socket:
                        if message.type != aiohttp.WSMsgType.TEXT:
                            continue
                        event = json.loads(message.data)
                        if event["type"] == "screen":
                            self.adopt(event["screen"], "event")
                log("events", "closed")
            except Exception as error:
                log("events", f"failed: {type(error).__name__} {error}")
            await asyncio.sleep(2)

    async def flip(self, to):
        body = {"action": "flip", "to": to, "device": self.device}
        if to == "tv" and self.session:
            body["position"] = self.playhead
        status, answer = await self.json("POST", "/api/screen", body)
        log("flip", f"to {to}: {status} {answer}")
        _, view = await self.json("GET", "/api/screen")
        if view:
            self.adopt(view, "after flip")

    async def run(self):
        async with aiohttp.ClientSession() as self.http:
            _, playback = await self.json("GET", "/api/playback")
            _, view = await self.json("GET", "/api/screen")
            now = (playback or {}).get("now_playing")
            if now and not now.get("paused") and not self.args.force:
                sys.exit(f"The TV plays {now['title']}; not taking over (--force to anyway)")
            if view and view.get("play") and not view["play"].get("paused") and not self.args.force:
                sys.exit(f"{view['device']['name']} plays {view['play']['name']}; not taking over (--force to anyway)")
            tasks = [asyncio.ensure_future(task) for task in (self.events(), self.fetcher(), self.clock(), self.reporter())]
            await asyncio.sleep(1)
            status, answer = await self.json("POST", "/api/screen", {"action": "play", "movie": self.args.movie, "device": self.device})
            log("play", f"{status} {answer}")
            _, view = await self.json("GET", "/api/screen")
            self.adopt(view, "after play")
            end = time.monotonic() + self.args.minutes * 60
            flipped = False
            while time.monotonic() < end:
                await asyncio.sleep(1)
                elapsed = time.monotonic() - began
                if self.args.flip_at is not None and not flipped and elapsed >= self.args.flip_at:
                    flipped = True
                    await self.flip("tv")
                    await asyncio.sleep(self.args.flip_back)
                    await self.flip("phone")
                if int(elapsed) % 60 == 0:
                    log("state", f"playhead {self.playhead:.0f}, buffered to {self.buffered_to:.0f}, "
                                 f"session {self.session['id'][:8] if self.session else None}, stops {len(self.stops)}")
            for task in tasks:
                task.cancel()
            if self.session:
                await self.report("stopped")
                await self.request("DELETE", f"/api/stream/{self.session['id']}")
            await self.json("POST", "/api/screen", {"action": "stop", "device": self.device})
            await self.json("POST", "/api/screen", {"action": "flip", "to": "tv", "device": self.device})
            log("done", f"{len(self.stops)} stops: " + "; ".join(f"+{at:.0f}s {why}" for at, why in self.stops))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("base")
    parser.add_argument("--cookie", required=True)
    parser.add_argument("--movie", required=True)
    parser.add_argument("--minutes", type=float, default=20)
    parser.add_argument("--buffer", type=float, default=60, help="seconds AVPlayer keeps ahead")
    parser.add_argument("--flip-at", type=float)
    parser.add_argument("--flip-back", type=float, default=20)
    parser.add_argument("--quiet-at", type=float)
    parser.add_argument("--quiet-for", type=float, default=60)
    parser.add_argument("--force", action="store_true")
    asyncio.run(Phone(parser.parse_args()).run())


if __name__ == "__main__":
    main()
