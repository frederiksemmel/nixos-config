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
screen = {"kind": "tv", "device": None, "play": None, "notice": None}
screen_answers = {"delay": 0.0, "refuse": False}
tv = {"on": True}
opens = []
streams = {}
progress_log = []
ended = set()
screen_log = []
CONNECT_SECONDS = 1.5
SERVICE = "fake-service-token"
PROFILES = {
    1: {"id": 1, "name": "Frederik", "role": "admin", "color": "#E5484D", "password": "frederik-pass"},
    2: {"id": 2, "name": "Mami", "role": "viewer", "color": "#3E63DD", "password": "mami-pass"},
    3: {"id": 3, "name": "Ramu", "role": "viewer", "color": "#8E4EC6", "password": None},
}
sessions = {}
invites = {"good-invite": 2}
OPEN = ("/api/auth/choices", "/api/auth/password", "/api/auth/passkey/start", "/api/auth/invite/")
SCANNING = "Open the case near the TV and hold the button on its back until the light flashes white."


def now_playing():
    if not tv["on"]:
        return None
    return {
        "title": "The Endless Summer", "caption": "1966", "movie": "the-endless-summer-1966",
        "show": None, "season": None, "episode": None,
        "position": playing["position"], "duration": 5498.56, "paused": playing["paused"],
        "muted": False, "speed": 1.0, "subtitle": playing["subtitle"], "audio": 1, "loading": False, "jump": None,
        "subtitle_delay": playing["delay"],
    }


def person(profile):
    return {key: profile[key] for key in ("id", "name", "role", "color")}


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
    if not path.startswith("/api/") or path.startswith(OPEN) or path.startswith("/api/stream/") and path != "/api/stream/open":
        return await handler(request)
    if request.headers.get("x-fake-harness") or path == "/api/events" and request.headers.get("authorization") == f"Bearer {SERVICE}":
        return await handler(request)
    who = caller(request)
    if who is None:
        return web.json_response({"error": "Not signed in"}, status=401)
    if path.startswith(("/api/storage", "/api/auth/members", "/api/tv/pair")) and who["role"] != "admin":
        return web.json_response({"error": "Only an admin can do that"}, status=403)
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


async def auth_switch(request):
    profile = (await request.json())["profile"]
    found = tokens(request)
    target = [token for token in found if sessions[token] == profile]
    if not target or PROFILES[profile]["role"] == "admin":
        return web.json_response({"error": f"Sign in as {PROFILES[profile]['name']}", "ok": False, "sign_in": True}, status=401)
    response = web.json_response({"ok": True, "person": person(PROFILES[profile])})
    response.set_cookie("semmeltv_session", ".".join([target[0], *[token for token in found if token != target[0]]]), path="/")
    return response


async def auth_sign_out(request):
    found = tokens(request)
    if found:
        sessions.pop(found[0], None)
    response = web.json_response({"ok": True, "person": None})
    response.set_cookie("semmeltv_session", ".".join(found[1:]), path="/")
    return response


async def auth_invite(request):
    profile = invites.get(request.match_info["token"])
    if profile is None:
        return web.json_response({"error": "This invite link is not valid", "ok": False}, status=404)
    if request.method == "GET":
        return web.json_response({"person": person(PROFILES[profile])})
    return signed_in(request, PROFILES[profile])


async def auth_members(request):
    return web.json_response({"members": [
        {**person(profile), "password": True, "passkeys": 0, "sessions": 1, "last_seen_at": None, "invite_expires_at": None}
        for profile in PROFILES.values()]})


async def auth_member_invite(request):
    return web.json_response({"ok": True, "message": "Invite link", "link": "http://127.0.0.1:3000/invite/good-invite", "expires_in_days": 7})


async def tv_profile(request):
    return web.json_response({"profile": 1, "name": "Frederik", "mine": profile_of(request) == 1})


async def fake_sign_out_all(request):
    sessions.clear()
    return web.json_response({"ok": True})


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
                              "finished": marked.get((profile_of(request), "movie", card["key"]), False)})


