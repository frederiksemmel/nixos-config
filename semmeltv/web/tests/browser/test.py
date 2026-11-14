import sys
import time
import urllib.error
import urllib.request
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

BASE = sys.argv[1]
FAKE = sys.argv[2]
TILE = 'main a[data-topcoat-link][href^="/movies/"]'
failures = []


def check(engine, step, ok, detail=""):
    print(f'{engine:8} {"ok  " if ok else "FAIL"} {step} {detail}', flush=True)
    if not ok:
        failures.append(f"{engine}: {step}")


def answers(page, engine, step):
    started = time.monotonic()
    try:
        page.wait_for_function("new Promise((done) => requestAnimationFrame(() => done(true)))", timeout=5000)
        check(engine, f"{step}: page answers", True, f"{int((time.monotonic() - started) * 1000)} ms")
        return True
    except Exception:
        check(engine, f"{step}: page answers", False)
        return False


def until(page, engine, step, condition, timeout=8000):
    try:
        page.wait_for_function(condition, timeout=timeout)
        check(engine, step, True)
    except Exception:
        check(engine, step, False, f"at {page.evaluate('location.pathname')}")


def scrolls(page, engine, step):
    page.evaluate("window.scrollTo(0, 0)")
    page.wait_for_timeout(200)
    if engine == "webkit":
        page.keyboard.press("PageDown")
    else:
        page.mouse.move(200, 400)
        page.mouse.wheel(0, 600)
    try:
        page.wait_for_function("scrollY > 100", timeout=3000)
    except Exception:
        pass
    y = page.evaluate("scrollY")
    check(engine, f"{step}: page scrolls", y > 100, f"scrollY {y}")
    page.evaluate("window.scrollTo(0, 0)")
    page.wait_for_timeout(200)


def scrolls_both_ways(page, engine, step):
    start = page.evaluate("scrollY")
    reached = [start]
    for key, dy in (("PageUp", -600), ("PageDown", 600)):
        if engine == "webkit":
            page.keyboard.press(key)
        else:
            page.mouse.move(200, 400)
            page.mouse.wheel(0, dy)
        page.wait_for_timeout(700)
        reached.append(page.evaluate("scrollY"))
    check(engine, f"{step}: page scrolls up and down", reached[1] < start - 100 and reached[2] > reached[1] + 100, f"scrollY {reached}")


def settings_subpages(page, engine):
    page.locator('a[href="/settings"]').first.tap()
    until(page, engine, "Settings opens", 'location.pathname === "/settings" && document.querySelector("main .settings")')
    page.wait_for_timeout(500)
    for sub in ("/downloads", "/storage"):
        link = page.locator(f'main a[href="{sub}"]')
        link.scroll_into_view_if_needed()
        link.tap()
        until(page, engine, f"{sub} opens", f'location.pathname === "{sub}" && !document.querySelector("semmel-shell")')
        answers(page, engine, sub)
        if sub == "/downloads":
            field = page.locator("main input[type=search]")
            field.tap()
            page.keyboard.type("dune")
            check(engine, "the Downloads field takes typing", field.input_value() == "dune", repr(field.input_value()))
        page.locator("main a.back").first.tap()
        until(page, engine, f"Back from {sub} returns to Settings",
              'location.pathname === "/settings" && !document.querySelector("semmel-shell") && document.querySelector("main .settings")')
        page.wait_for_timeout(500)
        answers(page, engine, f"Settings after Back from {sub}")
        scrolls_both_ways(page, engine, f"Settings after Back from {sub}")
    page.locator("main a.back").first.tap()
    until(page, engine, "Back from Settings returns home", 'location.pathname === "/" && !document.querySelector("semmel-shell")')
    page.wait_for_timeout(500)
    scrolls(page, engine, "after Back from Settings")


def opens_title(page, engine, step):
    tile = page.locator(TILE).first
    href = tile.get_attribute("href")
    tile.scroll_into_view_if_needed()
    tile.tap()
    until(page, engine, f"{step}: tapped title opens",
          f'location.pathname === {href!r} && !document.querySelector("semmel-shell") && document.querySelector("main .detail h1")')


def sound(page, engine):
    phones = page.locator(".sound .choice").nth(1)
    phones.scroll_into_view_if_needed()
    phones.tap()
    until(page, engine, "tapping AirPods shows it connecting",
          'document.querySelector(".sound .answer").textContent.startsWith("Connecting")', 3000)
    until(page, engine, "AirPods take over the sound",
          'document.querySelector(".sound").dataset.output === "headphones" && getComputedStyle(document.querySelector(".bar-phones")).display !== "none"', 5000)
    until(page, engine, "a toast says the sound moved to the AirPods",
          '[...document.querySelectorAll(".toast")].some((toast) => toast.textContent.includes("Sound is on AirPods"))', 3000)
    until(page, engine, "the volume keys follow the sound to the AirPods",
          'document.querySelector(".volume-head span").textContent.includes("AirPods") && document.querySelector(".volume-head output").textContent === "50%"', 3000)
    until(page, engine, "the AirPods timing shows while they have the sound",
          'document.querySelector(".sound-timing output")?.textContent === "0 ms"', 3000)
    later = page.locator(".sound-timing .choice").nth(1)
    later.scroll_into_view_if_needed()
    later.tap()
    until(page, engine, "+20 ms makes the AirPods sound later", 'document.querySelector(".sound-timing output")?.textContent === "+20 ms"', 3000)
    louder = page.locator('.keys.volume .key[aria-label="Louder"]')
    louder.scroll_into_view_if_needed()
    louder.tap()
    until(page, engine, "Louder turns the AirPods up", 'document.querySelector(".volume-head output").textContent === "56%"', 3000)
    page.locator(".sound .choice").nth(0).scroll_into_view_if_needed()
    page.locator(".sound .choice").nth(0).tap()
    until(page, engine, "TV takes the sound back", 'document.querySelector(".sound").dataset.output === "tv" && !document.querySelector(".sound-timing")', 3000)
    fake("/fake/fail-next-connect")
    page.locator(".sound .choice").nth(1).tap()
    until(page, engine, "a failed connect says why",
          '[...document.querySelectorAll(".toast[data-tone=error]")].some((toast) => toast.textContent.includes("didn\'t connect"))', 5000)


