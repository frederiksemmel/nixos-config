import asyncio
import json
import sys
import time
from pathlib import Path

from aiohttp import web

HERE = Path(__file__).parent
LIBRARY = json.loads((HERE / "library.json").read_text())
MOVIE = json.loads((HERE / "movie.json").read_text())
SUBTITLES = json.loads((HERE / "subtitles.json").read_text())
CARDS = {card["key"]: card for row in LIBRARY.values() for card in row}
playing = {"paused": False, "position": 2223.0, "subtitle": 1, "delay": 0.0}
fetched = set()
together = {}
audio = {"output": "tv", "headphones": None, "with_headphones": [], "pairing": None, "error": None}
sound = {"changed": 0, "fail_next": False}
screen = {"kind": "tv", "boot": 1, "device": None, "play": None, "notice": None}
CONNECT_SECONDS = 1.5
SERVICE = "fake-service-token"
PROFILES = {
    1: {"id": 1, "name": "Frederik", "role": "admin", "color": "#E5484D", "password": "frederik-pass",
        "capabilities": ["control-tv", "download"]},
    2: {"id": 2, "name": "Mami", "role": "viewer", "color": "#3E63DD", "password": "mami-pass",
        "capabilities": ["control-tv"]},
    3: {"id": 3, "name": "Ramu", "role": "viewer", "color": "#8E4EC6", "password": None,
        "capabilities": ["control-tv"]},
}
sessions = {}
OPEN = ("/api/auth/choices", "/api/auth/password", "/api/auth/passkey/start")
DONE = {"state": "done", "resolution": {"kind": "fixed", "commit": "abc1234", "what": "The sheet stays shut"}}
REPORTS = [
    {"id": f"20261119-1200{index}0-ab1{index}", "at": 1763550000 + index, "when": "2026-11-19 12:00 UTC", "host": "semmeltv5",
     "person": {"name": "Mami"}, "device": "iPhone 1a2b", "page": "/", "message": message, "screenshot": None,
     "tv_screenshot": None, "status": status, "marked_at": None, "marked_by": None, "comments": []}
    for index, (message, status) in enumerate([
        ("The list jumped", {"state": "new"}),
        ("Should Back skip the player?", {"state": "needs_review", "why": "Two ways", "decide": "Skip or stop"}),
        ("The sheet opened by itself", DONE),
    ])
]
SCANNING = "Open the case near the TV and hold the button on its back until the light flashes white."


def now_playing():
    return {
        "title": "The Endless Summer", "caption": "1966", "kind": "movie", "movie": "the-endless-summer-1966",
        "position": playing["position"], "duration": 5498.56, "paused": playing["paused"],
        "muted": False, "speed": 1.0, "subtitle": playing["subtitle"], "audio": 1, "loading": False, "jump": None,
        "subtitle_delay": playing["delay"],
    }


def person(profile):
    return {key: profile[key] for key in ("id", "name", "role", "color", "capabilities")}


def tokens(request):
    return [token for token in request.cookies.get("semmeltv_session", "").split(".") if token in sessions]


def caller(request):
    found = tokens(request)
    return PROFILES[sessions[found[0]]] if found else None


def profile_of(request):
    return (caller(request) or PROFILES[1])["id"]


@web.middleware
async def gate(request, handler):
    path = request.path
    if not path.startswith("/api/") or path.startswith(OPEN):
        return await handler(request)
    if path == "/api/events" and request.headers.get("authorization") == f"Bearer {SERVICE}":
        return await handler(request)
    who = caller(request)
    if who is None:
        return web.json_response({"error": "Not signed in"}, status=401)
    return await handler(request)


def signed_in(request, profile):
    token = f"t{len(sessions) + 1}x{profile['id']}"
    sessions[token] = profile["id"]
    rest = [old for old in tokens(request) if sessions[old] != profile["id"]]
    response = web.json_response({"ok": True, "person": person(profile)})
    response.set_cookie("semmeltv_session", ".".join([token, *rest]), path="/", httponly=True, samesite="Lax")
    return response


async def auth_choices(request):
    found = tokens(request)
    active = sessions[found[0]] if found else None
    on_device = {sessions[token] for token in found}
    return web.json_response({"profiles": [
        {**person(profile), "password": profile["password"] is not None, "passkey": False, "active": profile["id"] == active,
         "signed_in": profile["id"] in on_device}
        for profile in PROFILES.values()]})


