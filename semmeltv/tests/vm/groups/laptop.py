with scenario("a laptop hides no scroll bars and drags rows with the mouse without opening a title"):
    desk = Phone(playwright, device=None, viewport={"width": 760, "height": 800})
    desk.goto("/")
    desk.page.wait_for_selector(".row.posters a[data-topcoat-link]")
    desk.page.wait_for_timeout(1500)
    assert desk.js("""[document.documentElement, ...document.querySelectorAll('.row, .sheet-scroll')]
      .every((node) => getComputedStyle(node).scrollbarWidth === 'none')"""), "no scroll bars on the page, the rows and the sheet"
    row = desk.page.locator(".row.posters").first
    box = row.bounding_box()
    assert box
    y = box["y"] + box["height"] / 2
    x = box["x"] + min(box["width"] - 20, 600)
    desk.page.mouse.move(x, y)
    desk.page.mouse.down()
    for step in range(1, 11):
        desk.page.mouse.move(x - 30 * step, y)
    desk.page.mouse.up()
    desk.page.wait_for_timeout(800)
    moved = row.evaluate("(row) => row.scrollLeft")
    assert moved > 50, f"dragging a row scrolls it, scrollLeft {moved}"
    assert desk.js("location.pathname") == "/", "dragging a row opens nothing"
    tile = desk.page.locator(".row.posters a[data-topcoat-link]").first
    href = tile.get_attribute("href")
    tile.click()
    desk.until(f"location.pathname === {href!r}")
    desk.close()

with scenario("on a wide laptop the rows run to the window's edges and fade into the background there"):
    wide = Phone(playwright, device=None, viewport={"width": 1646, "height": 1029})
    wide.goto("/")
    wide.page.wait_for_selector(".row.posters a[data-topcoat-link]")
    wide.page.wait_for_timeout(1000)
    rows = wide.js("""[...document.querySelectorAll('main .row')].map((row) => {
      const box = row.getBoundingClientRect();
      const title = row.closest('.shelf').querySelector('h2').getBoundingClientRect();
      const first = [...row.children].find((tile) => tile.getClientRects().length).getBoundingClientRect();
      const style = getComputedStyle(row);
      return { left: box.left, right: box.right, window: innerWidth, title: title.left, first: first.left,
               scrolled: row.scrollLeft, fade: style.maskImage || style.webkitMaskImage || '' };
    })""")
    assert rows, "the laptop's Home has rows"
    for row in rows:
        assert abs(row["left"]) < 1 and abs(row["right"] - row["window"]) < 1, f"a row runs from edge to edge of the window: {row}"
        assert row["scrolled"] == 0 and abs(row["first"] - row["title"]) < 1, f"at rest a row's first card lines up with its title: {row}"
        assert "linear-gradient" in row["fade"], f"a row fades out at the window's edges: {row}"
    wide.close()