def to_settings(page, engine, step):
    page.locator('a[href="/settings"]').first.tap()
    until(page, engine, f"{step}: Settings opens", 'location.pathname === "/settings" && document.querySelector("main .headphones")')
    page.wait_for_timeout(500)
    page.locator("main .headphones").scroll_into_view_if_needed()


def home(page, engine, step):
    page.locator("main a.back").first.tap()
    until(page, engine, f"{step}: Back returns home", 'location.pathname === "/" && !document.querySelector("semmel-shell")')
    page.wait_for_timeout(500)


def headphones(page, engine):
    to_settings(page, engine, "pairing")
    pair = page.locator("main .headphones .pill:not(.quiet)")
    pair.tap()
    until(page, engine, "pairing says what to do with the headphones",
          '[...document.querySelectorAll("main .headphones .answer")].some((note) => note.textContent.includes("hold the button"))', 3000)
    page.locator("main .headphones .pill.quiet", has_text="Cancel").tap()
    until(page, engine, "Cancel stops the pairing", '!document.querySelector("main .headphones .pill.quiet")', 3000)
    pair.tap()
    until(page, engine, "the paired headphones show up connected",
          '[...document.querySelectorAll("main .headphones .setting-name")].some((name) => name.textContent.includes("AirPods von Frederik") && name.textContent.includes("Connected"))', 5000)
    answers(page, engine, "Settings after pairing")
    home(page, engine, "after pairing")
    page.locator(".bar-open").tap()
    until(page, engine, "sheet opens after pairing", 'document.documentElement.hasAttribute("data-sheet")', 3000)
    page.wait_for_timeout(800)
    sound(page, engine)
    page.locator(".sheet-close").tap()
    until(page, engine, "sheet closes after the sound choice", '!document.documentElement.hasAttribute("data-sheet")', 3000)
    page.wait_for_timeout(800)
    to_settings(page, engine, "forgetting")
    page.locator("main .headphones .pill.quiet", has_text="Forget").tap()
    until(page, engine, "Forget removes the headphones and the Sound choice",
          '!document.querySelector("main .headphones .pill.quiet") && !document.querySelector(".sound")', 3000)
    home(page, engine, "after forgetting")


def forward(route):
    try:
        route.fulfill(response=route.fetch(url=FAKE + urlparse(route.request.url).path))
    except Exception:
        pass


def harness(path):
    return urllib.request.Request(FAKE + path, headers={"x-fake-harness": "1"})


def fake(path):
    urllib.request.urlopen(urllib.request.Request(FAKE + path, method="POST", data=b""))


def subtitle_timing(page, engine):
    row = page.locator("[data-tracks=subtitles] .delay")
    until(page, engine, "the subtitle timing shows while subtitles are on",
          '!document.querySelector("[data-tracks=subtitles] .delay").hidden', 3000)
    row.scroll_into_view_if_needed()
    row.locator(".choice").nth(1).tap()
    until(page, engine, "+0.5 s moves the subtitles later",
          'document.querySelector(".now-state").dataset.delay === "0.5" && document.querySelector("[data-tracks=subtitles] .delay output").textContent === "+0.5 s"', 3000)
    row.locator(".choice").nth(0).tap()
    until(page, engine, "−0.5 s moves them back", 'document.querySelector(".now-state").dataset.delay === "0.0"', 3000)


def finds_subtitles(page, engine):
    until(page, engine, "a film without subtitles offers to find them", '!!document.querySelector("main .find-subtitles-row .pill")', 3000)
    page.wait_for_timeout(300)
    find = page.locator("main .find-subtitles-row .pill")
    if not find.count():
        return
    find.tap()
    until(page, engine, "finding subtitles says they were added and how well they fit",
          '[...document.querySelectorAll(".toast")].some((toast) => toast.textContent.includes("found by title only")) && document.querySelector("main .find-subtitles-row .pill").hidden', 5000)


def marks_watched(page, engine):
    film = page.locator("main .mark-film")
    until(page, engine, "a film page offers to mark it watched", '!!document.querySelector("main .mark-film")', 3000)
    film.scroll_into_view_if_needed()
    film.tap()
    until(page, engine, "marking a film watched flips the button",
          'document.querySelector("main .mark-film").getAttribute("aria-pressed") === "true"', 3000)
    film.tap()
    until(page, engine, "and back to unwatched",
          'document.querySelector("main .mark-film").getAttribute("aria-pressed") === "false"', 3000)


