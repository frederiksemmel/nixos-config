with scenario("skipping on the TV keeps its sound output running, so the TV never has to pick the sound up again"):
    tv_plays_film(phone)
    resumed_at = tv_position() or 0.0
    try:
        def near_the_start():
            phone.api("POST", "/api/playback/seek", {"to": 20})
            time.sleep(1)
            return 19 <= (tv_position() or 0) < 40

        eventually("the TV plays from 20 s, where three skips stay far from the end", near_the_start, timeout=30, every=1)
        tv_plays_past(22)
        skipped_from = tv_position() or 0.0
        status = tv.succeed("grep -l 'state:' /proc/asound/card*/pcm*p/sub*/status").split()[0]
        tv.succeed(f"(for i in $(seq 800); do head -1 {status}; sleep 0.005; done > /tmp/sound-states 2>&1) </dev/null >/dev/null 2>&1 &")
        for _ in range(3):
            assert phone.api("POST", "/api/playback/skip", {"direction": "forward"})["status"] == 200
            time.sleep(0.7)
        eventually("the skips reach the TV", lambda: (tv_position() or 0) > skipped_from + 5)
        time.sleep(2)
        states = set(tv.succeed("cat /tmp/sound-states").split("\n")) - {""}
        assert states <= {"state: RUNNING", "state: XRUN"}, f"the sound output stopped during the skips: {states}"
    finally:
        tv_seek(phone, int(resumed_at))
        stop_everything(phone)

with scenario("the remote moves the TV's subtitles half a second at a time"):
    tv_plays_film(phone)
    tv.send_key("s")
    eventually("the TV shows subtitles", lambda: mpv("get_property", "sub-visibility") is True and mpv("get_property", "sid") not in (None, False, "no"))
    phone.goto("/")
    at_home(phone)
    phone.page.locator(".bar-open").tap()
    phone.until('document.documentElement.hasAttribute("data-sheet")', 3000)
    phone.until('!document.querySelector("[data-tracks=subtitles] .delay").hidden')
    assert phone.js('!document.querySelector(".sound")'), "no Sound choice while no headphones are paired"
    row = phone.page.locator("[data-tracks=subtitles] .delay")
    row.scroll_into_view_if_needed()
    row.locator(".choice").nth(1).tap()
    phone.until('document.querySelector(".now-state").dataset.delay === "0.5" && document.querySelector("[data-tracks=subtitles] .delay output").textContent === "+0.5 s"')
    eventually("mpv shows them half a second later", lambda: abs((mpv("get_property", "sub-delay") or 0) - 0.5) < 0.01)
    row.locator(".choice").nth(0).tap()
    phone.until('document.querySelector(".now-state").dataset.delay === "0.0"')
    eventually("and back", lambda: abs(mpv("get_property", "sub-delay") or 0) < 0.01)
    phone.page.locator(".sheet-close").tap()
    phone.until('!document.documentElement.hasAttribute("data-sheet")', 3000)
    stop_everything(phone)

with scenario("a tap on the remote fades the TV's sound out before it pauses, and back in when it plays"):
    phone.goto("/")
    at_home(phone)
    switch_to(phone, "tv")
    phone.api("POST", "/api/play", {"show": SHOW, "season": 1, "episode": 1})
    tv_starts_at()
    phone.until('document.querySelector(".bar-text span")?.textContent.includes("Anfang")', 30000)
    phone.page.locator(".bar-toggle").tap()
    eventually("the tap pauses the TV", lambda: mpv("get_property", "pause") is True)
    assert mpv("get_property", "volume") == 0, "the sound faded out before the pause"
    phone.until('document.querySelector(".now-state")?.classList.contains("paused") && !document.documentElement.hasAttribute("data-flip")', 5000)
    phone.page.locator(".bar-toggle").tap()
    eventually("the next tap resumes the TV", lambda: mpv("get_property", "pause") is False)
    eventually("the sound fades back in to full", lambda: mpv("get_property", "volume") == 100)
    stop_everything(phone)

with scenario("an iPhone back from the background shows the remote closed, and a swipe closes it after the live connection restarts"):
    iphone = Phone(playwright, "webkit", "iPhone 15")
    film_page(iphone)
    tv_plays_film(iphone)
    iphone.page.locator(".bar-open").tap()
    iphone.until('document.documentElement.hasAttribute("data-sheet")', 3000)
    iphone.page.wait_for_timeout(800)
    skipped_from = tv_position() or 0.0
    iphone.page.locator('.skip[data-skip="forward"]').tap()
    eventually("the skip reaches the TV", lambda: (tv_position() or 0) > skipped_from + 3)
    iphone.js("document.getElementById('sheet').dataset.rendered = 'before'")
    goes_away(iphone, 5)
    iphone.until('!document.getElementById("sheet").dataset.rendered', 10000)
    assert iphone.js('!document.documentElement.hasAttribute("data-sheet")'), "the page comes back without the remote"
    iphone.page.locator(".bar-open").tap()
    iphone.until('document.documentElement.hasAttribute("data-sheet")', 3000)
    iphone.page.wait_for_timeout(800)
    iphone.js("document.getElementById('sheet').dataset.rendered = 'before'")
    iphone.js('window.dispatchEvent(new Event("online"))')
    iphone.until('!document.getElementById("sheet").dataset.rendered', 10000)
    iphone.page.wait_for_timeout(500)
    assert iphone.js('document.documentElement.hasAttribute("data-sheet") && !document.getElementById("sheet").inert'), "the re-rendered remote stays open and takes touches"
    grip = iphone.page.locator(".sheet-top").bounding_box()
    assert grip
    x, y = grip["x"] + grip["width"] / 2, grip["y"] + grip["height"] / 2
    iphone.page.mouse.move(x, y)
    iphone.page.mouse.down()
    for step in range(1, 13):
        iphone.page.mouse.move(x, y + 40 * step)
    assert iphone.js('document.documentElement.hasAttribute("data-sheet-drag")'), "the remote follows the finger"
    iphone.page.mouse.up()
    iphone.until('!document.documentElement.hasAttribute("data-sheet")', 3000)
    stop_everything(iphone)
    iphone.close()