async def show(request):
    seasons = [dict(season, episodes=[dict(episode, watched=marked.get((profile_of(request), "show", episode["season"], episode["episode"]), episode["watched"]))
                                      for episode in season["episodes"]])
               for season in SHOW["seasons"]]
    return web.json_response({**SHOW, "seasons": seasons})


async def watched(request):
    body = await request.json()
    if body["kind"] == "movie":
        marked[(profile_of(request), "movie", body["key"])] = body["watched"]
    else:
        marked[(profile_of(request), "show", body["season"], body["episode"])] = body["watched"]
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


def film_place(position, state, session=None):
    return {"state": state, "kind": "movie", "movie": "the-endless-summer-1966", "name": "The Endless Summer",
            "caption": "1966", "position": position, "session": session, "paused": False}


def place_of(body, state):
    if body.get("movie"):
        return film_place(0.0, state)
    season, episode = body.get("season") or 8, body.get("episode") or 1
    return {"state": state, "kind": "episode", "show": body["show"], "season": season, "episode": episode,
            "name": "Brooklyn Nine-Nine", "caption": f"{episode}. Episode {episode}", "position": 0.0,
            "session": None, "paused": False}


async def screen_view(request):
    return web.json_response(screen)


async def screen_command(request):
    body = await request.json()
    action = body["action"]
    screen_log.append(body)
    await asyncio.sleep(screen_answers["delay"])
    if screen_answers["refuse"]:
        return web.json_response({"message": "The TV said no", "ok": False}, status=422)
    if action == "flip" and body["to"] == "phone":
        play = film_place(playing["position"], "playing", "fake-session") if tv["on"] else None
        tv["on"] = False
        screen.update(kind="phone", device=body["device"], play=play)
    elif action == "flip":
        if screen["play"]:
            playing["position"] = body.get("position") or screen["play"]["position"]
            tv["on"] = True
        screen.update(kind="tv", device=None, play=None)
    elif action == "play":
        tv["on"] = False
        screen.update(kind="phone", device=body["device"], play=place_of(body, "starting"))
    elif action in ("failed", "stop"):
        screen.update(play=None)
    return web.json_response({"message": "done", "ok": True})


async def set_screen_answers(request):
    screen_answers.update(await request.json())
    return web.json_response(screen_answers)


async def stream_open(request):
    body = await request.json()
    opens.append(body)
    id = f"fake-session-{len(opens)}"
    streams[id] = body
    common = {"id": id, "playlist": f"/api/stream/{id}/main.m3u8", "backdrop": "", "start": 0, "audio": [],
              "audio_index": None, "converted_audio": False, "subtitles": [], "subtitle_delay": 0, "credits": None,
              "bitrate": 4000000, "heavy": False}
    if body.get("show"):
        season, episode = body.get("season") or 8, body.get("episode") or 1
        following = {"season": season, "episode": episode + 1, "label": f"S{season} E{episode + 1}",
                     "title": f"Episode {episode + 1}", "still": f"/api/shows/{body['show']}/episodes/{season}/{episode + 1}/still"}
        return web.json_response({**common, "kind": "show", "key": body["show"], "title": "Brooklyn Nine-Nine",
                                  "caption": f"{episode}. Episode {episode}", "season": season, "episode": episode,
                                  "poster": f"/api/shows/{body['show']}/poster", "duration": 1300.0,
                                  "next": following if episode < 10 else None})
    return web.json_response({**common, "kind": "movie", "key": "the-endless-summer-1966", "title": "The Endless Summer",
                              "caption": "1966", "season": None, "episode": None,
                              "poster": "/api/movies/the-endless-summer-1966/poster", "duration": 5498.56, "next": None})