def marks_episodes(page, engine):
    page.goto(BASE + "/shows/brooklyn-nine-nine", wait_until="commit")
    until(page, engine, "a show page lists episodes with a watched toggle", 'document.querySelectorAll("main .episode-row .mark-seen").length > 1', 8000)
    page.wait_for_timeout(500)
    toggle = page.locator("main .episode-row .mark-seen").nth(1)
    before = toggle.get_attribute("aria-pressed")
    toggle.scroll_into_view_if_needed()
    toggle.tap()
    after = "false" if before == "true" else "true"
    until(page, engine, "tapping an episode's check flips it",
          f'document.querySelectorAll("main .episode-row .mark-seen")[1].getAttribute("aria-pressed") === "{after}"', 3000)
    check(engine, "and does not start the episode", page.evaluate("location.pathname") == "/shows/brooklyn-nine-nine")
    page.reload(wait_until="commit")
    until(page, engine, "the episode stays flipped after a reload",
          f'document.querySelectorAll("main .episode-row .mark-seen")[1]?.getAttribute("aria-pressed") === "{after}"', 8000)


def raise_toast(page, text, tone="info"):
    page.evaluate("([text, tone]) => window.dispatchEvent(new CustomEvent('semmeltv:toast', { detail: { text, tone } }))", [text, tone])


def toasts(page, engine):
    check(engine, "the top bar has no microphone text", page.evaluate('!document.getElementById("microphone-note")'))
    if engine == "chromium":
        page.locator("#microphone-talk").tap()
        until(page, engine, "a microphone problem shows as a toast, not in the bar",
              '!!document.querySelector(".toast[data-tone=error]") && document.getElementById("microphone").dataset.state === "idle"', 5000)
    page.evaluate('document.querySelectorAll(".toast").forEach((toast) => toast.remove())')
    raise_toast(page, "first")
    raise_toast(page, "second")
    check(engine, "a toast has no backdrop blur, so its fade is one smooth step",
          page.evaluate('["backdropFilter", "webkitBackdropFilter"].every((name) => [undefined, "none"].includes(getComputedStyle(document.querySelector(".toast"))[name]))'))
    until(page, engine, "toasts stack, newest on top", '[...document.querySelectorAll(".toast")].map((t) => t.textContent).join() === "second,first"', 2000)
    newest = page.locator(".toast").first.bounding_box()
    page.mouse.move(newest["x"] + 60, newest["y"] + 20)
    page.mouse.down()
    for step in range(1, 9):
        page.mouse.move(newest["x"] + 60 + 25 * step, newest["y"] + 20)
    page.mouse.up()
    until(page, engine, "swiping a toast sideways sends it away", '[...document.querySelectorAll(".toast:not([data-leaving])")].map((t) => t.textContent).join() === "first"', 2000)
    page.locator(".toast:not([data-leaving])").first.click()
    until(page, engine, "tapping a toast dismisses it", '!document.querySelector(".toast:not([data-leaving])")', 2000)
    raise_toast(page, "goes by itself")
    until(page, engine, "a toast goes away by itself", '!document.querySelector(".toast")', 7000)


def tv_plays(engine, step, wanted, timeout=5.0):
    import json
    deadline = time.time() + timeout
    while True:
        with urllib.request.urlopen(harness("/api/playback")) as response:
            playing = json.load(response)["now_playing"] is not None
        if playing == wanted or time.time() > deadline:
            check(engine, step, playing == wanted)
            return
        time.sleep(0.2)


HLS_STUB = (
    "window.Hls = class { static isSupported() { return true; } "
    "constructor() { this.handlers = {}; this.subtitleTracks = []; } "
    "on(name, handler) { this.handlers[name] = handler; } loadSource() {} "
    "attachMedia(media) { setTimeout(() => { this.handlers.m?.(); media.dispatchEvent(new Event('playing')); }, 50); } "
    "destroy() {} };"
    "window.Hls.Events = { ERROR: 'e', MANIFEST_PARSED: 'm', SUBTITLE_TRACKS_UPDATED: 's' };"
)


def fake_get(path):
    import json
    with urllib.request.urlopen(harness(path)) as response:
        return json.load(response)


def fake_until(page, engine, step, condition, timeout=6.0, detail=lambda: ""):
    deadline = time.time() + timeout
    while True:
        ok = condition()
        if ok or time.time() > deadline:
            check(engine, step, bool(ok), "" if ok else str(detail()))
            return
        page.wait_for_timeout(200)


def opened_since(before):
    return [(o.get("episode"), o.get("replaces")) for o in fake_get("/fake/opens")[before:]]


def stub_hls(page):
    page.route("https://cdn.jsdelivr.net/**", lambda route: route.fulfill(body=HLS_STUB, content_type="text/javascript"))


def stop_player(page, engine, step):
    if not page.evaluate('!!document.querySelector(".pp.pp-mini")'):
        page.locator(".pp-close").tap()
    until(page, engine, f"{step}: the chevron folds the player into the bar", '!!document.querySelector(".pp.pp-mini")', 3000)
    page.locator(".pp-mini-stop").tap()
    until(page, engine, f"{step}: and its stop button closes it", '!document.querySelector(".pp")', 3000)