async def auth_me(request):
    who = caller(request)
    others = [person(PROFILES[sessions[token]]) for token in tokens(request)[1:]]
    return web.json_response({"person": person(who), "others": others, "passkeys": [], "password": True})


async def auth_password(request):
    body = await request.json()
    profile = PROFILES.get(body.get("profile"))
    if not profile:
        return web.json_response({"error": "There is no such profile", "ok": False}, status=404)
    if profile["password"] is None or body.get("password") != profile["password"]:
        return web.json_response({"error": "That password is wrong", "ok": False}, status=401)
    return signed_in(request, profile)


async def auth_passkey_start(request):
    profile = PROFILES[(await request.json())["profile"]]
    return web.json_response({"error": f"{profile['name']} has no passkey for this address yet", "ok": False, "password": True}, status=422)







async def tv_profile(request):
    return web.json_response({"profile": 1, "name": "Frederik", "mine": profile_of(request) == 1})



async def push_key(request):
    return web.json_response({"key": "B" + "A" * 86})


async def events(request):
    socket = web.WebSocketResponse()
    await socket.prepare(request)
    tick = 0
    sent = None
    while not socket.closed:
        await socket.send_str(json.dumps({"type": "playback", "now_playing": now_playing()}))
        await socket.send_str(json.dumps({"type": "screen", "screen": screen}))
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
    if profile_of(request) == 1:
        return web.json_response(LIBRARY)
    return web.json_response({**LIBRARY, "watching": [], "forgotten": []})


async def movie(request):
    card = CARDS.get(request.match_info["key"])
    if not card or card["kind"] != "movie":
        return web.json_response({"error": "not found"}, status=404)
    return web.json_response({**MOVIE, **card, "subtitles": 1 if card["key"] in fetched else 0,
                              "finished": False})