async def stream_progress(request):
    id = request.match_info["id"]
    state = (await request.json())["state"]
    progress_log.append({"session": id, "state": state, "play": bool(screen["play"])})
    if id in ended:
        return web.json_response({"error": "this video closed; open it again"}, status=404)
    if state in ("stopped", "ended"):
        ended.add(id)
    play = screen["play"]
    opened = streams.get(id, {})
    holder = screen["kind"] == "phone" and screen["device"] and opened.get("device", {}).get("id") == screen["device"]["id"]
    if play and state == "start":
        play.update(state="playing", session=id)
    elif play and state in ("stopped", "ended") and play["session"] == id:
        screen.update(play=None)
    elif not play and holder and state in ("playing", "paused"):
        screen.update(play={**place_of(opened, "playing"), "session": id, "paused": state == "paused"})
    return web.json_response({"message": "Noted", "ok": True})


async def fake_progress_log(request):
    return web.json_response(progress_log[-12:])


async def drop_play(request):
    screen.update(play=None, notice={"id": 99, "text": "Simulator went quiet, so its place is kept", "tone": "quiet"})
    return web.json_response({"ok": True})


async def reset_screen(request):
    screen.update(kind="tv", device=None, play=None, notice=None)
    tv["on"] = True
    screen_answers.update(delay=0.0, refuse=False)
    return web.json_response({"ok": True})


async def fake_opens(request):
    return web.json_response(opens)


async def fake_screen_log(request):
    return web.json_response(screen_log)


async def stream_slow(request):
    await asyncio.sleep(30)
    return web.Response(status=404)


async def accepted(request):
    return web.json_response({"ok": True})


app = web.Application(middlewares=[gate])
app.router.add_get("/api/auth/choices", auth_choices)
app.router.add_get("/api/auth/me", auth_me)
app.router.add_post("/api/auth/password", auth_password)
app.router.add_post("/api/auth/passkey/start", auth_passkey_start)
app.router.add_post("/api/auth/switch", auth_switch)
app.router.add_post("/api/auth/sign-out", auth_sign_out)
app.router.add_get("/api/auth/invite/{token}", auth_invite)
app.router.add_post("/api/auth/invite/{token}", auth_invite)
app.router.add_get("/api/auth/members", auth_members)
app.router.add_post("/api/auth/members/{id}/invite", auth_member_invite)
app.router.add_post("/fake/sign-out-all", fake_sign_out_all)
app.router.add_get("/api/tv-profile", tv_profile)
app.router.add_get("/api/events", events)
app.router.add_get("/api/library", library)
app.router.add_get("/api/movies/{key}", movie)
app.router.add_get("/api/shows/{key}", show)
app.router.add_get("/api/subtitles", subtitles)
app.router.add_get("/api/storage", storage)
app.router.add_get("/api/downloads", downloads)
app.router.add_get("/api/playback", playback)
app.router.add_get("/api/screen", screen_view)
app.router.add_post("/api/screen", screen_command)
app.router.add_post("/fake/screen-answers", set_screen_answers)
app.router.add_post("/api/stream/open", stream_open)
app.router.add_post("/api/stream/{id}/progress", stream_progress)
app.router.add_get("/fake/opens", fake_opens)
app.router.add_post("/fake/reset-screen", reset_screen)
app.router.add_post("/fake/drop-play", drop_play)
app.router.add_get("/fake/progress-log", fake_progress_log)
app.router.add_get("/fake/screen-log", fake_screen_log)
app.router.add_get("/api/stream/{tail:.*}", stream_slow)
app.router.add_post("/api/subtitles/delay", subtitle_delay)
app.router.add_post("/api/subtitles/fetch", fetch_subtitles)
app.router.add_post("/api/watched", watched)
app.router.add_post("/fake/forget-marks", forget_marks)
app.router.add_post("/api/volume/{step}", volume)
app.router.add_post("/api/audio", choose_output)
app.router.add_post("/api/audio/pair", pair)
app.router.add_post("/api/audio/delay", audio_delay)
app.router.add_delete("/api/audio/pair", stop_pairing)
app.router.add_delete("/api/audio/headphones", forget)
app.router.add_post("/fake/fail-next-connect", fail_next_connect)
app.router.add_post("/fake/forget-subtitles", forget_subtitles)
app.router.add_post("/api/{tail:.*}", accepted)
web.run_app(app, host="127.0.0.1", port=int(sys.argv[1]), print=None)