def next_episode(page, engine):
    stub_hls(page)
    page.goto(BASE + "/shows/brooklyn-nine-nine", wait_until="commit")
    until(page, engine, "the show page lists its episodes", 'document.querySelectorAll("main button.episode[data-play-show]").length >= 3', 8000)
    page.wait_for_timeout(800)
    if page.evaluate('document.querySelector("#mode").dataset.mode') == "tv":
        page.locator("#mode").tap()
        until(page, engine, "the switch moves to the phone", 'document.querySelector("#mode").dataset.mode === "phone"', 5000)
        page.wait_for_timeout(800)
        if page.evaluate('!!document.querySelector(".pp")'):
            stop_player(page, engine, "the handed-over film")
    fake_until(page, engine, "closing the player tells the TV it stopped", lambda: fake_get("/api/screen")["play"] is None)
    before = len(fake_get("/fake/opens"))
    episode = page.locator('main button.episode[data-episode="1"]')
    episode.scroll_into_view_if_needed()
    episode.tap()
    until(page, engine, "tapping an episode opens it on the phone", 'document.querySelector(".pp-title")?.textContent.includes("1. Episode 1")', 5000)
    fake_until(page, engine, "and streams it", lambda: [o.get("episode") for o in fake_get("/fake/opens")[before:]] == [1])
    page.wait_for_timeout(300)
    page.locator('.pp-pill[aria-label="Next episode"]').tap()
    fake_until(page, engine, "Next streams the next episode in place of this one",
               lambda: opened_since(before) == [(1, None), (2, f"fake-session-{before + 1}")], detail=lambda: opened_since(before))
    until(page, engine, "and the player shows it", 'document.querySelector(".pp-title")?.textContent.includes("2. Episode 2")', 5000)
    page.wait_for_timeout(500)
    page.evaluate('document.querySelector(".pp-video").dispatchEvent(new Event("ended"))')
    fake_until(page, engine, "an episode that ends plays the next one",
               lambda: opened_since(before)[2:] == [(3, f"fake-session-{before + 2}")], detail=lambda: opened_since(before))
    until(page, engine, "and the player shows that one", 'document.querySelector(".pp-title")?.textContent.includes("3. Episode 3") && !document.querySelector(".pp-failed")', 5000)
    page.wait_for_timeout(500)
    check(engine, "the TV keeps the screen on the phone through it", fake_get("/api/screen")["play"]["episode"] == 3)
    page.go_back()
    until(page, engine, "Back folds the player into the bar and stays on the show",
          '!!document.querySelector(".pp.pp-mini") && location.pathname === "/shows/brooklyn-nine-nine" && !document.documentElement.classList.contains("pp-open")', 3000)
    check(engine, "the bar shows what plays", page.evaluate('document.querySelector(".pp-mini-text").textContent.includes("3. Episode 3")'))
    page.wait_for_timeout(800)
    check(engine, "and it keeps playing there", (fake_get("/api/screen")["play"] or {}).get("episode") == 3)
    scrolls(page, engine, "with the player in the bar")
    page.locator(".pp-mini-text").tap()
    until(page, engine, "tapping the bar opens the player again",
          '!document.querySelector(".pp-mini") && document.documentElement.classList.contains("pp-open") && history.state?.player', 3000)
    page.wait_for_timeout(500)
    page.locator(".pp-close").tap()
    until(page, engine, "the chevron folds it again", '!!document.querySelector(".pp.pp-mini")', 3000)
    page.wait_for_timeout(500)
    streams = len(fake_get("/fake/opens"))
    playing = page.locator('main button.episode[data-episode="3"]')
    playing.scroll_into_view_if_needed()
    playing.tap()
    until(page, engine, "tapping the episode that plays opens its player",
          '!document.querySelector(".pp-mini") && document.querySelector(".pp-title").textContent.includes("3. Episode 3")', 3000)
    page.wait_for_timeout(800)
    check(engine, "without streaming it again", len(fake_get("/fake/opens")) == streams)
    stop_player(page, engine, "after the bar")
    fake_until(page, engine, "and the TV hears that it stopped", lambda: fake_get("/api/screen")["play"] is None)
    page.wait_for_timeout(800)
    check(engine, "and the player stays closed", page.evaluate('!document.querySelector(".pp")'))
    check(engine, "and the bar is the TV remote again", page.evaluate('getComputedStyle(document.querySelector(".bar")).visibility !== "hidden" && !document.elementFromPoint(innerWidth / 2, innerHeight - 30).closest(".pp")'))


def screen_answers(**answers):
    import json
    request = urllib.request.Request(FAKE + "/fake/screen-answers", data=json.dumps(answers).encode(),
                                     headers={"content-type": "application/json"}, method="POST")
    urllib.request.urlopen(request).close()


def switch_answers_at_once(page, engine):
    page.route("https://cdn.jsdelivr.net/**", lambda route: route.fulfill(body=HLS_STUB, content_type="text/javascript"))
    mode = 'document.querySelector("#mode").dataset.mode'
    screen_answers(delay=1.5, refuse=False)
    page.locator("#mode").tap()
    until(page, engine, "the switch moves at the tap, before the TV answers", f'{mode} === "moving"', 400)
    until(page, engine, "and settles once the TV agrees", f'{mode} === "phone"', 4000)
    page.evaluate('document.querySelector("#mode").click()')
    until(page, engine, "switching back moves at once too", f'{mode} === "tv"', 400)
    page.wait_for_timeout(1800)
    check(engine, "and stays on the TV once it agrees", page.evaluate(f'{mode} === "tv"'))
    screen_answers(delay=0.5, refuse=True)
    page.locator("#mode").tap()
    until(page, engine, "a refused switch first moves", f'{mode} === "moving"', 400)
    until(page, engine, "then snaps back and says why",
          f'{mode} === "tv" && [...document.querySelectorAll(".toast")].some((t) => t.textContent.includes("The TV said no"))', 4000)
    screen_answers(delay=0.0, refuse=False)
    page.wait_for_timeout(800)