with scenario("the laptop player shows its controls for the mouse and keyboard, folds and stops"):
    laptop = Phone(playwright, device=None, viewport={"width": 1280, "height": 800})
    screen_posts: list[Any] = []
    laptop.page.on("request", lambda request: screen_posts.append(request.post_data_json) if request.url.endswith("/api/screen") and request.method == "POST" else None)
    film_page(laptop)
    laptop.until('document.documentElement.dataset.device === "laptop" && document.documentElement.hasAttribute("data-phone-ready")')
    switch_to(laptop, "phone")
    laptop.page.click("main button.play[data-play-movie]")
    laptop.until(PLAYS, 30000)
    shown = 'document.querySelector(".pp").classList.contains("pp-controls")'
    laptop.page.mouse.move(320, 300)
    laptop.page.mouse.move(330, 310)
    laptop.until(shown, 1000)
    laptop.page.wait_for_timeout(2200)
    assert laptop.js(f"!{shown}"), "the controls hide after a short idle"
    assert laptop.js('getComputedStyle(document.querySelector(".pp")).cursor === "none"'), "and so does the cursor"
    laptop.page.mouse.move(340, 320)
    laptop.until(shown, 1000)
    scrub = laptop.page.locator(".pp-scrub").bounding_box()
    assert scrub
    laptop.page.mouse.move(scrub["x"] + scrub["width"] / 3, scrub["y"] + scrub["height"] / 2)
    laptop.page.wait_for_timeout(2500)
    assert laptop.js(shown), "they stay up while the pointer is on them"
    laptop.page.mouse.move(320, 300)
    laptop.page.wait_for_timeout(2200)
    assert laptop.js(f"!{shown}"), "and hide once it leaves them"
    plays = len([body for body in screen_posts if (body or {}).get("action") == "play"])
    laptop.page.keyboard.press(" ")
    laptop.until(f"{VIDEO}.paused", 2000)
    assert len([body for body in screen_posts if (body or {}).get("action") == "play"]) == plays, "Space does not press the Play button behind the player again"
    laptop.page.mouse.click(320, 300)
    laptop.until(f"!{VIDEO}.paused", 2000)
    laptop.page.mouse.move(600, 600)
    laptop.page.mouse.move(610, 610)
    laptop.page.click(".pp-pill:has-text('Audio')")
    laptop.until('!document.querySelector(".pp-sheet").hidden', 2000)
    assert laptop.js('document.querySelector(".pp-sheet").getBoundingClientRect().width <= 640'), "the sheet keeps a readable width"
    laptop.page.mouse.click(320, 160)
    laptop.page.wait_for_timeout(300)
    assert laptop.js(f'document.querySelector(".pp-sheet").hidden && !{VIDEO}.paused'), "a click beside the sheet closes it and leaves playback alone"
    laptop.page.keyboard.press("Escape")
    laptop.until('!!document.querySelector(".pp.pp-mini")', 3000)
    assert laptop.js(f"!{VIDEO}.paused") and screen(laptop)["play"] is not None, "folded, it keeps playing"
    laptop.page.wait_for_timeout(600)
    bar = laptop.page.locator(".pp-minibar").bounding_box()
    assert bar
    x, y = bar["x"] + bar["width"] / 2, bar["y"] + bar["height"] / 2
    laptop.page.mouse.move(x, y)
    laptop.page.mouse.down()
    for step in range(1, 13):
        laptop.page.mouse.move(x, y - 40 * step)
    laptop.page.mouse.up()
    laptop.until('!document.querySelector(".pp-mini") && document.documentElement.classList.contains("pp-open")', 3000)
    laptop.page.wait_for_timeout(500)
    laptop.page.mouse.move(320, 200)
    laptop.page.mouse.down()
    for step in range(1, 13):
        laptop.page.mouse.move(320, 200 + 40 * step)
    laptop.page.mouse.up()
    laptop.until('!!document.querySelector(".pp.pp-mini")', 3000)
    assert laptop.js(f"!{VIDEO}.paused"), "dragging the player down does not pause it"
    laptop.page.wait_for_timeout(600)
    laptop.page.mouse.move(x, y)
    wheel(laptop, 30, 3)
    assert laptop.js('document.querySelector(".pp").classList.contains("pp-dragging")'), "scrolling on the bar pulls the player up with the touchpad"
    wheel(laptop, 40, 15)
    laptop.until('!document.querySelector(".pp-mini") && document.documentElement.classList.contains("pp-open")', 3000)
    laptop.page.wait_for_timeout(600)
    laptop.page.mouse.move(320, 300)
    wheel(laptop, -40, 18)
    laptop.until('!!document.querySelector(".pp.pp-mini")', 3000)
    assert laptop.js(f"!{VIDEO}.paused"), "scrolling the player down folds it and keeps it playing"
    laptop.page.wait_for_timeout(600)
    laptop.page.click(".pp-mini-stop")
    laptop.until('!document.querySelector(".pp")', 3000)
    eventually("the TV hears that the laptop stopped", lambda: screen(laptop)["play"] is None)
    laptop.until(NO_REMOTE, 3000)
    laptop.page.mouse.move(640, 770)
    laptop.page.mouse.down()
    for step in range(1, 13):
        laptop.page.mouse.move(640, 770 - 40 * step)
    laptop.page.mouse.up()
    laptop.page.wait_for_timeout(600)
    assert laptop.js('!document.documentElement.hasAttribute("data-sheet") && !document.querySelector(".pp")'), "watching here with nothing playing, dragging up from the bottom pulls nothing up"
    switch_to(laptop, "tv")
    laptop.until('getComputedStyle(document.querySelector(".bar")).display !== "none"', 3000)
    laptop.page.click(".bar-open")
    laptop.until('document.documentElement.hasAttribute("data-sheet")', 3000)
    laptop.page.wait_for_timeout(800)
    assert laptop.js('(() => { const box = document.querySelector(".sheet").getBoundingClientRect(); return box.width <= 640 && Math.abs(box.left - (innerWidth - box.right)) < 2; })()'), "the remote is a card on a laptop"
    laptop.close()

