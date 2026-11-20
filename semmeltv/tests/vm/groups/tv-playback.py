from PIL import Image, ImageChops

with subtest("the TV boots into the app and shows the library"):
    tv_shows("Newly Added", timeout=180)
    tv.wait_for_text("Testfilm", timeout=60)
    tv.screenshot("tv-home")

with scenario("no cursor shows on the TV when a pointer moves"):
    tv.screenshot("cursor-before")
    tv.succeed("systemd-run --unit=move-mouse semmeltv-move-mouse")
    tv.wait_until_succeeds("grep -q semmeltv-vm-test-mouse /proc/bus/input/devices")
    time.sleep(5)
    tv.screenshot("cursor-after")
    tv.succeed("systemctl stop move-mouse")
    before = Image.open(os.path.join(OUT, "cursor-before.png")).convert("RGB")
    after = Image.open(os.path.join(OUT, "cursor-after.png")).convert("RGB")
    difference = ImageChops.difference(before, after)
    changed = difference.getbbox()
    if changed:
        difference.point(lambda value: 255 if value else 0).save(os.path.join(OUT, "cursor-difference.png"))
    assert changed is None, f"the screen changed at {changed} after the pointer moved"

with scenario("a film played on the TV for a while puts it in Continue Watching on the TV and the phone"):
    phone.api("POST", "/api/play", {"movie": FILM})
    started = tv_starts_at()
    eventually(
        "the TV plays at double speed",
        lambda: phone.api("POST", "/api/playback/speed", {"speed": 2}) and mpv("get_property", "speed") == 2,
        every=1,
    )
    tv_plays_past(started + WATCHING_SECONDS + 4, timeout=60)
    phone.api("POST", "/api/playback/stop", {})
    tv_plays_nothing()
    tv_shows("Currently Watching")
    tv.screenshot("tv-continue-watching")
    phone.goto("/")
    phone.until(
        """[...document.querySelectorAll("main .shelf")].some((shelf) =>
             shelf.querySelector("h2")?.textContent === "Currently Watching" && shelf.textContent.includes("Testfilm"))"""
    )
    phone.shot("continue-watching")

with scenario("the TV profile picker switches whose Home the TV shows"):
    phone.goto("/")
    assert phone.api("GET", "/api/tv-profile")["body"]["name"] == "Frederik"
    tv.send_key("esc")
    tv.send_key("up")
    tv.send_key("up")
    tv.send_key("up")
    tv.screenshot("tv-profiles")
    tv.send_key("down")
    tv.send_key("ret")
    shown = eventually("the TV shows Mami's Home", lambda: (lambda body: body if body["name"] == "Mami" else None)(phone.api("GET", "/api/tv-profile")["body"]))
    assert shown["mine"] is False
    eventually("the TV's Home loses Frederik's Currently Watching", lambda: "Currently Watching" not in tv.get_screen_text(), timeout=60, every=2)
    tv.screenshot("tv-mami")
    phone.goto("/settings")
    phone.page.locator('[data-post="/api/tv-profile"]').tap()
    eventually("the TV shows Frederik's Home again", lambda: phone.api("GET", "/api/tv-profile")["body"]["mine"])
    tv.send_key("up")
    tv_shows("Currently Watching")

with scenario("on the TV, Up from a series' first season picks someone to watch it with, and its episodes still play"):
    tv.send_key("esc")
    for key in ["down", "down", "down", "up", "left", "left", "left", "ret"]:
        tv.send_key(key)
    tv_shows("Season 1")
    tv.send_key("up")
    tv_shows("Watch together")
    tv.screenshot("tv-watch-together")
    first = phone.api("GET", f"/api/shows/{SHOW}")["body"]["together"]["others"][0]
    tv.send_key("ret")
    paired = eventually(
        f"OK pairs Frederik with {first['name']}",
        lambda: phone.api("GET", f"/api/shows/{SHOW}")["body"]["together"]["partners"],
    )
    assert [partner["name"] for partner in paired] == [first["name"]], paired
    tv_shows("started")
    tv.send_key("down")
    tv.send_key("ret")
    eventually("Down and OK play the first episode", lambda: tv_episode(phone) == 1)
    tv.send_key("esc")
    tv_plays_nothing()
    phone.api("DELETE", f"/api/shows/{SHOW}/together/{first['id']}")
    tv.send_key("esc")

HEVC_FILM = "archiv-10-2010"


def tv_set():
    status, out = tv.execute("cat /run/fake-tv/player.json")
    return json.loads(out) if status == 0 else None


def tv_set_did(action):
    return tv.succeed("cat /run/fake-tv/actions || true").count(action)


def now_playing(cookie):
    return json.loads(stream_call(cookie, "GET", "/api/playback"))["now_playing"]


with scenario("an HEVC film plays on the TV set's own player, its DTS converted to AC-3"):
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