def watches_on_the_phone(page, engine):
    stub_hls(page)
    page.locator("#mode").tap()
    until(page, engine, "flipping to the phone opens the player", 'document.documentElement.dataset.mode === "phone" && !!document.querySelector(".pp")', 5000)
    tv_plays(engine, "flipping to the phone stops the TV", False)
    page.wait_for_timeout(600)
    page.reload(wait_until="commit")
    until(page, engine, "after a reload the player matches the backend", 'document.documentElement.dataset.mode === "phone" && !!document.querySelector(".pp")', 8000)
    page.evaluate('document.querySelector(".pp-tv")?.click()')
    until(page, engine, "Continue on TV closes the player", '!document.querySelector(".pp") && document.documentElement.dataset.mode === "tv"', 5000)
    tv_plays(engine, "Continue on TV plays on the TV again", True)
    page.locator("#mode").tap()
    until(page, engine, "flipping again moves it to the phone", 'document.documentElement.dataset.mode === "phone" && !!document.querySelector(".pp")', 5000)
    page.evaluate('document.querySelector("#mode").click()')
    until(page, engine, "flipping back leaves the switch on the TV while the TV plays",
          'document.documentElement.dataset.mode === "tv" && !document.querySelector(".pp")', 5000)
    tv_plays(engine, "and the TV plays", True)
    page.wait_for_timeout(800)


WATCHING = '[...document.querySelectorAll("main .shelf h2")].some((heading) => heading.textContent.includes("Currently Watching"))'


def fake_status(path):
    try:
        return urllib.request.urlopen(FAKE + path).status
    except urllib.error.HTTPError as error:
        return error.code


def sign_in(page, engine, name, password, step):
    until(page, engine, f"{step}: the profile picker shows", 'location.pathname === "/profiles" && !!document.querySelector(".profile-choice")')
    page.locator(".profile-choice", has_text=name).click()
    until(page, engine, f"{step}: {name} is asked for a password", '!document.querySelector("#password-form").hidden', 5000)
    page.fill("#password-input", password)
    page.locator("#password-form button[type=submit]").click()


def signs_in(page, engine):
    fake("/fake/sign-out-all")
    check(engine, "the API refuses a request without a session", fake_status("/api/library") == 401)
    page.goto(BASE + "/movies/the-endless-summer-1966", wait_until="commit")
    until(page, engine, "a signed-out visitor lands on the picker", 'location.pathname === "/profiles"')
    check(engine, "with no player or remote", page.evaluate('!document.querySelector(".bar-open") && !document.querySelector("#microphone")'))
    check(engine, "a profile nobody set up says so on its button",
          page.evaluate('[...document.querySelectorAll(".profile-choice")].some((choice) => choice.textContent.includes("Ramu") && choice.textContent.includes("Not set up yet"))'))
    page.locator(".profile-choice", has_text="Ramu").click()
    until(page, engine, "and tapping it says how to set it up", 'document.querySelector("#sign-in-note").textContent.includes("invite link")', 5000)
    sign_in(page, engine, "Frederik", "wrong-pass", "a wrong password")
    until(page, engine, "a wrong password says so", 'document.querySelector("#sign-in-note").textContent.includes("wrong")', 5000)
    page.fill("#password-input", "frederik-pass")
    page.locator("#password-form button[type=submit]").click()
    until(page, engine, "the right one goes back where the visitor was headed", 'location.pathname === "/movies/the-endless-summer-1966"', 8000)
    page.goto(BASE + "/", wait_until="commit")


def profiles(page, engine):
    page.goto(BASE + "/", wait_until="commit")
    until(page, engine, "Frederik's Home has his Currently Watching", WATCHING)
    check(engine, "the avatar in the top bar is Frederik's", page.evaluate('document.querySelector(".profile-link .avatar").textContent.trim() === "F"'))
    page.locator(".profile-link").tap()
    until(page, engine, "the avatar opens the picker with Frederik watching", 'location.pathname === "/profiles" && document.querySelector(".profile-choice.active .profile-name").textContent === "Frederik"')
    sign_in(page, engine, "Mami", "mami-pass", "switching to Mami")
    until(page, engine, "Mami lands on Home", 'location.pathname === "/" && document.querySelector(".profile-link .avatar").textContent.trim() === "M"', 8000)
    page.wait_for_timeout(800)
    check(engine, "Mami's Home has her own (empty) Currently Watching", page.evaluate(f"!({WATCHING})"))
    page.goto(BASE + "/settings", wait_until="commit")
    until(page, engine, "a viewer's Settings open", '!!document.querySelector("main .settings")')
    check(engine, "and have no Storage, Downloads or TV pairing", page.evaluate('!document.querySelector(\'main a[href="/storage"], main a[href="/downloads"], [data-post="/api/tv/pair"]\')'))
    check(engine, "Mami can put her profile on the TV", page.evaluate('!!document.querySelector(\'[data-post="/api/tv-profile"]\')'))
    page.goto(BASE + "/storage", wait_until="commit")
    until(page, engine, "a viewer sent to Storage lands on Home", 'location.pathname === "/"')
    page.goto(BASE + "/profiles", wait_until="commit")
    sign_in(page, engine, "Frederik", "frederik-pass", "back to the admin, who always signs in again")
    until(page, engine, "Frederik is back", 'location.pathname === "/" && document.querySelector(".profile-link .avatar").textContent.trim() === "F"', 8000)
    until(page, engine, "with his Currently Watching", WATCHING)
    page.goto(BASE + "/profiles", wait_until="commit")
    until(page, engine, "Mami stays on this device", '[...document.querySelectorAll(".profile-choice")].some((choice) => choice.textContent.includes("On this device"))')
    page.locator(".profile-choice", has_text="Mami").tap()
    until(page, engine, "and switching to her needs no password", 'location.pathname === "/" && document.querySelector(".profile-link .avatar").textContent.trim() === "M"', 8000)
    page.goto(BASE + "/profiles", wait_until="commit")
    sign_in(page, engine, "Frederik", "frederik-pass", "and back to Frederik")
    until(page, engine, "Frederik again", 'location.pathname === "/" && document.querySelector(".profile-link .avatar").textContent.trim() === "F"', 8000)


