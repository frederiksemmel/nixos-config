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
SHOW = json.loads((HERE / "show.json").read_text())
CARDS = {card["key"]: card for row in LIBRARY.values() for card in row}
playing = {"paused": False, "position": 2223.0, "subtitle": 1, "delay": 0.0}
fetched = set()
marked = {}
audio = {"output": "tv", "headphones": None, "pairing": None, "error": None}
sound = {"changed": 0, "fail_next": False}
CONNECT_SECONDS = 1.5
SCANNING = "Open the case near the TV and hold the button on its back until the light flashes white."


def now_playing():
    return {
        "title": "The Endless Summer", "caption": "1966", "movie": "the-endless-summer-1966",
        "show": None, "season": None, "episode": None,
        "position": playing["position"], "duration": 5498.56, "paused": playing["paused"],
        "muted": False, "speed": 1.0, "subtitle": playing["subtitle"], "audio": 1, "loading": False, "jump": None,
        "subtitle_delay": playing["delay"],
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
    return web.json_response({**MOVIE, **card, "subtitles": 1 if card["key"] in fetched else 0,
                              "finished": marked.get(("movie", card["key"]), False)})


async def show(request):
    seasons = [dict(season, episodes=[dict(episode, watched=marked.get(("show", episode["season"], episode["episode"]), episode["watched"]))
                                      for episode in season["episodes"]])
               for season in SHOW["seasons"]]
    return web.json_response({**SHOW, "seasons": seasons})


async def watched(request):
    body = await request.json()
    if body["kind"] == "movie":
        marked[("movie", body["key"])] = body["watched"]
    else:
        marked[("show", body["season"], body["episode"])] = body["watched"]
    return web.json_response({"message": "Marked", "ok": True, "watched": body["watched"]})


async def forget_marks(request):
    marked.clear()
    return web.json_response({"ok": True})


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


async def finish_pairing():
    await asyncio.sleep(CONNECT_SECONDS)
    if not audio["pairing"] or audio["pairing"]["state"] != "scanning":
        return
    set_audio(headphones={"name": "AirPods von Frederik", "address": "34:0E:22:67:03:38", "state": "connected"},
              pairing={"state": "paired", "message": "AirPods von Frederik are paired."})


async def pair(request):
    if audio["pairing"] and audio["pairing"]["state"] in ("scanning", "pairing"):
        return web.json_response({"error": "Pairing is already running."}, status=409)
    set_audio(pairing={"state": "scanning", "message": SCANNING}, error=None)
    asyncio.get_running_loop().create_task(finish_pairing())
    return web.json_response(audio)


async def stop_pairing(request):
    set_audio(pairing=None)
    return web.json_response(audio)


async def forget(request):
    set_audio(output="tv", headphones=None, pairing=None, error=None)
    return web.json_response(audio)


async def forget_subtitles(request):
    fetched.clear()
    return web.json_response({"ok": True})


async def fail_next_connect(request):
    sound["fail_next"] = True
    return web.json_response({"ok": True})


async def subtitle_delay(request):
    playing["delay"] = round((await request.json())["delay"], 1)
    return web.json_response({"message": f"Subtitle delay {playing['delay']:+.1f} s", "ok": True})


async def fetch_subtitles(request):
    body = await request.json()
    await asyncio.sleep(0.5)
    if "movie" not in body:
        return web.json_response({"error": "No English subtitles found"}, status=422)
    fetched.add(body["movie"])
    return web.json_response({"message": "English subtitles added", "file": "Movies/x.en.srt", "language": "en",
                              "label": "English", "release": "x", "matched": "title", "shown": False})


async def accepted(request):
    return web.json_response({"ok": True})


app = web.Application()
app.router.add_get("/api/events", events)
app.router.add_get("/api/library", library)
app.router.add_get("/api/movies/{key}", movie)
app.router.add_get("/api/shows/{key}", show)
app.router.add_get("/api/subtitles", subtitles)
app.router.add_get("/api/storage", storage)
app.router.add_get("/api/downloads", downloads)
app.router.add_get("/api/playback", playback)
app.router.add_post("/api/subtitles/delay", subtitle_delay)
app.router.add_post("/api/subtitles/fetch", fetch_subtitles)
app.router.add_post("/api/watched", watched)
app.router.add_post("/fake/forget-marks", forget_marks)
app.router.add_post("/api/audio", choose_output)
app.router.add_post("/api/audio/pair", pair)
app.router.add_delete("/api/audio/pair", stop_pairing)
app.router.add_delete("/api/audio/headphones", forget)
app.router.add_post("/fake/fail-next-connect", fail_next_connect)
app.router.add_post("/fake/forget-subtitles", forget_subtitles)
app.router.add_post("/api/{tail:.*}", accepted)
web.run_app(app, host="127.0.0.1", port=int(sys.argv[1]), print=None)
