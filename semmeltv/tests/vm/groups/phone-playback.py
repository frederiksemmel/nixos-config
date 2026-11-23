with scenario("a phone plays the next episode in place of the one that ends"):
    plays_the_next_episodes(phone)

with scenario("Apple's native HLS path plays an episode, the next one and its subtitles"):
    apple = Phone(playwright)
    apple.context.add_init_script("Object.defineProperty(Navigator.prototype, 'vendor', { get: () => 'Apple Computer, Inc.' })")
    assert apple.js('document.createElement("video").canPlayType("application/vnd.apple.mpegurl")'), "this Chromium plays HLS itself"
    plays_the_next_episodes(apple, through_the_end=False)
    assert apple.js(f'{VIDEO}.src.endsWith("/main.m3u8") && !window.Hls'), "the video plays the playlist itself, without hls.js"
    ranges = apple.js(f"Array.from({{ length: {VIDEO}.seekable.length }}, (_, i) => [{VIDEO}.seekable.start(i), {VIDEO}.seekable.end(i)])")
    print(f"the native player can seek in {ranges} at {apple.js(VIDEO + '.currentTime')} s, the player at {phone_time(apple)} s")
    controls(apple)
    apple.page.locator(".pp-pill", has_text="Audio").tap()
    apple.until('!document.querySelector(".pp-sheet").hidden')
    apple.page.locator(".pp-sheet section").nth(1).locator(".pp-choice", has_text="English").tap()
    apple.until(f"({ACTIVE_CUES})().length > 0", 15000)
    checked = 0
    for _ in range(12):
        at = phone_time(apple)
        cues = apple.js(ACTIVE_CUES)
        if 0.3 < at % 5 < 3.7:
            assert cues == [f"Episode 2 line {int(at // 5) + 1}"], f"at {at:.2f} s the subtitles show {cues}"
            checked += 1
        time.sleep(0.4)
    assert checked >= 5, f"{checked} subtitle checks"
    reopened = []
    apple.page.on("request", lambda request: reopened.append(request.url) if request.url.endswith("/api/stream/open") else None)
    jumped_from = float(phone_time(apple))
    apple.page.keyboard.press("ArrowLeft")
    eventually("a jump back plays from 10 s earlier", lambda: apple.js(PLAYS) and jumped_from - 13 < phone_time(apple) < jumped_from - 5, timeout=20)
    print(f"jumped back from {jumped_from:.1f} s to {phone_time(apple):.1f} s, {'by opening the stream again' if reopened else 'in the stream'}")
    close_player(apple)
    eventually("the TV hears that the Apple phone stopped", lambda: screen(apple)["play"] is None)
    switch_to(apple, "tv")
    apple.close()

with scenario("a phone pauses where someone it watches the series with stopped, and says whose place it is"):
    miriam = Phone(playwright, who="Miriam")
    miriam.goto("/")
    miriam.until(LOADED)
    episode_2 = {"show": SHOW, "season": 1, "episode": 2}

    def stops_at(someone, position):
        opened = someone.api("POST", "/api/stream/open", episode_2 | {"client": NO_DOLBY, "device": {"id": "vm-setup", "name": "setup"}})
        assert opened["status"] == 200, opened
        stream = opened["body"]["id"]
        for state, at in (("start", 0), ("stopped", position)):
            reported = someone.api("POST", f"/api/stream/{stream}/progress", {"position": at, "played": at, "state": state})
            assert reported["status"] == 200, reported
        someone.api("DELETE", f"/api/stream/{stream}")

    stops_at(miriam, 30)
    stops_at(phone, 2)
    others = phone.api("GET", f"/api/shows/{SHOW}")["body"]["together"]["others"]
    miriam_id = next(person["id"] for person in others if person["name"] == "Miriam")
    assert phone.api("PUT", f"/api/shows/{SHOW}/together/{miriam_id}")["body"]["ok"]
    phone.goto(f"/shows/{SHOW}")
    phone.until('document.querySelectorAll("main button.episode[data-play-show]").length === 3')
    switch_to(phone, "phone")
    episode = phone.page.locator('main button.episode[data-episode="2"]')
    episode.scroll_into_view_if_needed()
    episode.tap()
    phone.until(PLAYS, 30000)
    phone.until(f'{VIDEO}.paused && [...document.querySelectorAll(".pp-toast")].some((toast) => toast.textContent === "Paused where Miriam stopped")', 45000)
    paused_at = phone_time(phone)
    assert 29 <= paused_at <= 32, f"paused at {paused_at} s, Miriam stopped at 30 s"
    phone.page.wait_for_timeout(1500)
    assert phone.js(f"{VIDEO}.paused"), "it stays paused until the viewer goes on"
    close_player(phone)
    assert phone.api("DELETE", f"/api/shows/{SHOW}/together/{miriam_id}")["body"]["ok"]
    miriam.close()
