with scenario("an episode that ends on the TV counts down to the next, OK plays it at once, and nothing follows the last"):
    counted = tv_said("^up next: /")
    phone.api("POST", "/api/play", {"show": SHOW, "season": 1, "episode": 1})
    tv_starts_at()
    tv_seek(phone, 54)
    eventually("the TV counts down to episode 2", lambda: tv_said("^up next: /") > counted, timeout=60, every=0.3)
    counting = time.monotonic()
    tv.send_key("ret")
    eventually("OK opens episode 2", lambda: mpv("get_property", "pid") is not None, timeout=30, every=0.3)
    assert time.monotonic() - counting < UP_NEXT_SECONDS * 0.7, "OK plays the next episode without waiting for the countdown"
    eventually("the TV plays episode 2", lambda: tv_episode(phone) == 2)
    counted = tv_said("^up next: /")
    tv_starts_at()
    tv_seek(phone, 54)
    eventually("the TV counts down to episode 3", lambda: tv_said("^up next: /") > counted, timeout=60, every=0.3)
    counting = time.monotonic()
    time.sleep(1)
    tv.screenshot("tv-up-next")
    shown = "\n".join(tv.get_screen_text_variants())
    assert "Up next" in shown and re.search(r"Plays ?in", shown), f"the TV shows the Up next card: {shown!r}"
    eventually("the countdown plays episode 3", lambda: mpv("get_property", "pid") is not None, timeout=30, every=0.3)
    assert time.monotonic() - counting > UP_NEXT_SECONDS * 0.7, "episode 3 waits for the countdown"
    eventually("the TV plays episode 3", lambda: tv_episode(phone) == 3)
    counted = tv_said("^up next: /")
    tv_starts_at()
    tv_seek(phone, 54)
    tv_plays_nothing()
    time.sleep(3)
    assert tv_said("^up next: /") == counted, "nothing counts down after the last episode"
    assert mpv("get_property", "pid") is None, "nothing plays after the last episode"