def invite(p):
    engine = "invite"
    browser = p.chromium.launch()
    page = browser.new_context(**p.devices["Pixel 7"]).new_page()
    page.set_default_timeout(8000)
    errors = []
    page.on("pageerror", lambda error: errors.append(f"{error} {getattr(error, 'stack', '')}"))
    page.route("**/api/**", forward)
    page.goto(BASE + "/invite/good-invite", wait_until="commit")
    until(page, engine, "an invite greets the person it is for", '!!document.querySelector("#redeem") && document.querySelector("main h1").textContent.includes("Mami")')
    page.locator("#redeem").tap()
    until(page, engine, "continuing signs them in and offers a passkey and a password", 'location.pathname === "/account" && !!document.querySelector("#add-passkey") && !!document.querySelector("#password-set")', 8000)
    page.goto(BASE + "/invite/made-up", wait_until="commit")
    until(page, engine, "a bad invite link says so", '(document.querySelector("main")?.textContent || "").includes("not valid")')
    if "not valid" not in (page.evaluate('document.body ? document.body.innerText : ""') or ""):
        print("invite page shows:", page.evaluate('document.documentElement.outerHTML')[:3000])
    check(engine, "no script errors", not errors, "; ".join(errors))
    browser.close()


def top_bar(page, engine):
    bar = page.evaluate("""() => {
      const box = (selector) => document.querySelector(selector).getBoundingClientRect();
      const top = box("header.top"), brand = box(".brand"), talk = box(".microphone-talk"), settings = [...document.querySelectorAll("header.top .top-link")].pop().getBoundingClientRect();
      const border = parseFloat(getComputedStyle(document.querySelector("header.top")).borderBottomWidth);
      return { above: talk.top - top.top, below: top.bottom - border - talk.bottom, left: brand.left, right: innerWidth - settings.right,
               brand: brand.top + brand.height / 2 - (talk.top + talk.height / 2) };
    }""")
    check(engine, "the top bar's buttons sit in its middle", abs(bar["above"] - bar["below"]) < 1 and abs(bar["brand"]) < 1, str(bar))
    check(engine, "the top bar's buttons line up with its edges", abs(bar["left"] - bar["right"]) < 1, str(bar))


def restart(page, engine):
    def down(route):
        if route.request.headers.get("x-topcoat-runtime") == "true":
            route.fulfill(status=502, body="")
        else:
            route.fallback()

    page.route(BASE + "/**", down)
    page.evaluate('window.dispatchEvent(new Event("topcoat:invalidate-prefetches"))')
    tile = page.locator(TILE).first
    href = tile.get_attribute("href")
    tile.scroll_into_view_if_needed()
    tile.tap()
    until(page, engine, "a title tapped while the server restarts says it is waiting",
          '[...document.querySelectorAll(".toast")].some((toast) => toast.textContent.includes("isn\'t answering"))', 6000)
    check(engine, "and stays on the page it was on", page.evaluate("location.pathname") == "/")
    page.unroute(BASE + "/**", down)
    until(page, engine, "the title opens once the server is back",
          f'location.pathname === {href!r} && !!document.querySelector("main .detail h1") && !document.querySelector(".toast:not([data-leaving])")', 8000)
    page.evaluate("history.back()")
    until(page, engine, "and Back still returns home", 'location.pathname === "/" && !document.querySelector("semmel-shell")')
    page.wait_for_timeout(500)


def missing(page, engine):
    page.goto(BASE + "/movies/not-in-the-library", wait_until="commit")
    until(page, engine, "a title that isn't there says so inside the app",
          'document.querySelector("main h1")?.textContent === "Not in the library" && !!document.querySelector("header.top") && !!document.querySelector("main .trouble a[href=\'/\']")')


