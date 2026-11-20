"""Plays a real library file the way a phone or laptop browser plays it from
the TV, in a Firefox that decodes HEVC, and checks that playback moves after
seeks.

The server here does what semmeltv/app/src/stream does for one browser: it
runs ffmpeg with the TV's arguments (remux.py), measures the stream's offset
the way remux.rs does, and serves the master playlist, the media playlist
with EXT-X-START, the init segment and the media segments. The page runs
hls.js with phone.js's own player code (window.SemmelPlayer), so the timeline
maths under test is the shipped code.

    nix-shell --run "python3 -I seek.py FILE"            # every scenario
    nix-shell --run "python3 -I seek.py FILE --headed"   # watch it

Exits 1 when a scenario's playback did not move.
"""

import argparse
import asyncio
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import uuid

from aiohttp import web
from playwright.async_api import async_playwright

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import remux  # noqa: E402
from inspect_segments import boxes, child  # noqa: E402

PHONE_JS = os.path.join(HERE, "..", "..", "web", "src", "phone.js")


def hls_url():
    text = open(PHONE_JS).read()
    return re.search(r'const HLS_JS = "([^"]+)"', text).group(1)


def marked(kind):
    """phone.js's code between its '// KIND: begin' and '// KIND: end'
    markers: `timeline` holds Player methods, `timeline functions` the
    top-level functions they call."""
    text = open(PHONE_JS).read()
    found = re.findall(rf"// {kind}: begin\n(.*?)// {kind}: end\n", text, re.S)
    if not found:
        raise SystemExit(f"phone.js has no '// {kind}: begin' ... '// {kind}: end' block")
    return "\n".join(found)


def hevc_codecs(init):
    moov = child(init, 0, len(init), "moov")
    for kind, body, stop in boxes(init, *moov):
        if kind != "trak":
            continue
        mdia = child(init, body, stop, "mdia")
        hdlr = child(init, *mdia, "hdlr")
        if init[hdlr[0] + 8 : hdlr[0] + 12] != b"vide":
            continue
        start = init.find(b"hvcC", body, stop)
        if start < 0:
            return None
        config = init[start + 4 : start + 40]
        kind = init[start - 86 : start - 82].decode()
        profile_byte = config[1]
        space = ["", "A", "B", "C"][profile_byte >> 6]
        tier = "H" if profile_byte & 0x20 else "L"
        compatibility = int(f"{struct.unpack('>I', config[2:6])[0]:032b}"[::-1], 2)
        constraints = config[6:12].rstrip(b"\0")
        text = f"{kind}.{space}{profile_byte & 0x1F}.{compatibility:X}.{tier}{config[12]}"
        return text + "".join(f".{byte:X}" for byte in constraints) + ",mp4a.40.2"
    return None


