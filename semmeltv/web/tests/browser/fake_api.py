import asyncio
import json
import sys
from pathlib import Path

from aiohttp import web

HERE = Path(__file__).parent
LIBRARY = json.loads((HERE / "library.json").read_text())
MOVIE = json.loads((HERE / "movie.json").read_text())
SUBTITLES = json.loads((HERE / "subtitles.json").read_text())
STORAGE = json.loads((HERE / "storage.json").read_text())
DOWNLOADS = json.loads((HERE / "downloads.json").read_text())
CARDS = {card["key"]: card for row in LIBRARY.values() for card in row}
playing = {"paused": False, "position": 2223.0}
audio = {"output": "tv", "headphones": {"name": "AirPods", "state": "disconnected"}, "error": None}
sound = {"changed": 0, "fail_next": False}
CONNECT_SECONDS = 1.5


def now_playing():
    return {
        "title": "The Endless Summer", "caption": "1966", "movie": "the-endless-summer-1966",
        "show": None, "season": None, "episode": None,
        "position": playing["position"], "duration": 5498.56, "paused": playing["paused"],
        "muted": False, "speed": 1.0, "subtitle": None, "audio": 1, "loading": False, "jump": None,
    }


async def events(request):
    socket = web.WebSocketResponse()
    await socket.prepare(request)
    tick = 0
    sent = None
    while not socket.closed:
        await socket.send_str(json.dumps({"type": "playback", "now_playing": now_playing()}))
        if sent != sound["changed"]:
            sent = sound["changed"]
            await socket.send_str(json.dumps({"type": "audio", **audio}))
        await asyncio.sleep(0.5)
        tick += 1
        playing["position"] += 0.5
        if tick % 4 == 0:
            playing["paused"] = not playing["paused"]
    return socket


async def library(request):
    return web.json_response(LIBRARY)


async def movie(request):
    card = CARDS.get(request.match_info["key"])
    if not card or card["kind"] != "movie":
        return web.json_response({"error": "not found"}, status=404)
    return web.json_response({**MOVIE, **card})


async def subtitles(request):
    return web.json_response(SUBTITLES)


async def storage(request):
    return web.json_response(STORAGE)


async def downloads(request):
    return web.json_response(DOWNLOADS)


async def playback(request):
    return web.json_response({"now_playing": now_playing()})


def set_audio(**fields):
    audio.update(fields)
    sound["changed"] += 1


async def connect_headphones(fail):
    await asyncio.sleep(CONNECT_SECONDS)
    if audio["headphones"]["state"] != "connecting":
        return
    if fail:
        set_audio(output="tv", headphones={**audio["headphones"], "state": "disconnected"},
                  error="AirPods didn't connect. Open the case near the TV and try again.")
    else:
        set_audio(output="headphones", headphones={**audio["headphones"], "state": "connected"})


async def choose_output(request):
    output = (await request.json())["output"]
    if output == "headphones" and audio["output"] != "headphones":
        set_audio(headphones={**audio["headphones"], "state": "connecting"}, error=None)
        asyncio.get_running_loop().create_task(connect_headphones(sound["fail_next"]))
        sound["fail_next"] = False
    elif output == "tv":
        set_audio(output="tv", headphones={**audio["headphones"], "state": "disconnected"}, error=None)
    return web.json_response(audio)


async def fail_next_connect(request):
    sound["fail_next"] = True
    return web.json_response({"ok": True})


async def accepted(request):
    return web.json_response({"ok": True})


app = web.Application()
app.router.add_get("/api/events", events)
app.router.add_get("/api/library", library)
app.router.add_get("/api/movies/{key}", movie)
app.router.add_get("/api/subtitles", subtitles)
app.router.add_get("/api/storage", storage)
app.router.add_get("/api/downloads", downloads)
app.router.add_get("/api/playback", playback)
app.router.add_post("/api/audio", choose_output)
app.router.add_post("/fake/fail-next-connect", fail_next_connect)
app.router.add_post("/api/{tail:.*}", accepted)
web.run_app(app, host="127.0.0.1", port=int(sys.argv[1]), print=None)