def run(p, engine, device):
    browser = getattr(p, engine).launch()
    page = browser.new_context(**p.devices[device]).new_page()
    page.set_default_timeout(8000)
    errors = []
    page.on("pageerror", lambda error: errors.append(f"{error} at {page.url} {getattr(error, 'stack', '')}"))
    page.route("**/api/**", forward)
    fake("/fake/forget-subtitles")
    fake("/fake/forget-marks")
    fake("/fake/reset-screen")
    signs_in(page, engine)
    page.wait_for_timeout(2500)
    if not answers(page, engine, "cold load"):
        browser.close()
        return
    check(engine, "a film is playing with a chosen speed",
          page.evaluate('!!document.querySelector(".now-state") && !!document.querySelector(".choices .choice[aria-pressed=true]")'))
    page.wait_for_timeout(2500)
    answers(page, engine, "after the TV paused and resumed")
    scrolls(page, engine, "cold load")
    top_bar(page, engine)
    check(engine, "page chrome is not selectable", page.evaluate('getComputedStyle(document.body).userSelect === "none" || getComputedStyle(document.body).webkitUserSelect === "none"'))
    page.locator("main .chips .chip", has_text="Movies").tap()
    until(page, engine, "a tapped chip switches", '[...document.querySelectorAll("main .chips .chip.on")].some((chip) => chip.textContent.trim() === "Movies")', 3000)
    page.locator("main .chips .chip", has_text="Home").tap()
    until(page, engine, "Home chip switches back", '[...document.querySelectorAll("main .chips .chip.on")].some((chip) => chip.textContent.trim() === "Home")', 3000)
    page.wait_for_selector(TILE)
    page.wait_for_timeout(1000)
    opens_title(page, engine, "cold load")
    finds_subtitles(page, engine)
    marks_watched(page, engine)
    answers(page, engine, "title page")
    scrolls(page, engine, "title page")
    page.evaluate("window.scrollTo(0, document.documentElement.scrollHeight)")
    page.wait_for_timeout(300)
    page.locator("main a.back").first.tap()
    until(page, engine, "Back returns home", 'location.pathname === "/" && !document.querySelector("semmel-shell")')
    page.wait_for_timeout(500)
    answers(page, engine, "after Back")
    page.evaluate("window.scrollTo(0, 300)")
    page.wait_for_timeout(200)
    scrolls_both_ways(page, engine, "after Back")
    scrolls(page, engine, "after Back")
    opens_title(page, engine, "after Back")
    page.evaluate("history.back()")
    until(page, engine, "history back returns home", 'location.pathname === "/" && !document.querySelector("semmel-shell")')
    page.wait_for_timeout(500)
    restart(page, engine)
    page.locator(".bar-open").tap()
    until(page, engine, "remote sheet opens", 'document.documentElement.hasAttribute("data-sheet")', 3000)
    page.wait_for_timeout(800)
    answers(page, engine, "sheet open")
    subtitle_timing(page, engine)
    check(engine, "no Sound choice while no headphones are paired", page.evaluate('!document.querySelector(".sound")'))
    page.locator(".sheet-close").tap()
    until(page, engine, "remote sheet closes", '!document.documentElement.hasAttribute("data-sheet")', 3000)
    page.wait_for_timeout(800)
    answers(page, engine, "after the sheet")
    scrolls(page, engine, "after the sheet")
    toasts(page, engine)
    settings_subpages(page, engine)
    headphones(page, engine)
    switch_answers_at_once(page, engine)
    watches_on_the_phone(page, engine)
    page.locator(".bar-toggle").tap()
    answers(page, engine, "after play/pause")
    opens_title(page, engine, "after play/pause")
    marks_episodes(page, engine)
    next_episode(page, engine)
    missing(page, engine)
    profiles(page, engine)
    check(engine, "no script errors", not errors, "; ".join(errors))
    browser.close()


def signed_in_page(browser, **options):
    page = browser.new_page(**options)
    page.set_default_timeout(8000)
    page.route("**/api/**", forward)
    page.goto(BASE + "/profiles", wait_until="commit")
    sign_in(page, "setup", "Frederik", "frederik-pass", "signing in")
    page.wait_for_function('location.pathname === "/"')
    return page


def desktop(p):
    engine = "desktop"
    browser = p.chromium.launch()
    page = signed_in_page(browser, viewport={"width": 760, "height": 800})
    page.wait_for_selector(".row.posters a[data-topcoat-link]")
    page.wait_for_timeout(1500)
    hidden = page.evaluate("""() => [document.documentElement, ...document.querySelectorAll('.row, .sheet-scroll')]
      .every((node) => getComputedStyle(node).scrollbarWidth === 'none')""")
    check(engine, "no scroll bars on the page, the rows and the sheet", hidden)
    row = page.locator(".row.posters").first
    box = row.bounding_box()
    y = box["y"] + box["height"] / 2
    x = box["x"] + min(box["width"] - 20, 600)
    page.mouse.move(x, y)
    page.mouse.down()
    for step in range(1, 11):
        page.mouse.move(x - 30 * step, y)
    page.mouse.up()
    page.wait_for_timeout(800)
    moved = row.evaluate("(row) => row.scrollLeft")
    check(engine, "dragging a row with the mouse scrolls it", moved > 50, f"scrollLeft {moved}")
    check(engine, "dragging a row opens nothing", page.evaluate("location.pathname") == "/")
    tile = page.locator(".row.posters a[data-topcoat-link]").first
    href = tile.get_attribute("href")
    tile.click()
    until(page, engine, "clicking a title still opens it", f"location.pathname === {href!r}")
    browser.close()


PLAYING = """() => {
  const video = document.querySelector(".pp-video");
  let paused = false;
  Object.defineProperty(video, "paused", { get: () => paused, configurable: true });
  video.play = () => { paused = false; video.dispatchEvent(new Event("playing")); return Promise.resolve(); };
  video.pause = () => { paused = true; video.dispatchEvent(new Event("pause")); };
  window.playerPaused = () => paused;
  video.dispatchEvent(new Event("playing"));
}"""