with scenario("Back after a reload folds the laptop's player into the bar, and stopping it stays on the film"):
    laptop = Phone(playwright, device=None, viewport={"width": 1280, "height": 800})
    film_page(laptop)
    switch_to(laptop, "phone")
    laptop.page.click("main button.play[data-play-movie]")
    laptop.until(PLAYS, 30000)
    laptop.page.reload(wait_until="commit")
    laptop.until(PLAYS, 30000)
    laptop.page.go_back()
    laptop.until('!!document.querySelector(".pp.pp-mini")', 3000)
    assert laptop.js("location.pathname") == f"/movies/{FILM}"
    laptop.page.locator(".pp-mini-text").click()
    laptop.until('!!document.querySelector(".pp") && !document.querySelector(".pp-mini")', 3000)
    laptop.page.wait_for_timeout(600)
    laptop.page.keyboard.press("Escape")
    laptop.until('!!document.querySelector(".pp.pp-mini")', 3000)
    laptop.page.wait_for_timeout(600)
    laptop.page.click(".pp-mini-stop")
    laptop.until('!document.querySelector(".pp")', 3000)
    laptop.page.wait_for_timeout(1500)
    assert laptop.js("location.pathname") == f"/movies/{FILM}", laptop.js("location.pathname")
    laptop.close()

with scenario("a laptop that goes quiet for a while loses the screen, keeps its place, and its next report takes it back"):
    laptop = Phone(playwright, device=None, viewport={"width": 1280, "height": 800})
    reports: list[Any] = []
    laptop.page.on("requestfinished", lambda request: reports.append((time.monotonic(), request.post_data_json)) if request.url.endswith("/progress") else None)
    laptop.goto(f"/movies/{FRESH_FILM}")
    laptop.until('!!document.querySelector("main button.play[data-play-movie]")')
    switch_to(laptop, "phone")
    laptop.page.click("main button.play[data-play-movie]")
    laptop.until(PLAYS, 30000)
    eventually("the TV knows the laptop plays", lambda: (screen(phone)["play"] or {}).get("state") == "playing")
    phone.goto("/")
    phone.until('document.querySelector(".bar-text strong")?.textContent === "Kinofilm"')
    eventually("the laptop has reported where it is", lambda: phone.page.wait_for_timeout(200) or [body for _, body in reports if (body or {}).get("state") == "playing"] or None, timeout=20)
    laptop.context.set_offline(True)
    heard, last_report = [(at, body) for at, body in reports if body][-1]
    while time.monotonic() < heard + QUIET_SECONDS - 3:
        view = screen(phone)
        assert view["kind"] == "phone" and (view["play"] or {}).get("state") == "playing", f"{time.monotonic() - heard:.0f} s after the laptop's last report the screen is {view}"
        assert mpv("get_property", "pid") is None, "the TV doesn't take over"
        time.sleep(2)
    view = eventually(
        "the TV gives up on the quiet laptop",
        lambda: (lambda view: view if view["play"] is None else None)(screen(phone)),
        timeout=10,
        every=1,
    )
    given_up = time.monotonic() - heard
    print(f"the TV gave up {given_up:.0f} s after the laptop's last report")
    assert view["kind"] == "phone" and view["device"]["name"] == "laptop", view
    assert "went quiet, so its place is kept" in (view["notice"] or {}).get("text", ""), view
    kept = phone.api("GET", f"/api/movies/{FRESH_FILM}")["body"]["stopped_at"]
    assert abs(kept - last_report["position"]) < 1.5, f"the laptop last reported {last_report['position']}, the TV keeps {kept}"
    assert mpv("get_property", "pid") is None, "the TV doesn't take over"
    phone.until('[...document.querySelectorAll(".toast")].some((toast) => toast.textContent.includes("went quiet"))')
    phone.until('document.querySelector(".bar-text span")?.textContent === "Nothing is playing"')
    laptop.context.set_offline(False)
    eventually(
        "the laptop's next report takes the screen back",
        lambda: (lambda view: (view["play"] or {}).get("state") == "playing" and view["device"]["name"] == "laptop")(screen(phone)),
        timeout=30,
    )
    assert laptop.js('!!document.querySelector(".pp") && !document.querySelector(".pp-failed")'), "the laptop's player stays open"
    close_player(laptop)
    laptop.close()
    stop_everything(phone)