class Streams:
    def __init__(self, source, root, subtitles):
        self.source = source
        self.root = root
        self.subtitles = subtitles
        self.live = {}

    async def open(self, request):
        body = await request.json()
        start = float(body.get("start", 0))
        replaces = body.get("replaces")
        if replaces in self.live:
            self.live.pop(replaces)["process"].kill()
        id = uuid.uuid4().hex[:8]
        directory = os.path.join(self.root, id)
        process = remux.start(self.source, directory, start, self.subtitles)
        playlist = os.path.join(directory, "video.m3u8")
        opened = asyncio.get_running_loop().time()
        for _ in range(450):
            listed = open(playlist).read() if os.path.exists(playlist) else ""
            if remux.segments(listed) >= remux.READY_SEGMENTS or (process.poll() == 0 and remux.segments(listed) > 0):
                break
            await asyncio.sleep(0.1)
        else:
            process.kill()
            raise web.HTTPServiceUnavailable(text="the video did not start in time")
        offset = remux.first_time(directory)
        offset = start if offset is None else offset
        init = open(os.path.join(directory, "init.mp4"), "rb").read()
        self.live[id] = {"process": process, "dir": directory, "start": start, "offset": offset, "codecs": hevc_codecs(init)}
        took = asyncio.get_running_loop().time() - opened
        print(f"stream {id}: opened at {start:.0f} s (from {offset:.3f} s) in {took:.1f} s", flush=True)
        return web.json_response({"id": id, "start": start, "offset": offset, "playlist": f"/s/{id}/main.m3u8"})

    def find(self, request):
        live = self.live.get(request.match_info["id"])
        if live is None:
            raise web.HTTPNotFound(text="this video closed; open it again")
        return live

    async def master(self, request):
        live = self.find(request)
        text = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-STREAM-INF:BANDWIDTH=4000000"
        if live["codecs"]:
            text += f',CODECS="{live["codecs"]}"'
        return web.Response(text=text + "\nvideo.m3u8\n", content_type="application/vnd.apple.mpegurl")

    async def media(self, request):
        live = self.find(request)
        text = open(os.path.join(live["dir"], "video.m3u8")).read()
        at = live["start"] - live["offset"]
        if at >= 0.5 and "\n" in text:
            head, rest = text.split("\n", 1)
            text = f"{head}\n#EXT-X-START:TIME-OFFSET={at:.3f},PRECISE=YES\n{rest}"
        return web.Response(text=text, content_type="application/vnd.apple.mpegurl", headers={"Cache-Control": "no-store"})

    async def file(self, request):
        live = self.find(request)
        name = request.match_info["name"]
        if not re.fullmatch(r"init\.mp4|\d+\.m4s", name):
            raise web.HTTPNotFound()
        path = os.path.join(live["dir"], name)
        if not os.path.exists(path):
            raise web.HTTPNotFound()
        return web.Response(body=open(path, "rb").read(), content_type="video/mp4")

    def close(self):
        for live in self.live.values():
            live["process"].kill()


PAGE = """<!doctype html><meta charset=utf-8><title>seek</title>
<video id=video muted playsinline style="width:640px"></video>
<script src="HLS_URL"></script>
<script>
const trail = [];
function note(message, detail) { trail.push(`${message} | ${detail || ""}`); }
function clock(seconds) {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = s % 60;
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}` : `${m}:${String(r).padStart(2, "0")}`;
}
const api = "";
function loadHls() { return Promise.resolve(window.Hls); }
PLAYER_FUNCTIONS
function samePick() { return true; }
class SemmelPlayer {
  constructor(video) {
    this.video = video;
    this.session = null;
    this.base = 0;
    this.ready = false;
    this.hls = null;
    this.closed = false;
    this.chosen = null;
    this.pick = {};
    this.capabilities = { native: false };
  }
  async open(pick, options = {}) {
    const body = { start: options.start ?? 0 };
    if (this.session) body.replaces = this.session.id;
    const response = await fetch("/open", { method: "POST", body: JSON.stringify(body) });
    const session = await response.json();
    session.duration = 5486;
    this.session = session;
    this.ready = false;
    this.load(session);
  }
  pickOf() { return {}; }
  duration() { return this.session ? this.session.duration : 0; }
  clearSubtitles() {}
  chooseSubtitle() {}
  applyDefaultSubtitles() {}
  trouble(details) { note(`hls.js fatal error: ${details}`); }
  failed(message) { note(`failed: ${message}`); }
  detach() {
    if (this.hls) { this.hls.destroy(); this.hls = null; }
    this.video.removeAttribute("src");
    this.video.load();
  }
  blocked() { note("autoplay was blocked"); }
  PLAYER_CODE
}
const video = document.getElementById("video");
window.player = new SemmelPlayer(video);
video.addEventListener("waiting", () => note(`waiting at ${clock(player.now())}`));
video.addEventListener("playing", () => note(`playing at ${clock(player.now())}`));
window.state = () => {
  const ranges = Array.from({ length: video.buffered.length }, (_, i) => `${video.buffered.start(i).toFixed(2)}-${video.buffered.end(i).toFixed(2)}`).join(" ");
  const seekable = Array.from({ length: video.seekable.length }, (_, i) => `${video.seekable.start(i).toFixed(2)}-${video.seekable.end(i).toFixed(2)}`).join(" ");
  const buffers = player.hls?.bufferController?.sourceBuffer || {};
  const tracks = Object.entries(buffers).filter(([, buffer]) => buffer && buffer.buffered.length)
    .map(([kind, buffer]) => `${kind} ${buffer.buffered.start(0).toFixed(3)}`).join(" ");
  return { now: player.now(), currentTime: video.currentTime, base: player.base, buffered: ranges, tracks, seekable, paused: video.paused, readyState: video.readyState, trail: trail.splice(0) };
};
</script>"""