def laptop_player(p):
    engine = "laptop"
    browser = p.chromium.launch()
    page = signed_in_page(browser, viewport={"width": 1280, "height": 800})
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    stub_hls(page)
    page.goto(BASE + "/movies/the-endless-summer-1966", wait_until="commit")
    until(page, engine, "a laptop gets the laptop player", 'document.documentElement.dataset.device === "laptop" && !!document.querySelector("main button.play[data-play-movie]")')
    page.wait_for_timeout(1000)
    if page.evaluate('document.querySelector("#mode").dataset.mode') == "tv":
        page.click("#mode")
        until(page, engine, "the switch moves to the laptop", 'document.querySelector("#mode").dataset.mode === "phone"', 5000)
        page.wait_for_timeout(800)
        if page.evaluate('!!document.querySelector(".pp")'):
            page.click(".pp-close")
            until(page, engine, "the chevron folds the handed-over film into the bar", '!!document.querySelector(".pp.pp-mini")', 3000)
            page.click(".pp-mini-stop")
            until(page, engine, "and stop closes it", '!document.querySelector(".pp")', 3000)
    page.click("main button.play[data-play-movie]")
    until(page, engine, "Play opens the player on the laptop", '!!document.querySelector(".pp") && !document.querySelector(".pp-loading")', 5000)
    page.wait_for_timeout(500)
    page.evaluate(PLAYING)
    shown = 'document.querySelector(".pp").classList.contains("pp-controls")'
    page.mouse.move(320, 300)
    page.mouse.move(330, 310)
    until(page, engine, "moving the mouse shows the controls", shown, 500)
    page.wait_for_timeout(2200)
    check(engine, "they hide again after a short idle", page.evaluate(f"!{shown}"))
    check(engine, "and so does the cursor", page.evaluate('getComputedStyle(document.querySelector(".pp")).cursor === "none"'))
    page.mouse.move(340, 320)
    until(page, engine, "moving it again brings them back", shown, 500)
    scrub = page.locator(".pp-scrub").bounding_box()
    page.mouse.move(scrub["x"] + scrub["width"] / 3, scrub["y"] + scrub["height"] / 2)
    page.wait_for_timeout(2500)
    check(engine, "they stay up while the pointer is on them", page.evaluate(shown))
    page.mouse.move(320, 300)
    page.wait_for_timeout(2200)
    check(engine, "and hide once it leaves them", page.evaluate(f"!{shown}"))
    plays = len([command for command in fake_get("/fake/screen-log") if command["action"] == "play"])
    page.keyboard.press(" ")
    page.wait_for_timeout(500)
    check(engine, "Space pauses, though the Play button keeps the focus behind the player", page.evaluate("playerPaused()"))
    check(engine, "and does not press that button again",
          len([command for command in fake_get("/fake/screen-log") if command["action"] == "play"]) == plays)
    page.mouse.click(320, 300)
    page.wait_for_timeout(300)
    check(engine, "a click on the picture plays again", page.evaluate("!playerPaused()"))
    fake("/fake/drop-play")
    page.evaluate('document.dispatchEvent(new Event("visibilitychange"))')
    page.wait_for_timeout(1500)
    check(engine, "a phone the TV gave up on while it played keeps playing",
          page.evaluate('!playerPaused() && !document.querySelector(".pp-failed")'))
    fake_until(page, engine, "and its report takes the play back", lambda: (fake_get("/api/screen")["play"] or {}).get("session") is not None)
    page.mouse.move(600, 600)
    page.mouse.move(610, 610)
    page.click(".pp-pill:has-text('Audio')")
    until(page, engine, "Audio & subtitles opens its sheet", '!document.querySelector(".pp-sheet").hidden', 2000)
    check(engine, "the sheet keeps a readable width on a wide screen", page.evaluate('document.querySelector(".pp-sheet").getBoundingClientRect().width <= 640'))
    page.mouse.click(320, 160)
    page.wait_for_timeout(300)
    check(engine, "a click beside the sheet closes it and leaves playback alone",
          page.evaluate('document.querySelector(".pp-sheet").hidden && !playerPaused()'))
    page.keyboard.press("Escape")
    until(page, engine, "Escape folds the player into the bar", '!!document.querySelector(".pp.pp-mini")', 3000)
    check(engine, "and it keeps playing", page.evaluate("!playerPaused()") and fake_get("/api/screen")["play"] is not None)
    page.wait_for_timeout(600)
    bar = page.locator(".pp-minibar").bounding_box()
    x, y = bar["x"] + bar["width"] / 2, bar["y"] + bar["height"] / 2
    page.mouse.move(x, y)
    page.mouse.down()
    for step in range(1, 13):
        page.mouse.move(x, y - 40 * step)
    page.mouse.up()
    until(page, engine, "dragging the bar up opens the player", '!document.querySelector(".pp-mini") && document.documentElement.classList.contains("pp-open")', 3000)
    page.wait_for_timeout(500)
    page.mouse.move(320, 200)
    page.mouse.down()
    for step in range(1, 13):
        page.mouse.move(320, 200 + 40 * step)
    page.mouse.up()
    until(page, engine, "dragging the player down folds it into the bar", '!!document.querySelector(".pp.pp-mini")', 3000)
    check(engine, "without pausing it", page.evaluate("!playerPaused()"))
    page.wait_for_timeout(500)
    page.click(".pp-mini-stop")
    until(page, engine, "the bar's stop button closes the player", '!document.querySelector(".pp")', 3000)
    fake_until(page, engine, "and the TV hears that it stopped", lambda: fake_get("/api/screen")["play"] is None,
               detail=lambda: (fake_get("/api/screen")["play"], fake_get("/fake/progress-log")))
    page.click(".bar-open")
    until(page, engine, "the remote opens on the laptop", 'document.documentElement.hasAttribute("data-sheet")', 3000)
    page.wait_for_timeout(800)
    check(engine, "and is a card, not the full width",
          page.evaluate('(() => { const box = document.querySelector(".sheet").getBoundingClientRect(); return box.width <= 640 && Math.abs(box.left - (innerWidth - box.right)) < 2; })()'))
    check(engine, "no script errors", not errors, "; ".join(errors))
    browser.close()


with sync_playwright() as p:
    run(p, "webkit", "iPhone 15")
    run(p, "chromium", "Pixel 7")
    desktop(p)
    laptop_player(p)
    invite(p)

print("PASS" if not failures else "FAIL: " + ", ".join(failures))
sys.exit(1 if failures else 0)
