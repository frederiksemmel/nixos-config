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