async def subtitles(request):
    return web.json_response(SUBTITLES)




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
        set_audio(output="headphones", headphones={**audio["headphones"], "state": "connected", "volume": {"level": 50, "muted": False}, "audio_delay": 0})


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
    set_audio(headphones={"name": "AirPods von Frederik", "address": "34:0E:22:67:03:38", "owners": [1], "state": "connected"},
              with_headphones=[1],
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
    set_audio(output="tv", headphones=None, with_headphones=[], pairing=None, error=None)
    return web.json_response(audio)


async def forget_subtitles(request):
    fetched.clear()
    return web.json_response({"ok": True})


async def volume(request):
    step = request.match_info["step"]
    phones = audio["headphones"]
    if audio["output"] != "headphones" or not phones or not phones.get("volume"):
        return web.json_response({"message": "TV volume 13", "ok": True})
    level, muted = phones["volume"]["level"], phones["volume"]["muted"]
    if step == "mute":
        muted = not muted
    else:
        level = max(0, min(100, level + (6 if step == "up" else -6)))
        muted = False
    set_audio(headphones={**phones, "volume": {"level": level, "muted": muted}})
    return web.json_response({"message": "AirPods muted" if muted else f"AirPods volume {level}%", "ok": True})


async def audio_delay(request):
    phones = audio["headphones"]
    if not phones or phones.get("state") != "connected":
        return web.json_response({"error": "The headphones aren't connected"}, status=409)
    delay = max(-500, min(500, phones.get("audio_delay", 0) + (await request.json())["by"]))
    set_audio(headphones={**phones, "audio_delay": delay})
    return web.json_response(audio)


async def fail_next_connect(request):
    sound["fail_next"] = True
    return web.json_response({"ok": True})



async def fetch_subtitles(request):
    body = await request.json()
    await asyncio.sleep(0.5)
    if "movie" not in body:
        return web.json_response({"error": "No English subtitles found"}, status=422)
    fetched.add(body["movie"])
    return web.json_response({"message": "English subtitles added", "file": "Movies/x.en.srt", "language": "en",
                              "label": "English", "release": "x", "matched": "title", "shown": False})




async def screen_view(request):
    return web.json_response(screen)














def episode(number, watched):
    return {"season": 1, "episode": number, "label": f"S1 E{number}", "short_label": f"S1 E{number}",
            "still": None, "ratings": {"imdb": None, "rotten_tomatoes": None, "fresh": None, "popcorn": None,
                                       "full": None},
            "band": None, "watched": watched, "details": "H.264 · 50m", "stopped_at": None, "duration": 3000.0,
            "runtime_minutes": 50, "tags": {}}


async def show(request):
    card = CARDS.get(request.match_info["key"])
    if not card or card["kind"] != "show":
        return web.json_response({"error": "not found"}, status=404)
    me = profile_of(request)
    partners = together.get(card["key"], set())
    people = [profile for id, profile in PROFILES.items() if id != me]
    return web.json_response({
        **card, "last_played": None, "last_played_label": None, "downloaded_by": [],
        "next": {"season": 1, "episode": 2, "label": "S1 E2", "title": None, "stopped_at": 600.0, "duration": 3000.0},
        "seasons": [{"number": 1, "label": "Season 1", "episodes": [episode(1, True), episode(2, False)]}],
        "together": {
            "partners": [{**person(profile), "with_me": False, "spot": {"state": "not_started"}}
                         for profile in people if profile["id"] in partners],
            "others": [person(profile) for profile in people if profile["id"] not in partners],
        },
    })


async def watch_together(request):
    partners = together.setdefault(request.match_info["key"], set())
    partner = int(request.match_info["partner"])
    if request.method == "PUT":
        partners.add(partner)
    else:
        partners.discard(partner)
    return web.json_response({"ok": True, "message": "You watch it together"})


async def feedback(request):
    return web.json_response({"dir": "/var/lib/semmeltv/feedback", "reports": REPORTS})


async def mark_feedback(request):
    return web.json_response({"status": await request.json()})


async def comment_feedback(request):
    was = next(report["status"] for report in REPORTS if report["id"] == request.match_info["id"])
    return web.json_response({"status": {"state": "reopened", "was": was} if was["state"] in ("needs_review", "done") else was})


def deploy_view():
    return {"revision": "aaaaaaa1", "checked_at": int(time.time()), "main": "ccccccc3", "others": 1, "asked": None, "run": None,
            "pending": [{"sha": "ccccccc3", "at": int(time.time()) - 600, "subject": "semmeltv: a deploy button",
                         "reports": [{"id": REPORTS[0]["id"], "person": "Frederik", "message": "A deploy button"}]}]}


async def deploy_state(request):
    return web.json_response(deploy_view())


async def ask_deploy(request):
    return web.json_response({**deploy_view(), "asked": {"at": int(time.time()), "by": "Frederik"}})


async def accepted(request):
    return web.json_response({"ok": True})





app = web.Application(middlewares=[gate])
app.router.add_get("/api/auth/choices", auth_choices)
app.router.add_get("/api/auth/me", auth_me)
app.router.add_post("/api/auth/password", auth_password)
app.router.add_post("/api/auth/passkey/start", auth_passkey_start)
app.router.add_get("/api/tv-profile", tv_profile)
app.router.add_get("/api/events", events)
app.router.add_get("/api/library", library)
app.router.add_get("/api/movies/{key}", movie)
app.router.add_get("/api/shows/{key}", show)
app.router.add_route("*", "/api/shows/{key}/together/{partner}", watch_together)
app.router.add_get("/api/subtitles", subtitles)
app.router.add_get("/api/playback", playback)
app.router.add_get("/api/screen", screen_view)
app.router.add_post("/api/subtitles/fetch", fetch_subtitles)
app.router.add_post("/api/volume/{step}", volume)
app.router.add_post("/api/audio", choose_output)
app.router.add_post("/api/audio/pair", pair)
app.router.add_post("/api/audio/delay", audio_delay)
app.router.add_delete("/api/audio/pair", stop_pairing)
app.router.add_delete("/api/audio/headphones", forget)
app.router.add_post("/fake/fail-next-connect", fail_next_connect)
app.router.add_post("/fake/forget-subtitles", forget_subtitles)
app.router.add_get("/api/feedback", feedback)
app.router.add_post("/api/feedback/{id}/status", mark_feedback)
app.router.add_post("/api/feedback/{id}/comments", comment_feedback)
app.router.add_get("/api/deploy", deploy_state)
app.router.add_get("/api/push", push_key)
app.router.add_post("/api/deploy", ask_deploy)
app.router.add_post("/api/{tail:.*}", accepted)
web.run_app(app, host="127.0.0.1", port=int(sys.argv[1]), print=None)
