with scenario("the remote's Skip intro does what OK does while the TV shows the prompt, and goes with it"):
    intro = json.dumps({"intro": [{"start_ms": 0, "end_ms": 40000}]})
    tv.succeed(f"mkdir -p /run/fake-introdb/v4 && echo {shlex.quote(intro)} > /run/fake-introdb/v4/media")
    try:
        SKIP = 'document.querySelector(".skip-prompt .pill:not([hidden])")'
        phone.goto("/")
        at_home(phone)
        phone.page.locator(".bar-open").tap()
        phone.until('document.documentElement.hasAttribute("data-sheet")', 3000)
        assert phone.js(f"!{SKIP}"), "no Skip pill while nothing plays"
        phone.api("POST", "/api/play", {"show": SHOW, "season": 1, "episode": 2})
        assert tv_starts_at() < 20, "episode 2 starts inside its intro"
        phone.until(f"{SKIP}?.textContent === 'Skip intro'", 30000)
        assert phone.api("GET", "/api/playback")["body"]["now_playing"]["skip"] == {"kind": "intro", "label": "Skip intro"}
        tv_shows("Skip intro")
        phone.shot("remote-skip-intro")
        tv.screenshot("tv-skip-intro")
        phone.page.locator(".skip-prompt .pill").tap()
        eventually("the TV jumps to where the intro ends", lambda: (tv_position() or 0) >= 39)
        phone.until(f"!{SKIP}", 5000)
        eventually("the TV's prompt is gone", lambda: phone.api("GET", "/api/playback")["body"]["now_playing"]["skip"] is None)
        assert mpv("get_property", "pause") is False, "skipping the intro does not pause"
        refused = phone.api("POST", "/api/playback/skip-segment", {})
        assert refused["status"] == 422, refused
        assert mpv("get_property", "pause") is False, "a skip without a prompt does not pause as OK would"
        phone.page.locator(".sheet-close").tap()
        phone.until('!document.documentElement.hasAttribute("data-sheet")', 3000)
    finally:
        tv.succeed("rm -f /run/fake-introdb/v4/media")
        stop_everything(phone)
        stopped = lambda: phone.api("GET", f"/api/shows/{SHOW}")["body"]["seasons"][0]["episodes"][1]["stopped_at"]
        eventually("the TV is done with episode 2", lambda: phone.api("GET", "/api/playback")["body"]["now_playing"] is None)
        phone.api("POST", "/api/watched", {"kind": "show", "key": SHOW, "season": 1, "episode": 2, "watched": False})
        eventually("episode 2 starts over again", lambda: stopped() is None)

with scenario("Play next shows from where the credits start, and the remote and OK both play the next episode at once"):
    credits = json.dumps({"credits": [{"start_ms": 35000, "end_ms": None}]})
    tv.succeed(f"mkdir -p /run/fake-introdb/v4 && echo {shlex.quote(credits)} > /run/fake-introdb/v4/media")
    try:
        PILL = 'document.querySelector(".skip-prompt .pill:not([hidden])")'
        skip = lambda: phone.api("GET", "/api/playback")["body"]["now_playing"]["skip"]
        episodes = lambda: phone.api("GET", f"/api/shows/{SHOW}")["body"]["seasons"][0]["episodes"]
        phone.goto("/")
        at_home(phone)
        phone.page.locator(".bar-open").tap()
        phone.until('document.documentElement.hasAttribute("data-sheet")', 3000)
        counted = tv_said("^up next: /")
        phone.api("POST", "/api/play", {"show": SHOW, "season": 1, "episode": 1})
        tv_starts_at()
        tv_seek(phone, 25)
        assert skip() is None, "no Play next before the credits"
        tv_seek(phone, 37)
        phone.until(f"{PILL}?.textContent === 'Play next'", 30000)
        assert skip() == {"kind": "next", "label": "Play next"}
        tv_shows("Play next")
        phone.shot("remote-play-next")
        tv.screenshot("tv-play-next")
        phone.page.locator(".skip-prompt .pill").tap()
        eventually("the remote's Play next plays episode 2", lambda: tv_episode(phone) == 2, timeout=20)
        assert tv_said("^up next: /") == counted, "Play next does not count down"
        eventually("episode 1 counts as watched", lambda: episodes()[0]["watched"])
        tv_starts_at()
        tv_seek(phone, 37)
        tv_shows("Play next")
        tv.send_key("ret")
        eventually("OK on Play next plays episode 3", lambda: tv_episode(phone) == 3, timeout=20)
        assert tv_said("^up next: /") == counted, "OK on Play next does not count down"
        tv_starts_at()
        tv_seek(phone, 37)
        time.sleep(3)
        assert skip() is None, "no Play next on the last episode"
        phone.page.locator(".sheet-close").tap()
        phone.until('!document.documentElement.hasAttribute("data-sheet")', 3000)
    finally:
        tv.succeed("rm -f /run/fake-introdb/v4/media")
        stop_everything(phone)
        phone.api("POST", "/api/watched", {"kind": "show", "key": SHOW, "season": 1, "episode": 2, "watched": False})
        eventually("episode 2 starts over again", lambda: episodes()[1]["stopped_at"] is None)

