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
        lambda: phone.api("GET", f"/api/shows/{SHOW}")["body"]["together"]["partners"] or None,
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

def mpv_gone_within(seconds):
    asked = time.monotonic()
    while tv.execute("pgrep -x mpv")[0] == 0:
        assert time.monotonic() - asked < seconds, f"mpv still runs {seconds} s after it was asked to quit"
        time.sleep(0.1)
    return time.monotonic() - asked


with scenario("Back half a second after a film starts on the TV closes the player within a few seconds"):
    phone.api("POST", "/api/play", {"movie": FILM})
    time.sleep(0.5)
    tv.send_key("esc")
    print(f"the player closed {mpv_gone_within(3):.1f} s after Back")
    eventually("nothing plays after the early Back", lambda: phone.api("GET", "/api/playback")["body"]["now_playing"] is None)

with scenario("a TV player that hangs still stops when asked, and the place it reported is kept"):
    phone.api("POST", "/api/play", {"movie": FILM})
    tv_starts_at()
    tv_seek(phone, 30)
    eventually("the TV has reported the seek", lambda: (phone.api("GET", "/api/playback")["body"]["now_playing"] or {}).get("position", 0) >= 29)
    tv.succeed("pkill -STOP -x mpv")
    phone.api("POST", "/api/playback/stop", {})
    print(f"the hung player was gone {mpv_gone_within(3):.1f} s after Stop")
    eventually("nothing plays after the hung player", lambda: phone.api("GET", "/api/playback")["body"]["now_playing"] is None)
    kept = phone.api("GET", f"/api/movies/{FILM}")["body"]["stopped_at"]
    assert kept and kept >= 29, f"the place before it hung is kept: {kept}"