async def page(request):
    html = PAGE.replace("HLS_URL", hls_url()).replace("PLAYER_CODE", marked("timeline")).replace("PLAYER_FUNCTIONS", marked("timeline functions"))
    return web.Response(text=html, content_type="text/html")


SCENARIOS = {
    # Marino on 2026-11-20: plays from the start, drags to 2:00 and on to 3:49.
    "seek-from-start": {"open": 0, "play": 12, "seeks": [120, 229]},
    # Opening in the middle, as Resume does.
    "open-in-the-middle": {"open": 829, "play": 0, "seeks": []},
    # A seek back inside what the stream already has.
    "seek-back-within": {"open": 600, "play": 12, "seeks": [603]},
}


async def run(page, name, scenario, watch):
    print(f"\n== {name}", flush=True)
    await page.goto(page.url)
    await page.evaluate("(start) => player.open({}, { start })", scenario["open"])
    target = scenario["open"]
    await asyncio.sleep(scenario["play"] or 0)
    for at in scenario["seeks"]:
        await page.evaluate("(at) => player.seekTo(at)", at)
        target = at
        await asyncio.sleep(1)
    for second in range(watch):
        await asyncio.sleep(1)
        state = await page.evaluate("state()")
        for line in state.pop("trail"):
            print("   ", line, flush=True)
        if second % 3 == 0 or second == watch - 1:
            print(f"  t+{second + 1:2d}s now {state['now']:8.2f}  currentTime {state['currentTime']:8.2f}  base {state['base']:7.2f}  buffered [{state['buffered']}] starts [{state['tracks']}]  seekable [{state['seekable']}]  ready {state['readyState']}", flush=True)
    moved = state["now"] - target
    ok = watch / 3 < moved < watch + 5
    print(f"  {name}: {'ok' if ok else 'STUCK'}, now {state['now']:.1f} s, target {target} s, moved {moved:.1f} s in {watch} s", flush=True)
    return ok


async def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("source")
    parser.add_argument("--headed", action="store_true")
    parser.add_argument("--watch", type=int, default=25)
    parser.add_argument("--only", action="append")
    parser.add_argument("--no-subtitles", action="store_true", help="skip the WebVTT outputs (faster, less faithful)")
    args = parser.parse_args()

    subtitles = [] if args.no_subtitles else None
    root = tempfile.mkdtemp(prefix="semmeltv-seek-")
    streams = Streams(os.path.abspath(args.source), root, subtitles)
    app = web.Application()
    app.router.add_get("/", page)
    app.router.add_post("/open", streams.open)
    app.router.add_get("/s/{id}/main.m3u8", streams.master)
    app.router.add_get("/s/{id}/video.m3u8", streams.media)
    app.router.add_get("/s/{id}/{name}", streams.file)
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    port = site._server.sockets[0].getsockname()[1]

    results = {}
    try:
        async with async_playwright() as playwright:
            browser = await playwright.firefox.launch(headless=not args.headed, firefox_user_prefs={"media.autoplay.default": 0})
            tab = await browser.new_page()
            tab.on("console", lambda message: print("  console:", message.text, flush=True) if message.type in ("error", "warning") else None)
            await tab.goto(f"http://127.0.0.1:{port}/")
            for name, scenario in SCENARIOS.items():
                if args.only and name not in args.only:
                    continue
                results[name] = await run(tab, name, scenario, args.watch)
                streams.close()
                streams.live.clear()
            await browser.close()
    finally:
        streams.close()
        await runner.cleanup()
        shutil.rmtree(root, ignore_errors=True)
    print("\n" + json.dumps(results))
    sys.exit(0 if all(results.values()) else 1)


asyncio.run(main())