with scenario("Back on the TV and the remote's × hide Skip recap and Skip intro and playing goes on, and the next Back stops"):
    sections = json.dumps({"recap": [{"start_ms": 0, "end_ms": 20000}], "intro": [{"start_ms": 20000, "end_ms": 40000}]})
    tv.succeed(f"mkdir -p /run/fake-introdb/v4 && echo {shlex.quote(sections)} > /run/fake-introdb/v4/media")
    try:
        PILL = 'document.querySelector(".skip-prompt .pill:not([hidden])")'
        now_playing = lambda: phone.api("GET", "/api/playback")["body"]["now_playing"]
        phone.goto("/")
        at_home(phone)
        phone.page.locator(".bar-open").tap()
        phone.until('document.documentElement.hasAttribute("data-sheet")', 3000)
        phone.api("POST", "/api/play", {"show": SHOW, "season": 1, "episode": 2})
        assert tv_starts_at() < 10, "episode 2 starts inside its recap"
        phone.until(f"{PILL}?.textContent === 'Skip recap'", 30000)
        tv.send_key("esc")
        eventually("Back hides Skip recap", lambda: now_playing()["skip"] is None)
        phone.until(f"!{PILL}", 5000)
        assert (tv_position() or 0) < 20, "Back does not skip the recap"
        assert mpv("get_property", "pause") is False, "Back on the pill does not pause"
        phone.until(f"{PILL}?.textContent === 'Skip intro'", 30000)
        phone.shot("remote-skip-intro-hide")
        phone.page.locator(".skip-prompt .skip-hide").tap()
        eventually("the remote's × hides Skip intro on the TV", lambda: now_playing()["skip"] is None)
        phone.until('!document.querySelector(".skip-prompt button")', 5000)
        assert (tv_position() or 0) < 40, "the × does not skip the intro"
        assert phone.api("POST", "/api/playback/hide-skip", {})["status"] == 422
        assert now_playing() is not None, "hiding the pill does not stop playing"
        tv.send_key("esc")
        eventually("Back without a pill stops", lambda: now_playing() is None)
        phone.page.locator(".sheet-close").tap()
        phone.until('!document.documentElement.hasAttribute("data-sheet")', 3000)
    finally:
        tv.succeed("rm -f /run/fake-introdb/v4/media")
        stop_everything(phone)
        stopped = lambda: phone.api("GET", f"/api/shows/{SHOW}")["body"]["seasons"][0]["episodes"][1]["stopped_at"]
        eventually("the TV is done with episode 2", lambda: phone.api("GET", "/api/playback")["body"]["now_playing"] is None)
        phone.api("POST", "/api/watched", {"kind": "show", "key": SHOW, "season": 1, "episode": 2, "watched": False})
        eventually("episode 2 starts over again", lambda: stopped() is None)
