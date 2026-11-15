HEVC_FILM = "archiv-10-2010"


def tv_set():
    status, out = tv.execute("cat /run/fake-tv/player.json")
    return json.loads(out) if status == 0 else None


def tv_set_did(action):
    return tv.succeed("cat /run/fake-tv/actions || true").count(action)


def now_playing(cookie):
    return json.loads(stream_call(cookie, "GET", "/api/playback"))["now_playing"]


with subtest("an HEVC film plays on the TV set's own player, its DTS converted to AC-3"):
    cookie = stream_cookie()
    stream_call(cookie, "POST", "/api/play", {"movie": HEVC_FILM})
    probed = eventually("the TV set reads what the Pi serves", lambda: json.loads(tv.succeed("cat /run/fake-tv/probe.json 2>/dev/null || echo null")), timeout=60)
    codecs = sorted(stream["codec_name"] for stream in probed["streams"])
    assert codecs == ["ac3", "hevc"], probed
    assert tv.execute("pgrep -x mpv")[0] != 0, "mpv stays closed while the TV set plays"
    playing = eventually("the phone sees the TV set play", lambda: (lambda shown: shown if shown and shown["position"] >= 3 and not shown["loading"] else None)(now_playing(cookie)))
    assert playing["kind"] == "movie" and playing["movie"] == HEVC_FILM, playing
    assert abs(playing["duration"] - 120) < 2, playing

    stream_call(cookie, "POST", "/api/playback/pause", {})
    eventually("the TV set pauses", lambda: tv_set()["state"] == "PAUSED_PLAYBACK")
    eventually("the phone sees the pause", lambda: now_playing(cookie)["paused"])
    stream_call(cookie, "POST", "/api/playback/resume", {})
    eventually("the TV set plays on", lambda: tv_set()["state"] == "PLAYING")

    reloads = tv_set_did("SetAVTransportURI")
    stream_call(cookie, "POST", "/api/playback/seek", {"to": 40})
    eventually("a seek reloads the converted stream from there", lambda: tv_set_did("SetAVTransportURI") > reloads and "from=40" in tv_set()["uri"])
    eventually("the position follows", lambda: 40 <= now_playing(cookie)["position"] < 60)

    stream_call(cookie, "POST", "/api/playback/stop", {})
    eventually("the TV set stops", lambda: tv_set()["state"] == "STOPPED")
    eventually("nothing plays any more", lambda: now_playing(cookie) is None)
    tv.wait_until_fails("ss -ltn | grep -q ':8097 '", timeout=10)
    kept = json.loads(stream_call(cookie, "GET", f"/api/movies/{HEVC_FILM}"))["stopped_at"]
    assert kept and kept >= 40, f"the place on the TV set is kept: {kept}"
