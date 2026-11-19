with scenario("a film played on the TV resumes where it was after the app restarts"):
    film_page(phone)
    switch_to(phone, "tv")
    phone.page.locator("main button.play[data-play-movie]").tap()
    tv_starts_at()
    tv_seek(phone, 100)
    tv_plays_past(105)
    left_at = tv_position() or 0
    tv.screenshot("tv-playing")
    tv.systemctl("restart cage-tty1.service")
    app_restarted()
    tv_plays_nothing()
    film_page(phone)
    switch_to(phone, "tv")
    phone.page.locator("main button.play[data-play-movie]").tap()
    resumed = tv_starts_at()
    print(f"left at {left_at:.1f} s, resumed at {resumed:.1f} s")
    assert 60 < resumed and left_at - 31 <= resumed <= left_at + 3, f"left at {left_at}, resumed at {resumed}"
    stop_everything(phone)

with scenario("an episode played again after it finished starts over, then resumes where it stopped"):
    episode = {"show": SHOW, "season": 1, "episode": 1}
    phone.api("POST", "/api/play", episode)
    tv_starts_at()
    counted = tv_said("^up next: /")
    tv_seek(phone, 54)
    tv_plays_nothing()
    eventually("the TV counts down to episode 2", lambda: tv_said("^up next: /") > counted)
    tv.send_key("esc")
    eventually("Back cancels the countdown", lambda: tv_said("^up next: cancelled") > 0, timeout=5)
    time.sleep(UP_NEXT_SECONDS + 2)
    assert mpv("get_property", "pid") is None, "after Back, episode 2 does not play by itself"
    eventually(
        "the episode counts as watched",
        lambda: phone.api("GET", f"/api/shows/{SHOW}")["body"]["seasons"][0]["episodes"][0]["watched"],
    )
    phone.api("POST", "/api/play", episode)
    again = tv_starts_at()
    assert again < 5, f"a finished episode started at {again}"
    tv_seek(phone, 30)
    tv_plays_past(32)
    stopped_at = tv_position() or 0
    phone.api("POST", "/api/playback/stop", {})
    tv_plays_nothing()
    phone.api("POST", "/api/play", episode)
    resumed = tv_starts_at()
    assert abs(resumed - stopped_at) < 3, f"stopped at {stopped_at}, resumed at {resumed}"
    phone.api("POST", "/api/playback/stop", {})
    tv_plays_nothing()
    opened = phone.api(
        "POST",
        "/api/stream/open",
        episode | {"client": {"hevc": False, "eac3": False, "ac3": False}, "device": {"id": "vm-test", "name": "test"}},
    )
    assert opened["status"] == 200, opened
    assert abs(opened["body"]["start"] - resumed) < 3, f"the phone stream starts at {opened['body']['start']}, the TV stopped at {resumed}"
    phone.api("DELETE", f"/api/stream/{opened['body']['id']}")
