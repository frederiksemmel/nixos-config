import contextlib
import json
import traceback
import shlex
import time
import urllib.request
from typing import Any

from PIL import Image, ImageChops
from playwright.sync_api import sync_playwright

PHONE = "http://localhost:8080"
OUT = os.environ["out"]
HLS_URL = "https://cdn.jsdelivr.net/npm/hls.js@1.5.17/dist/hls.min.js"
FILM = "testfilm-2021"
SHOW = "testserie"
SPARE_FILM = "archiv-01-2001"
FRESH_FILM = "kinofilm-2020"
FEEDBACK = "/var/lib/semmeltv/feedback"
SERVICE_TOKEN = "vm-test-service-token"


FAILED: list[str] = []
CLEANUP: list[Any] = []


@contextlib.contextmanager
def scenario(name):
    with subtest(name):
        try:
            yield
        except Exception as error:
            FAILED.append(f"{name}: {error}")
            print(f"SCENARIO FAILED: {name}\n{traceback.format_exc()}")
            for clean in CLEANUP:
                try:
                    clean()
                except Exception:
                    print(f"cleaning up after it failed too:\n{traceback.format_exc()}")


def eventually(what, check, timeout=30.0, every=0.5):
    deadline = time.monotonic() + timeout
    while True:
        value = check()
        if value is not None and value is not False:
            return value
        if time.monotonic() > deadline:
            raise AssertionError(f"{what}: not after {timeout} s, last {value!r}")
        time.sleep(every)


def mpv(*command):
    message = shlex.quote(json.dumps({"command": list(command), "request_id": 7}))
    status, out = tv.execute(f"echo {message} | socat -t 1 - UNIX-CONNECT:/run/user/$(id -u semmeltv)/semmeltv-mpv.sock 2>/dev/null")
    if status != 0:
        return None
    for line in out.splitlines():
        try:
            reply = json.loads(line)
        except ValueError:
            continue
        if reply.get("request_id") == 7:
            return reply.get("data") if reply.get("error") == "success" else None
    return None


def tv_position():
    position = mpv("get_property", "time-pos")
    return float(position) if isinstance(position, (int, float)) else None


def mpv_state():
    names = ("pause", "time-pos", "core-idle", "paused-for-cache", "eof-reached", "current-vo", "current-ao", "hwdec-current", "frame-drop-count", "speed", "seeking")
    return {name: mpv("get_property", name) for name in names}


def tv_plays_past(seconds, timeout=60):
    try:
        eventually(f"the TV plays past {seconds} s", lambda: (tv_position() or 0) > seconds, timeout=timeout)
    except AssertionError as error:
        tv.screenshot("tv-stuck")
        raise AssertionError(f"{error}; mpv: {mpv_state()}")


def tv_seek(phone, to):
    def seeked():
        answer = phone.api("POST", "/api/playback/seek", {"to": to})
        time.sleep(1)
        return (tv_position() or 0) >= to - 1 or answer

    eventually(f"the TV seeks to {to} s", lambda: seeked() is True, timeout=30, every=1)


def tv_shows(text, timeout=60):
    seen = [""]

    def look():
        seen[0] = tv.get_screen_text()
        return text in seen[0]

    try:
        eventually(f"the TV shows {text!r}", look, timeout=timeout, every=2)
    except AssertionError:
        tv.screenshot("tv-missing-text")
        raise AssertionError(f"the TV does not show {text!r}; it shows {seen[0]!r}")


def tv_plays_nothing():
    eventually("the TV's player closes", lambda: mpv("get_property", "pid") is None, timeout=30)


def tv_said(line):
    return int(tv.succeed(f"journalctl -b -o cat | grep -c {shlex.quote(line)} || true").strip())


def tv_episode(phone):
    shown = phone.api("GET", "/api/playback")["body"]["now_playing"]
    return shown and shown.get("kind") == "episode" and shown["episode"]


def invite(name):
    out = tv.succeed(
        f"runuser -u semmeltv -- env SEMMELTV_DB=/var/lib/semmeltv/semmeltv.db SEMMELTV_ORIGINS={PHONE} {SEMMELTV} invite {name}"
    )
    return out.split("/invite/")[1].split()[0]


def session_cookie(name):
    request = urllib.request.Request(f"{PHONE}/api/auth/invite/{invite(name)}", data=b"", method="POST", headers={"Origin": PHONE})
    with urllib.request.urlopen(request) as response:
        for header in response.headers.get_all("Set-Cookie") or []:
            if header.startswith("semmeltv_session="):
                return header.split(";")[0].split("=", 1)[1]
    raise AssertionError(f"redeeming an invite for {name} set no session")


class Phone:
    def __init__(self, playwright, engine="chromium", device="Pixel 7", who="Frederik", viewport=None):
        self.engine = engine
        self.browser = getattr(playwright, engine).launch()
        options: dict[str, Any] = dict(playwright.devices[device]) if device else {"viewport": viewport}
        self.touch = bool(options.get("has_touch"))
        self.context = self.browser.new_context(**options)
        self.context.route(
            HLS_URL,
            lambda route: route.fulfill(path=HLS_JS, content_type="text/javascript", headers={"access-control-allow-origin": "*"}),
        )
        if who:
            self.context.add_cookies([{"name": "semmeltv_session", "value": session_cookie(who), "url": PHONE}])
        self.page = self.context.new_page()
        self.page.set_default_timeout(15000)
        self.errors: list[str] = []
        self.page.on("pageerror", lambda error: self.errors.append(f"{error} at {self.page.url}: {error.stack}"))
        self.shots = 0

    def goto(self, path):
        self.page.goto(PHONE + path, wait_until="commit")

    def until(self, condition, timeout=15000):
        try:
            self.page.wait_for_function(condition, timeout=timeout)
        except Exception:
            raise AssertionError(f"not after {timeout} ms at {self.page.url}: {condition}")

    def js(self, expression):
        return self.page.evaluate(expression)

    def api(self, method, path, body=None):
        return self.page.evaluate(
            """async ([method, path, body]) => {
              const response = await fetch(path, { method, headers: body === null ? {} : { "content-type": "application/json" },
                                                    body: body === null ? undefined : JSON.stringify(body) });
              const text = await response.text();
              let data = null;
              try { data = JSON.parse(text); } catch {}
              return { status: response.status, body: data ?? text };
            }""",
            [method, path, body],
        )

    def shot(self, name):
        self.page.screenshot(path=os.path.join(OUT, f"{self.engine}-{name}.png"))

    def close(self):
        self.browser.close()
        errors = [error for error in self.errors if not (self.engine == "webkit" and "due to access control checks" in error)]
        assert not errors, f"script errors on the {self.engine} phone: {errors}"


VIDEO = 'document.querySelector(".pp-video")'
PLAYS = f'!!{VIDEO} && {VIDEO}.readyState >= 3 && {VIDEO}.currentTime > 0.5 && !{VIDEO}.paused && !document.querySelector(".pp-failed")'
ACTIVE_CUES = f"""() => [...{VIDEO}.textTracks].filter((track) => track.mode === "showing")
  .flatMap((track) => [...(track.activeCues || [])].map((cue) => cue.text))"""


def mode(phone):
    return phone.js('document.querySelector("#mode")?.dataset.mode')


def switch_to(phone, wanted):
    phone.until('!!document.querySelector("#mode") && document.documentElement.hasAttribute("data-phone-ready")')
    shown = screen(phone)["kind"]
    phone.until(f'document.querySelector("#mode").dataset.mode === "{shown}"')
    if shown != wanted:
        switch = phone.page.locator("#mode")
        switch.tap() if phone.touch else switch.click()
    phone.until(f'document.querySelector("#mode").dataset.mode === "{wanted}"')
    eventually(f"the TV agrees the screen is on the {wanted}", lambda: screen(phone)["kind"] == wanted)


def film_page(phone):
    phone.goto(f"/movies/{FILM}")
    phone.until('!!document.querySelector("main button.play[data-play-movie]")')


def controls(phone):
    if not phone.js('document.querySelector(".pp").classList.contains("pp-controls")'):
        phone.page.locator(".pp-layer").tap(position={"x": 30, "y": 300})
    phone.until('document.querySelector(".pp").classList.contains("pp-controls")', 3000)


def close_player(phone):
    if not phone.js('!!document.querySelector(".pp")'):
        return
    if not phone.js('!!document.querySelector(".pp.pp-mini")'):
        phone.js('document.querySelector(".pp-close").click()')
        phone.until('!!document.querySelector(".pp.pp-mini")')
    phone.js('document.querySelector(".pp-mini-stop").click()')
    phone.until('!document.querySelector(".pp")')


def phone_plays_film(phone):
    if phone.js(f"!!{VIDEO} && !{VIDEO}.paused && !document.querySelector('.pp-failed')"):
        return
    film_page(phone)
    switch_to(phone, "phone")
    phone.page.locator("main button.play[data-play-movie]").tap()
    phone.until(PLAYS, 30000)


def phone_time(phone):
    return phone.js('Number(document.querySelector(".pp-scrub").value)')


def screen(phone):
    return phone.api("GET", "/api/screen")["body"]


def stop_everything(phone):
    close_player(phone)
    view = screen(phone)
    if view["kind"] != "tv" and view.get("device"):
        phone.api("POST", "/api/screen", {"action": "stop", "device": view["device"]})
    phone.api("POST", "/api/playback/stop", {})
    tv_plays_nothing()


def tv_starts_at():
    return eventually("the TV plays", tv_position, timeout=60)


def app_restarted():
    tv.wait_for_unit("cage-tty1.service")
    tv.wait_for_open_port(8090)
    tv.wait_for_text("Testfilm", timeout=120)


def plays_the_next_episodes(phone, through_the_end=True):
    opens = []
    phone.page.on("request", lambda request: opens.append(request.post_data_json) if request.url.endswith("/api/stream/open") else None)
    phone.goto(f"/shows/{SHOW}")
    phone.until('document.querySelectorAll("main button.episode[data-play-show]").length === 3')
    switch_to(phone, "phone")
    episode = phone.page.locator('main button.episode[data-episode="1"]')
    episode.scroll_into_view_if_needed()
    episode.tap()
    phone.until('document.querySelector(".pp-title")?.textContent.includes("Anfang")')
    phone.until(PLAYS, 30000)
    controls(phone)
    phone.page.locator('.pp-pill[aria-label="Next episode"]').tap()
    phone.until('document.querySelector(".pp-title")?.textContent.includes("Mitte")')
    phone.until(PLAYS, 30000)
    assert opens[-1]["episode"] == 2 and opens[-1].get("replaces"), f"Next opens episode 2 in place of 1: {opens[-1]}"
    phone.page.wait_for_timeout(8000)
    assert phone.js(PLAYS), "the next episode plays on: the closed stream of the one before doesn't fail it"
    if not through_the_end:
        return
    end = f"{VIDEO}.seekable.end({VIDEO}.seekable.length - 1)"
    phone.until(f"{VIDEO}.seekable.length > 0 && {end} > 178", 90000)
    phone.js(f"{VIDEO}.currentTime = {end} - 1.5")
    phone.until('document.querySelector(".pp-title")?.textContent.includes("Ende")', 30000)
    phone.until(PLAYS, 30000)
    assert opens[-1]["episode"] == 3 and opens[-1].get("replaces"), f"an episode that ends plays the next: {opens[-1]}"
    assert (screen(phone)["play"] or {}).get("episode") == 3
    phone.page.go_back()
    phone.until(f'!!document.querySelector(".pp.pp-mini") && location.pathname === "/shows/{SHOW}" && !document.documentElement.classList.contains("pp-open")', 3000)
    assert "Ende" in phone.js('document.querySelector(".pp-mini-text").textContent')
    streams = len(opens)
    phone.page.locator(".pp-mini-text").tap()
    phone.until('!document.querySelector(".pp-mini") && document.documentElement.classList.contains("pp-open") && !!history.state?.player', 3000)
    phone.page.wait_for_timeout(500)
    phone.js('document.querySelector(".pp-close").click()')
    phone.until('!!document.querySelector(".pp.pp-mini")', 3000)
    phone.page.wait_for_timeout(500)
    playing = phone.page.locator('main button.episode[data-episode="3"]')
    playing.scroll_into_view_if_needed()
    playing.tap()
    phone.until('!document.querySelector(".pp-mini") && document.querySelector(".pp-title").textContent.includes("Ende")', 3000)
    phone.page.wait_for_timeout(800)
    assert len(opens) == streams, "tapping the episode that plays opens its player without streaming it again"
    close_player(phone)
    eventually("the TV hears that the phone stopped", lambda: screen(phone)["play"] is None)
    phone.page.wait_for_timeout(800)
    assert phone.js('!document.querySelector(".pp")')
    assert phone.js('getComputedStyle(document.querySelector(".bar")).visibility !== "hidden" && !document.elementFromPoint(innerWidth / 2, innerHeight - 30).closest(".pp")')
    switch_to(phone, "tv")


TILE = 'main a[data-topcoat-link][href^="/movies/"]'

CENTRED = """() => {
  const controls = [...document.querySelectorAll(".pill, .choice, .chip, .storage-button, .top-link, .back, .key")]
    .filter((control) => control.getClientRects().length && getComputedStyle(control).visibility !== "hidden");
  const off = [];
  for (const control of controls) {
    const box = control.getBoundingClientRect();
    if (box.height < 20) continue;
    const style = getComputedStyle(control);
    const top = box.top + parseFloat(style.borderTopWidth) + parseFloat(style.paddingTop);
    const bottom = box.bottom - parseFloat(style.borderBottomWidth) - parseFloat(style.paddingBottom);
    let low = Infinity;
    let high = -Infinity;
    const take = (rect) => {
      if (!rect.height) return;
      low = Math.min(low, rect.top);
      high = Math.max(high, rect.bottom);
    };
    const walker = document.createTreeWalker(control, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
    const texts = [];
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (node.nodeType === Node.TEXT_NODE) {
        if (node.textContent.trim()) texts.push(node);
      } else if (node.matches("svg, img") && getComputedStyle(node).display !== "none") {
        take(node.getBoundingClientRect());
      }
    }
    for (const text of texts) {
      const wrap = document.createElement("span");
      text.replaceWith(wrap);
      wrap.append(text);
      take(wrap.getBoundingClientRect());
      wrap.replaceWith(text);
    }
    if (low === Infinity) continue;
    const gap = (low + high) / 2 - (top + bottom) / 2;
    if (Math.abs(gap) > 2) off.push(`${control.tagName.toLowerCase()}.${control.className} "${control.textContent.trim().slice(0, 24)}" ${gap.toFixed(1)} px`);
  }
  return off;
}"""

TOP_BAR = """() => {
  const box = (selector) => document.querySelector(selector).getBoundingClientRect();
  const top = box("header.top"), brand = box(".brand"), talk = box(".microphone-talk"), settings = [...document.querySelectorAll("header.top .top-link")].pop().getBoundingClientRect();
  const border = parseFloat(getComputedStyle(document.querySelector("header.top")).borderBottomWidth);
  return { above: talk.top - top.top, below: top.bottom - border - talk.bottom, left: brand.left, right: innerWidth - settings.right,
           brand: brand.top + brand.height / 2 - (talk.top + talk.height / 2) };
}"""

FILTER_ROW = """() => {
  const chips = [...document.querySelectorAll("main .chips .chip")].filter((chip) => chip.getClientRects().length);
  const heights = chips.map((chip) => chip.getBoundingClientRect().height);
  const middles = (chip) => [...chip.querySelectorAll("svg")].map((svg) => { const box = svg.getBoundingClientRect(); return box.top + box.height / 2; });
  const stacked = chips.filter((chip) => chip.querySelector("svg")).map((chip) => { const m = middles(chip); return Math.max(...m) - Math.min(...m); });
  return { heights: Math.max(...heights) - Math.min(...heights), stacked: Math.max(0, ...stacked), chips: chips.length };
}"""

AWAY = """(away) => {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => away ? "hidden" : "visible" });
  document.dispatchEvent(new Event("visibilitychange"));
}"""


def answers(phone, where):
    try:
        phone.page.wait_for_function("new Promise((done) => requestAnimationFrame(() => done(true)))", timeout=5000)
    except Exception:
        raise AssertionError(f"the page stopped answering: {where}")


def centred(phone, where):
    if phone.engine == "webkit":
        return
    off = phone.js(CENTRED)
    assert not off, f"{where}: labels off centre: {off[:6]}"


def scrolls(phone, where):
    phone.js("window.scrollTo(0, 0)")
    phone.page.wait_for_timeout(200)
    if phone.engine == "webkit":
        phone.page.keyboard.press("PageDown")
    else:
        phone.page.mouse.move(200, 400)
        phone.page.mouse.wheel(0, 600)
    try:
        phone.page.wait_for_function("scrollY > 100", timeout=3000)
    except Exception:
        raise AssertionError(f"{where}: the page does not scroll, scrollY {phone.js('scrollY')}, height {phone.js('document.documentElement.scrollHeight')}")
    phone.js("window.scrollTo(0, 300)")
    phone.page.wait_for_timeout(300)
    reached = [phone.js("scrollY")]
    for key, dy in (("PageUp", -600), ("PageDown", 600)):
        if phone.engine == "webkit":
            phone.page.keyboard.press(key)
        else:
            phone.page.mouse.move(200, 400)
            phone.page.mouse.wheel(0, dy)
        phone.page.wait_for_timeout(700)
        reached.append(phone.js("scrollY"))
    assert reached[1] < reached[0] - 100 and reached[2] > reached[1] + 100, f"{where}: scrolls up and down {reached}"
    phone.js("window.scrollTo(0, 0)")
    phone.page.wait_for_timeout(200)


LOADED = 'document.readyState === "complete"'
SHEET_STILL = '[...document.querySelectorAll("#sheet, .sheet-dim")].every((element) => element.getAnimations().length === 0)'


def at_home(phone):
    phone.until(f'location.pathname === "/" && !document.querySelector("semmel-shell") && !!document.querySelector("main .shelf") && {LOADED}')
    phone.page.wait_for_timeout(500)


def opens_title(phone):
    tile = phone.page.locator(TILE).first
    href = tile.get_attribute("href")
    tile.scroll_into_view_if_needed()
    tile.tap()
    phone.until(f'location.pathname === {href!r} && !document.querySelector("semmel-shell") && !!document.querySelector("main .detail h1")')
    phone.page.wait_for_timeout(500)
    return href


def tv_plays_film(phone):
    phone.api("POST", "/api/play", {"movie": FILM})
    tv_starts_at()
    phone.until('!!document.querySelector(".now-state")', 30000)


def goes_away(phone, seconds):
    phone.page.evaluate(
        """async (ms) => {
          const show = (state) => {
            Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state });
            Object.defineProperty(document, "hidden", { configurable: true, get: () => state === "hidden" });
            document.dispatchEvent(new Event("visibilitychange"));
          };
          show("hidden");
          await new Promise((done) => setTimeout(done, ms));
          show("visible");
        }""",
        seconds * 1000,
    )


DEPLOYED = "/run/systemd/system/semmeltv-web.service.d/build.conf"


def deploy_web(phone, build):
    if build:
        tv.succeed(f"mkdir -p $(dirname {DEPLOYED}) && printf '[Service]\\nEnvironment=SEMMELTV_WEB_BUILD={build}\\n' > {DEPLOYED}")
    else:
        tv.succeed(f"rm -f {DEPLOYED}")
    tv.succeed("systemctl daemon-reload && systemctl restart semmeltv-web.service")
    if build:

        def serves():
            try:
                return phone.api("GET", "/build")["body"] == build
            except Exception:
                return False

        eventually(f"the web app serves build {build}", serves)


def raise_toast(phone, text):
    phone.page.evaluate("(text) => window.dispatchEvent(new CustomEvent('semmeltv:toast', { detail: { text, tone: 'info' } }))", text)



WATCHING = '[...document.querySelectorAll("main .shelf h2")].some((heading) => heading.textContent.includes("Currently Watching"))'
AVATAR = 'document.querySelector(".profile-link .avatar")?.textContent.trim()'


def password_sign_in(someone, name, password):
    someone.until('location.pathname === "/profiles" && !!document.querySelector(".profile-choice")')
    someone.page.locator(".profile-choice", has_text=name).click()
    someone.until('!document.querySelector("#password-form").hidden', 5000)
    someone.page.fill("#password-input", password)
    someone.page.locator("#password-form button[type=submit]").click()


start_all()

with subtest("the TV boots into the app and shows the library"):
    tv.wait_for_unit("cage-tty1.service")
    tv.wait_for_open_port(8090)
    tv.wait_for_text("Testfilm", timeout=180)
    tv.wait_for_unit("semmeltv-web.service")
    tv.wait_for_open_port(8091)
    tv.wait_for_open_port(8080)
    tv.forward_port(8080, 8080)
    tv.screenshot("tv-home")

with sync_playwright() as playwright:
    phone = Phone(playwright)
    CLEANUP.append(lambda: phone.shot(f"failed-{len(FAILED)}"))
    CLEANUP.append(lambda: stop_everything(phone))
    CLEANUP.append(lambda: phone.goto("/"))

    with scenario("an invite link signs a phone in, and Back never returns to it"):
        guest = Phone(playwright, who=None)
        guest.goto("/invite/made-up")
        guest.until('(document.querySelector("main")?.textContent || "").includes("not valid")')
        token = invite("Frederik")
        guest.goto(f"/invite/{token}")
        guest.until('!!document.querySelector("#redeem") && document.querySelector("main h1").textContent.includes("Frederik")')
        guest.page.locator("#redeem").tap()
        guest.until('location.pathname === "/account" && !!document.querySelector("#add-passkey") && !!document.querySelector("#password-set")')
        guest.page.go_back(wait_until="commit")
        guest.until('location.pathname === "/" && !!document.querySelector(".profile-link")')
        assert guest.api("GET", "/api/auth/me")["body"]["person"]["name"] == "Frederik"
        guest.goto(f"/invite/{token}")
        guest.until('location.pathname === "/" && !!document.querySelector(".profile-link")')
        guest.shot("signed-in")
        guest.close()

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

    with scenario("a minute of a film played on the TV puts it in Continue Watching on the TV and the phone"):
        phone.api("POST", "/api/play", {"movie": FILM})
        started = tv_starts_at()
        eventually(
            "the TV plays at double speed",
            lambda: phone.api("POST", "/api/playback/speed", {"speed": 2}) and mpv("get_property", "speed") == 2,
            every=1,
        )
        tv_plays_past(started + 64, timeout=90)
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

    with scenario("an episode played again after it finished starts over, then resumes where it stopped"):
        episode = {"show": SHOW, "season": 1, "episode": 1}
        phone.api("POST", "/api/play", episode)
        tv_starts_at()
        counted = tv_said("^up next: /")
        tv_seek(phone, 170)
        tv_plays_nothing()
        eventually("the TV counts down to episode 2", lambda: tv_said("^up next: /") > counted)
        tv.send_key("esc")
        eventually("Back cancels the countdown", lambda: tv_said("^up next: cancelled") > 0, timeout=5)
        time.sleep(12)
        assert mpv("get_property", "pid") is None, "after Back, episode 2 does not play by itself"
        eventually(
            "the episode counts as watched",
            lambda: phone.api("GET", f"/api/shows/{SHOW}")["body"]["seasons"][0]["episodes"][0]["watched"],
        )
        phone.api("POST", "/api/play", episode)
        again = tv_starts_at()
        assert again < 5, f"a finished episode started at {again}"
        tv_seek(phone, 60)
        tv_plays_past(62)
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

    with scenario("an episode that ends on the TV counts down to the next, OK plays it at once, and nothing follows the last"):
        counted = tv_said("^up next: /")
        phone.api("POST", "/api/play", {"show": SHOW, "season": 1, "episode": 1})
        tv_starts_at()
        tv_seek(phone, 170)
        eventually("the TV counts down to episode 2", lambda: tv_said("^up next: /") > counted, timeout=60, every=0.3)
        counting = time.monotonic()
        tv.send_key("ret")
        eventually("OK opens episode 2", lambda: mpv("get_property", "pid") is not None, timeout=30, every=0.3)
        assert time.monotonic() - counting < 7, "OK plays the next episode without waiting for the countdown"
        eventually("the TV plays episode 2", lambda: tv_episode(phone) == 2)
        counted = tv_said("^up next: /")
        tv_starts_at()
        tv_seek(phone, 170)
        eventually("the TV counts down to episode 3", lambda: tv_said("^up next: /") > counted, timeout=60, every=0.3)
        counting = time.monotonic()
        shown = tv.get_screen_text()
        tv.screenshot("tv-up-next")
        assert "Up next" in shown and "Plays in" in shown, f"the TV shows the Up next card: {shown!r}"
        eventually("the countdown plays episode 3", lambda: mpv("get_property", "pid") is not None, timeout=30, every=0.3)
        assert time.monotonic() - counting > 7, "episode 3 waits for the countdown"
        eventually("the TV plays episode 3", lambda: tv_episode(phone) == 3)
        counted = tv_said("^up next: /")
        tv_starts_at()
        tv_seek(phone, 170)
        tv_plays_nothing()
        time.sleep(15)
        assert tv_said("^up next: /") == counted, "nothing counts down after the last episode"
        assert mpv("get_property", "pid") is None, "nothing plays after the last episode"

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

    with scenario("a phone stream plays at its pace with subtitles at the right time"):
        phone.goto(f"/movies/{FRESH_FILM}")
        phone.until('!!document.querySelector("main button.play[data-play-movie]")')
        switch_to(phone, "phone")
        phone.page.locator("main button.play[data-play-movie]").tap()
        phone.until(PLAYS, 30000)
        began = time.monotonic()
        assert phone_time(phone) < 5, f"a film never played starts at {phone_time(phone)}"
        controls(phone)
        phone.page.locator(".pp-pill", has_text="Audio").tap()
        phone.until('!document.querySelector(".pp-sheet").hidden')
        phone.page.locator(".pp-sheet section").nth(1).locator(".pp-choice", has_text="English").tap()
        phone.until(f"({ACTIVE_CUES})().length > 0", int(max(10 - (time.monotonic() - began), 1) * 1000))
        print(f"the first subtitle showed {time.monotonic() - began:.1f} s after playback began")
        checked = 0
        first = (time.monotonic(), phone_time(phone))
        while time.monotonic() - first[0] < 12:
            at = phone_time(phone)
            cues = phone.js(ACTIVE_CUES)
            if 0.3 < at % 5 < 3.7:
                assert cues == [f"Kino line {int(at // 5) + 1}"], f"at {at:.2f} s the subtitles show {cues}"
                checked += 1
            time.sleep(0.4)
        last = (time.monotonic(), phone_time(phone))
        pace = (last[1] - first[1]) / (last[0] - first[0])
        print(f"{checked} subtitle checks, the video ran at {pace:.2f}x")
        assert checked >= 10 and 0.9 < pace < 1.1, f"{checked} checks, pace {pace}"
        phone.shot("subtitles")

    with scenario("a film moves from the phone to the TV and back, keeping its place"):
        phone_plays_film(phone)
        at = phone_time(phone)
        controls(phone)
        phone.page.locator(".pp-tv").tap()
        phone.until('!document.querySelector(".pp") && document.querySelector("#mode").dataset.mode === "tv"')
        on_tv = tv_starts_at()
        assert abs(on_tv - at) < 5, f"the phone was at {at}, the TV starts at {on_tv}"
        tv_plays_past(on_tv + 3)
        tv_at = tv_position() or 0
        phone.page.locator("#mode").tap()
        phone.until(PLAYS, 30000)
        on_phone = phone_time(phone)
        assert abs(on_phone - tv_at) < 6, f"the TV was at {tv_at}, the phone starts at {on_phone}"
        tv_plays_nothing()
        assert screen(phone)["kind"] == "phone"

    with scenario("a phone reopens its stream where it was after the TV app restarts"):
        phone_plays_film(phone)
        opens: list[Any] = []
        phone.page.on("request", lambda request: opens.append(request.post_data_json) if request.url.endswith("/api/stream/open") else None)
        boot = screen(phone)["boot"]
        at = phone_time(phone)
        tv.systemctl("restart cage-tty1.service")
        app_restarted()
        eventually("the phone opens its stream again", lambda: phone.page.wait_for_timeout(200) or opens, timeout=60)
        assert abs(opens[-1]["start"] - at) < 15, f"the phone was at {at}, reopened at {opens[-1]['start']}"
        view = eventually(
            "the TV knows the phone plays again",
            lambda: (lambda view: view if view["boot"] != boot and view["kind"] == "phone" and (view["play"] or {}).get("state") == "playing" else None)(screen(phone)),
            timeout=60,
        )
        phone.until(PLAYS)
        assert phone_time(phone) > at
        print(f"phone was at {at:.1f}, plays on at {view['play']['position']}")
        stop_everything(phone)

    with scenario("a laptop that goes quiet for 45 s loses the screen, keeps its place, and its next report takes it back"):
        laptop = Phone(playwright, device=None, viewport={"width": 1280, "height": 800})
        reports: list[Any] = []
        laptop.page.on("request", lambda request: reports.append(request.post_data_json) if request.url.endswith("/progress") else None)
        laptop.goto(f"/movies/{FRESH_FILM}")
        laptop.until('!!document.querySelector("main button.play[data-play-movie]")')
        switch_to(laptop, "phone")
        laptop.page.click("main button.play[data-play-movie]")
        laptop.until(PLAYS, 30000)
        eventually("the TV knows the laptop plays", lambda: (screen(phone)["play"] or {}).get("state") == "playing")
        phone.goto("/")
        phone.until('document.querySelector(".bar-text strong")?.textContent === "Kinofilm"')
        eventually("the laptop has reported where it is", lambda: phone.page.wait_for_timeout(200) or [body for body in reports if (body or {}).get("state") == "playing"], timeout=20)
        laptop.context.set_offline(True)
        cut = time.monotonic()
        last_report = [body for body in reports if body][-1]
        while time.monotonic() - cut < 30:
            view = screen(phone)
            assert view["kind"] == "phone" and (view["play"] or {}).get("state") == "playing", f"{time.monotonic() - cut:.0f} s after the laptop went quiet the screen is {view}"
            assert mpv("get_property", "pid") is None, "the TV doesn't take over"
            time.sleep(2)
        view = eventually(
            "the TV gives up on the quiet laptop",
            lambda: (lambda view: view if view["play"] is None else None)(screen(phone)),
            timeout=30,
            every=1,
        )
        given_up = time.monotonic() - cut
        print(f"the TV gave up {given_up:.0f} s after the laptop went quiet")
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

    for engine, device in (("chromium", "Pixel 7"), ("webkit", "iPhone 15")):
        with scenario(f"feedback from a phone lands on the TV with its screenshot and one of the TV ({engine})"):
            sender = phone if engine == "chromium" else Phone(playwright, engine, device)
            said = f"The list jumped back to the top ({engine})"
            sender.goto("/")
            sender.until('!!document.querySelector("#feedback-open")')
            sender.page.locator("#feedback-open").tap()
            sender.until('document.activeElement === document.querySelector(".feedback-panel textarea")')
            assert sender.js('parseFloat(getComputedStyle(document.querySelector(".feedback-panel textarea")).fontSize) >= 16'), "the box is 16 px, so iOS doesn't zoom"
            sender.page.locator(".feedback-panel textarea").fill(said)
            sender.page.locator(".feedback-attach input[type=file]").set_input_files(os.path.join(OUT, "tv-home.png"))
            sender.until('!document.querySelector(".feedback-shot").hidden && document.querySelector(".feedback-shot img").naturalWidth > 0')
            sender.page.locator(".feedback-send").tap()
            sender.until('document.querySelector(".feedback").hidden')
            report = eventually(
                "the report lands on the TV",
                lambda: (lambda last: last if last["message"] == said else None)(json.loads(tv.succeed(f"tail -n 1 {FEEDBACK}/index.jsonl"))),
            )
            assert report["person"]["name"] == "Frederik", report
            for picture in ("screenshot", "tv_screenshot"):
                assert report[picture], report
                tv.succeed(f"test $(stat -c %s {FEEDBACK}/{report[picture]}) -gt 5000")
            tv.copy_from_machine(f"{FEEDBACK}/{report['tv_screenshot']}", "")
            client = json.loads(tv.succeed(f"cat {FEEDBACK}/{report['id']}.json"))["client"]
            assert {"fetch", "tap"} <= {entry["kind"] for entry in client["trail"]}, client["trail"][-5:]
            if engine == "webkit":
                safari = sender.js('(navigator.userAgent.match(/Version\\/([\\d.]+)/) || [])[1]')
                assert client["agent"] == f"iOS {safari}, Safari tab", client["agent"]
            phone.goto("/feedback")
            phone.until(
                f"""[...document.querySelectorAll(".report")].some((report) =>
                      report.querySelector(".report-message")?.textContent === {said!r}
                      && [...report.querySelectorAll(".report-pictures img")].filter((img) => img.complete && img.naturalWidth > 0).length === 2)"""
            )
            if sender is not phone:
                sender.close()

    with scenario("an admin and the service token set a report's status, and the page filters by it"):
        reports = [json.loads(line) for line in tv.succeed(f"tail -n 2 {FEEDBACK}/index.jsonl").splitlines()]
        by_hand, by_token = reports[-1]["id"], reports[-2]["id"]
        phone.goto("/feedback")
        row = phone.page.locator(".report").filter(has=phone.page.locator(".report-message", has_text=reports[-1]["message"]))
        assert row.locator(".report-status b").inner_text() == "New"
        row.get_by_role("button", name="Set status").tap()
        row.get_by_role("button", name="Needs review").tap()
        row.get_by_placeholder("Why it needs a person").fill("It is a design question")
        row.get_by_placeholder("What they have to decide").fill("Keep or drop the jump")
        row.get_by_role("button", name="Save").tap()
        phone.until(f"""[...document.querySelectorAll(".report")].some((report) =>
            report.querySelector(".report-copy").textContent.includes({by_hand!r})
            && report.querySelector(".report-status b").textContent === "Needs review")""")
        marked = json.loads(tv.succeed(f"cat {FEEDBACK}/{by_hand}.status.json"))
        assert marked["status"] == {"state": "needs_review", "why": "It is a design question", "decide": "Keep or drop the jump"}, marked
        assert marked["by"] == "Frederik", marked
        done = {"state": "done", "resolution": {"kind": "fixed", "commit": "abc1234", "what": "The list keeps its place"}}
        tv.succeed(f"printf '%s: %s %s\\n' Authorization Bearer {SERVICE_TOKEN} > /tmp/service-header")
        tv.succeed(
            "curl -sf -H @/tmp/service-header -H 'content-type: application/json' "
            f"-d '{json.dumps(done)}' http://127.0.0.1:8090/api/feedback/{by_token}/status"
        )
        new = json.loads(tv.succeed("curl -sf -H @/tmp/service-header 'http://127.0.0.1:8090/api/feedback?state=new'"))
        assert by_hand not in [report["id"] for report in new["reports"]] and by_token not in [report["id"] for report in new["reports"]], new
        phone.goto("/feedback")
        phone.page.locator(".report-filters").get_by_role("button", name="Done").tap()
        phone.until(f"""[...document.querySelectorAll(".report")].filter((report) => !report.hidden).every((report) =>
            report.querySelector(".report-status b").textContent.startsWith("Fixed") || report.querySelector(".report-status b").textContent.startsWith("Done"))
            && [...document.querySelectorAll(".report")].some((report) => !report.hidden && report.querySelector(".report-copy").textContent.includes({by_token!r}))""")

    with scenario("Claude answers in a report's thread, and a person's comment reopens it for triage"):
        settled = json.loads(tv.succeed(f"tail -n 2 {FEEDBACK}/index.jsonl").splitlines()[0])
        tv.succeed(
            "curl -sf -H @/tmp/service-header -H 'content-type: application/json' "
            f"""-d '{{"text": "Fixed in abc1234; the list keeps its place."}}' http://127.0.0.1:8090/api/feedback/{settled['id']}/comments"""
        )
        phone.goto("/feedback")
        row = phone.page.locator(".report").filter(has=phone.page.locator(".report-message", has_text=settled["message"]))
        assert row.locator(".report-status b").inner_text().startswith("Fixed"), "Claude's comment leaves the status alone"
        assert row.locator(".comment.claude .comment-who b").inner_text() == "Claude"
        field = row.get_by_role("textbox", name="Comment")
        assert phone.js('parseFloat(getComputedStyle(document.querySelector(".report-say textarea")).fontSize) >= 16'), "16 px, so iOS doesn't zoom"
        field.fill("Still jumps on the second visit")
        row.get_by_role("button", name="Comment").tap()
        phone.until(f"""[...document.querySelectorAll(".report")].some((report) =>
            report.querySelector(".report-copy").textContent.includes({settled['id']!r})
            && report.querySelector(".report-status b").textContent === "Reopened"
            && report.querySelector(".comment.person:not([hidden]) .comment-text")?.textContent === "Still jumps on the second visit")""")
        reopened = json.loads(tv.succeed("curl -sf -H @/tmp/service-header 'http://127.0.0.1:8090/api/feedback?state=reopened'"))["reports"]
        assert [report["id"] for report in reopened] == [settled["id"]], reopened
        assert reopened[0]["status"]["was"]["resolution"]["commit"] == "abc1234", reopened[0]["status"]
        assert [(comment["author"]["kind"], comment["text"]) for comment in reopened[0]["comments"]] == [
            ("claude", "Fixed in abc1234; the list keeps its place."),
            ("person", "Still jumps on the second visit"),
        ], reopened[0]["comments"]
        phone.goto("/feedback")
        phone.until(f"""[...document.querySelectorAll(".report")].some((report) =>
            report.querySelector(".report-copy").textContent.includes({settled['id']!r})
            && [...report.querySelectorAll(".comment:not([hidden]) .comment-who b")].map((name) => name.textContent).join() === "Claude,Frederik")""")

    with scenario("a comment typed but not sent survives a reload, and is forgotten once sent"):
        draft = f'textarea[data-draft="{settled["id"]}:comment"]'
        phone.page.locator(draft).fill("Half a thought before the TV restarts")
        phone.goto("/feedback")
        phone.until(f"document.querySelector({draft!r})?.value === 'Half a thought before the TV restarts'")
        row = phone.page.locator(".report").filter(has=phone.page.locator(".report-message", has_text=settled["message"]))
        row.get_by_role("button", name="Comment").tap()
        phone.until(f"""[...document.querySelectorAll(".comment.person:not([hidden]) .comment-text")].some((text) => text.textContent === "Half a thought before the TV restarts")
            && document.querySelector({draft!r}).value === \"\"""")
        phone.page.wait_for_timeout(2500)
        phone.goto("/feedback")
        phone.page.wait_for_timeout(1000)
        assert phone.js(f"document.querySelector({draft!r}).value") == "", "a sent comment is no draft"
        assert phone.js(f"localStorage.getItem('semmeltv.draft.{settled['id']}:comment')") is None

    for engine, device in (("chromium", "Pixel 7"), ("webkit", "iPhone 15")):
        with scenario(f"the remote opens, closes by a swipe without a reload, and a reload brings it back closed ({engine})"):
            remote = phone if engine == "chromium" else Phone(playwright, engine, device)
            remote.goto("/")
            remote.until(f'!!document.querySelector(".bar-open") && {LOADED}')
            remote.page.locator(".bar-open").tap()
            remote.until('document.documentElement.hasAttribute("data-sheet")', 3000)
            remote.until(SHEET_STILL, 3000)
            remote.page.locator(".sheet-close").tap()
            remote.until('!document.documentElement.hasAttribute("data-sheet")', 3000)
            remote.until(SHEET_STILL, 3000)
            remote.page.locator(".bar-open").tap()
            remote.until('document.documentElement.hasAttribute("data-sheet")', 3000)
            remote.page.reload(wait_until="commit")
            remote.until(f'!!document.querySelector(".bar-open") && {LOADED} && !document.documentElement.hasAttribute("data-sheet")')
            remote.page.locator(".bar-open").tap()
            remote.until('document.documentElement.hasAttribute("data-sheet")', 3000)
            remote.until(SHEET_STILL, 3000)
            remote.js("window.stillHere = true")
            grip = remote.page.locator(".sheet-top").bounding_box()
            assert grip
            x, y = grip["x"] + grip["width"] / 2, grip["y"] + grip["height"] / 2
            remote.page.mouse.move(x, y)
            remote.page.mouse.down()
            for step in range(1, 13):
                remote.page.mouse.move(x, y + 40 * step)
            remote.page.mouse.up()
            remote.until('!document.documentElement.hasAttribute("data-sheet") && window.stillHere === true', 3000)
            if remote is not phone:
                remote.close()

    for engine, device in (("chromium", "Pixel 7"), ("webkit", "iPhone 15")):
        with scenario(f"a tap on the bar while the page still loads opens the remote, and it stays open once loading ends ({engine})"):
            early = Phone(playwright, engine, device)
            held: list[Any] = []
            holding = [True]

            def hold(route):
                kind = route.request.resource_type
                if holding[0] and (kind == "image" or (kind == "script" and "/player-" in route.request.url)):
                    held.append(route)
                else:
                    route.fallback()

            early.page.route("**/*", hold)
            early.goto("/")
            early.until('!!document.querySelector(".bar-open")')
            early.page.wait_for_timeout(300)
            assert not early.js("window.semmeltvSheetReady"), "the remote's script is still held back"
            early.page.locator(".bar-open").tap()
            early.page.wait_for_timeout(500)
            holding[0] = False
            for route in [route for route in held if route.request.resource_type == "script"]:
                route.continue_()
            early.until('document.documentElement.hasAttribute("data-sheet")', 5000)
            assert early.js('document.readyState !== "complete"'), "the images still hold the page's load"
            for route in [route for route in held if route.request.resource_type == "image"]:
                route.continue_()
            early.until('document.readyState === "complete"', 15000)
            early.page.wait_for_timeout(800)
            assert early.js('document.documentElement.hasAttribute("data-sheet")'), "the end of loading leaves the remote open"
            early.close()

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

    for engine, device in (("chromium", "Galaxy S5"), ("webkit", "iPhone SE")):
        with scenario(f"the phone's pages answer, scroll and keep their labels centred while the TV plays ({engine})"):
            browse = Phone(playwright, engine, device)
            tv_plays_film(phone)
            browse.goto("/")
            at_home(browse)
            answers(browse, "cold load while the TV plays")
            assert browse.js('!!document.querySelector(".choices .choice[aria-pressed=true]")'), "the remote shows the chosen speed"
            scrolls(browse, "Home")
            bar = browse.js(TOP_BAR)
            assert abs(bar["above"] - bar["below"]) < 1 and abs(bar["brand"]) < 1, f"the top bar's buttons sit in its middle: {bar}"
            assert abs(bar["left"] - bar["right"]) < 1, f"the top bar's buttons line up with its edges: {bar}"
            assert browse.js('getComputedStyle(document.body).userSelect === "none" || getComputedStyle(document.body).webkitUserSelect === "none"')
            browse.page.locator("main .chips .chip", has_text="Movies").tap()
            browse.until('[...document.querySelectorAll("main .chips .chip.on")].some((chip) => chip.textContent.trim() === "Movies")', 3000)
            filters = browse.js(FILTER_ROW)
            assert filters["heights"] <= 1 and filters["stacked"] <= 1, f"the filter chips are one slim row, icons beside each other: {filters}"
            browse.page.locator("main .chips .chip", has_text="Home").tap()
            browse.until('[...document.querySelectorAll("main .chips .chip.on")].some((chip) => chip.textContent.trim() === "Home")', 3000)
            browse.page.wait_for_selector(TILE)
            browse.page.wait_for_timeout(800)
            centred(browse, "Home")
            opens_title(browse)
            centred(browse, "a title page")
            answers(browse, "a title page")
            browse.js("window.scrollTo(0, document.documentElement.scrollHeight)")
            browse.page.wait_for_timeout(300)
            browse.page.locator("main a.back").first.tap()
            at_home(browse)
            answers(browse, "after Back")
            scrolls(browse, "after Back")
            opens_title(browse)
            browse.js("history.back()")
            at_home(browse)
            scrolls(browse, "after history.back()")
            browse.close()
            stop_everything(phone)

    for engine, device in (("chromium", "Pixel 7"), ("webkit", "iPhone 15")):
        with scenario(f"toasts stack and go away by a swipe, a tap or by themselves ({engine})"):
            toaster = phone if engine == "chromium" else Phone(playwright, engine, device)
            toaster.goto("/")
            at_home(toaster)
            toaster.until('document.documentElement.hasAttribute("data-phone-ready")')
            assert toaster.js('!document.getElementById("microphone-note")'), "the top bar has no microphone text"
            if engine == "chromium":
                toaster.page.locator("#microphone-talk").tap()
                toaster.until('!!document.querySelector(".toast[data-tone=error]") && document.getElementById("microphone").dataset.state === "idle"', 8000)
            toaster.js('document.querySelectorAll(".toast").forEach((toast) => toast.remove())')
            raise_toast(toaster, "first")
            raise_toast(toaster, "second")
            assert toaster.js('["backdropFilter", "webkitBackdropFilter"].every((name) => [undefined, "none"].includes(getComputedStyle(document.querySelector(".toast"))[name]))')
            toaster.until('[...document.querySelectorAll(".toast")].map((t) => t.textContent).join() === "second,first"', 2000)
            newest = toaster.page.locator(".toast").first.bounding_box()
            assert newest
            toaster.page.mouse.move(newest["x"] + 60, newest["y"] + 20)
            toaster.page.mouse.down()
            for step in range(1, 9):
                toaster.page.mouse.move(newest["x"] + 60 + 25 * step, newest["y"] + 20)
            toaster.page.mouse.up()
            toaster.until('[...document.querySelectorAll(".toast:not([data-leaving])")].map((t) => t.textContent).join() === "first"', 2000)
            toaster.page.locator(".toast:not([data-leaving])").first.click()
            toaster.until('!document.querySelector(".toast:not([data-leaving])")', 2000)
            raise_toast(toaster, "goes by itself")
            toaster.until('!document.querySelector(".toast")', 7000)
            if toaster is not phone:
                toaster.close()

    with scenario("a title tapped while the web app restarts opens once it is back"):
        phone.goto("/")
        at_home(phone)
        tv.systemctl("stop semmeltv-web.service")
        tile = phone.page.locator(TILE).first
        href = tile.get_attribute("href")
        tile.tap()
        phone.until('[...document.querySelectorAll(".toast")].some((toast) => toast.textContent.includes("isn\'t answering"))', 10000)
        assert phone.js("location.pathname") == "/"
        tv.systemctl("start semmeltv-web.service")
        phone.until(f'location.pathname === {href!r} && !!document.querySelector("main .detail h1") && !document.querySelector(".toast:not([data-leaving])")', 30000)
        phone.js("history.back()")
        at_home(phone)

    with scenario("pages are revalidated, and a page from before a deploy reloads into the new build"):
        iphone = Phone(playwright, "webkit", "iPhone 15")
        try:
            iphone.goto("/")
            at_home(iphone)
            page = iphone.page.request.get(PHONE + "/")
            assert page.headers.get("cache-control") == "no-cache", page.headers
            script = iphone.js('document.querySelector("script[src*=\'/_topcoat/assets/live-\']").src')
            asset = iphone.page.request.get(script)
            assert "immutable" in asset.headers.get("cache-control", ""), asset.headers
            old_build = iphone.js("document.documentElement.dataset.build")
            iphone.js("window.oldPage = true")
            deploy_web(iphone, "next")
            goes_away(iphone, 5)
            iphone.until('document.documentElement.dataset.build === "next" && !window.oldPage && !!document.querySelector(".bar-open")')
            assert old_build != "next", old_build
            iphone.page.wait_for_timeout(2000)
            iphone.js("window.oldPage = true")
            film_page(iphone)
            deploy_web(iphone, "after-next")
            iphone.page.go_back()
            iphone.until('location.pathname === "/" && document.documentElement.dataset.build === "after-next" && !window.oldPage && !!document.querySelector(".bar-open")')
            iphone.js("window.settled = true")
            iphone.page.wait_for_timeout(3000)
            assert iphone.js("window.settled === true"), "a page of the serving build doesn't reload again"
        finally:
            deploy_web(iphone, None)
            iphone.close()

    with scenario("a title that isn't there says so inside the app"):
        phone.goto("/movies/not-in-the-library")
        phone.until('document.querySelector("main h1")?.textContent === "Not in the library" && !!document.querySelector("header.top") && !!document.querySelector("main .trouble a[href=\'/\']")')

    with scenario("a film and an episode can be marked watched and back from the phone"):
        phone.goto(f"/movies/{SPARE_FILM}")
        film = phone.page.locator("main .mark-film")
        film.scroll_into_view_if_needed()
        before = film.get_attribute("aria-pressed")
        flipped = "false" if before == "true" else "true"
        film.tap()
        phone.until(f'document.querySelector("main .mark-film").getAttribute("aria-pressed") === "{flipped}" && !document.querySelector("main .mark-film[data-act]:not([data-act=\'\'])")')
        assert phone.api("GET", f"/api/movies/{SPARE_FILM}")["body"]["finished"] == (flipped == "true")
        film.tap()
        phone.until(f'document.querySelector("main .mark-film").getAttribute("aria-pressed") === "{before}"')
        phone.goto(f"/shows/{SHOW}")
        phone.until('document.querySelectorAll("main .episode-row .mark-seen").length === 3')
        toggle = phone.page.locator("main .episode-row .mark-seen").nth(2)
        toggle.scroll_into_view_if_needed()
        before = toggle.get_attribute("aria-pressed")
        flipped = "false" if before == "true" else "true"
        toggle.tap()
        phone.until(f'document.querySelectorAll("main .episode-row .mark-seen")[2].getAttribute("aria-pressed") === "{flipped}"', 3000)
        assert phone.js("location.pathname") == f"/shows/{SHOW}", "marking an episode does not start it"
        phone.until('!document.querySelector("main .episode-row .mark-seen[data-act=pending]")')
        phone.page.reload(wait_until="commit")
        phone.until(f'document.querySelectorAll("main .episode-row .mark-seen")[2]?.getAttribute("aria-pressed") === "{flipped}"')
        phone.page.locator("main .episode-row .mark-seen").nth(2).tap()
        phone.until(f'document.querySelectorAll("main .episode-row .mark-seen")[2].getAttribute("aria-pressed") === "{before}" && !document.querySelector("main .episode-row .mark-seen[data-act=pending]")')

    with scenario("skipping on the TV keeps its sound output running, so the TV never has to pick the sound up again"):
        tv_plays_film(phone)
        resumed_at = tv_position() or 0.0
        try:
            tv_seek(phone, 20)
            tv_plays_past(22)
            skipped_from = tv_position() or 0.0
            status = tv.succeed("grep -l 'state:' /proc/asound/card*/pcm*p/sub*/status").split()[0]
            tv.succeed(f"(for i in $(seq 800); do head -1 {status}; sleep 0.005; done > /tmp/sound-states 2>&1) &")
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

    with scenario("an admin downloads a season the library lacks from the show page, through the usual proposal"):
        phone.goto(f"/shows/{SHOW}")
        phone.until('[...document.querySelectorAll("main .season-downloads button")].map((button) => button.textContent.trim()).join("|") === "Download season 2"')
        assert phone.js('document.querySelector("main .season-downloads").getBoundingClientRect().top > document.querySelector("main .detail-episodes").getBoundingClientRect().bottom'), "the season buttons come after the episodes"
        assert phone.js('document.querySelector("main .season-downloads").textContent.includes("Season 3 is downloading")'), "a season already downloading has no button"
        button = phone.page.locator("main .season-downloads button", has_text="Download season 2")
        button.scroll_into_view_if_needed()
        button.tap()
        phone.until('!!document.querySelector("main .season-downloads .download-choice.best")')
        assert phone.js('document.querySelector("main .season-downloads .settings h2")?.textContent === "Testserie season 2"'), "the proposal names the season"
        phone.page.locator("main .season-downloads .download-choice.best button").tap()
        phone.until('document.querySelector("main .season-downloads .download-choice .controls-said")?.textContent === "Downloading Testserie season 2"')
        phone.shot("season-download")
        phone.page.reload(wait_until="commit")
        phone.until('!!document.querySelector("main .episode-row")')
        phone.until('document.querySelector("main .season-downloads")?.textContent.includes("Season 2 is downloading")')
        assert phone.js('!document.querySelector("main .season-downloads button")'), "no button once every missing season is downloading"

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

    with scenario("the TV/phone switch moves at the tap and snaps back when the TV refuses"):
        phone.goto("/")
        at_home(phone)
        switch_to(phone, "tv")
        MODE = 'document.querySelector("#mode").dataset.mode'

        phone.js("""(() => {
          const answered = window.fetch;
          window.answeredFetch = answered;
          window.fetch = (input, init = {}) => String(input).endsWith("/api/screen") && init.method === "POST"
            ? new Promise((done) => setTimeout(done, 1500)).then(() => answered(input, init))
            : answered(input, init);
        })()""")
        phone.page.locator("#mode").tap()
        phone.until(f'{MODE} === "moving"', 600)
        phone.until(f'{MODE} === "phone"', 5000)
        eventually("the TV agrees the screen is on the phone", lambda: screen(phone)["kind"] == "phone")
        phone.page.locator("#mode").tap()
        phone.until(f'{MODE} === "tv"', 600)
        eventually("the TV agrees the screen is on the TV", lambda: screen(phone)["kind"] == "tv", timeout=5)
        phone.page.wait_for_timeout(500)
        assert phone.js(MODE) == "tv"
        phone.js("(() => { window.fetch = window.answeredFetch; })()")
        phone.page.route(
            "**/api/screen",
            lambda route: route.fulfill(status=422, json={"message": "The TV said no", "ok": False}) if route.request.method == "POST" else route.continue_(),
        )
        phone.page.locator("#mode").tap()
        phone.until(f'{MODE} === "tv" && [...document.querySelectorAll(".toast")].some((t) => t.textContent.includes("The TV said no"))', 5000)
        phone.page.unroute("**/api/screen")

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

    with scenario("Settings opens Feedback and comes Back, and keeps scrolling"):
        phone.goto("/")
        at_home(phone)
        phone.page.locator('a[href="/settings"]').first.tap()
        phone.until('location.pathname === "/settings" && !!document.querySelector("main .settings")')
        phone.page.wait_for_timeout(500)
        centred(phone, "Settings")
        for sub in ("/feedback",):
            link = phone.page.locator(f'main a[href="{sub}"]')
            link.scroll_into_view_if_needed()
            link.tap()
            phone.until(f'location.pathname === "{sub}" && !document.querySelector("semmel-shell")')
            answers(phone, sub)
            phone.page.locator("main a.back").first.tap()
            phone.until('location.pathname === "/settings" && !document.querySelector("semmel-shell") && !!document.querySelector("main .settings")')
            phone.page.wait_for_timeout(500)
            answers(phone, f"Settings after Back from {sub}")
            scrolls(phone, f"Settings after Back from {sub}")
        phone.page.locator("main a.back").first.tap()
        at_home(phone)
        scrolls(phone, "Home after Back from Settings")

    with scenario("signed-out visitors land on the picker and sign in with a password"):
        assert phone.api("POST", "/api/auth/account/password", {"password": "frederik-pass"})["status"] == 200
        visitor = Phone(playwright, who=None)
        visitor.goto("/")
        assert visitor.api("GET", "/api/library")["status"] == 401
        visitor.goto(f"/movies/{FILM}")
        visitor.until('location.pathname === "/profiles"')
        assert visitor.js('!document.querySelector(".bar-open") && !document.querySelector("#microphone")'), "a signed-out visitor gets no player or remote"
        visitor.until('[...document.querySelectorAll(".profile-choice")].some((choice) => choice.textContent.includes("Ramu") && choice.textContent.includes("Not set up yet"))')
        visitor.page.locator(".profile-choice", has_text="Ramu").click()
        visitor.until('document.querySelector("#sign-in-note").textContent.includes("invite link")', 5000)
        password_sign_in(visitor, "Frederik", "wrong-pass")
        visitor.until('document.querySelector("#sign-in-note").textContent.toLowerCase().includes("wrong")', 5000)
        visitor.page.fill("#password-input", "frederik-pass")
        visitor.page.locator("#password-form button[type=submit]").click()
        visitor.until(f'location.pathname === "/movies/{FILM}"')
        visitor.close()

    with scenario("profiles on one phone: each has its own Home, a viewer is limited, and switching back to the admin asks again"):
        phone.goto("/")
        phone.until(WATCHING)
        assert phone.js(AVATAR) == "F"
        phone.goto(f"/invite/{invite('Mami')}")
        phone.page.locator("#redeem").tap()
        phone.until('location.pathname === "/account" && !!document.querySelector("#password-set")')
        phone.page.fill("#password-set input[name=password]", "mami-pass-1")
        phone.page.locator("#password-set [type=submit]").tap()
        phone.until('[...document.querySelectorAll(".toast")].some((toast) => /saved/i.test(toast.textContent))')
        phone.goto("/")
        phone.until(f'{AVATAR} === "M"')
        phone.page.wait_for_timeout(800)
        assert phone.js(f"!({WATCHING})"), "Mami's Home has no Currently Watching of her own"
        phone.goto("/settings")
        phone.until('!!document.querySelector("main .settings")')
        assert phone.js("""!document.querySelector('main a[href="/storage"], main a[href="/downloads"], [data-post="/api/tv/pair"]')"""), "a viewer has no Storage, Downloads or TV pairing"
        assert phone.js("""!!document.querySelector('[data-post="/api/tv-profile"]')"""), "a viewer can put her profile on the TV"
        phone.goto(f"/shows/{SHOW}")
        phone.until('!!document.querySelector("main .episode-row")')
        assert phone.js('!document.querySelector("main .season-downloads")'), "a viewer is offered no season downloads"
        phone.goto("/storage")
        phone.until('location.pathname === "/"')
        phone.page.locator(".profile-link").tap()
        phone.until('location.pathname === "/profiles" && document.querySelector(".profile-choice.active .profile-name")?.textContent === "Mami"')
        centred(phone, "the signed-in picker")
        password_sign_in(phone, "Frederik", "frederik-pass")
        phone.until(f'location.pathname === "/" && {AVATAR} === "F"')
        phone.until(WATCHING)
        phone.goto("/profiles")
        phone.until('[...document.querySelectorAll(".profile-choice")].some((choice) => choice.textContent.includes("Mami") && choice.textContent.includes("On this device"))')
        phone.page.locator(".profile-choice", has_text="Mami").tap()
        phone.until(f'location.pathname === "/" && {AVATAR} === "M"')
        phone.goto("/profiles")
        password_sign_in(phone, "Frederik", "frederik-pass")
        phone.until(f'location.pathname === "/" && {AVATAR} === "F"')

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
        laptop.page.wait_for_timeout(500)
        laptop.page.click(".pp-mini-stop")
        laptop.until('!document.querySelector(".pp")', 3000)
        eventually("the TV hears that the laptop stopped", lambda: screen(laptop)["play"] is None)
        laptop.page.click(".bar-open")
        laptop.until('document.documentElement.hasAttribute("data-sheet")', 3000)
        laptop.page.wait_for_timeout(800)
        assert laptop.js('(() => { const box = document.querySelector(".sheet").getBoundingClientRect(); return box.width <= 640 && Math.abs(box.left - (innerWidth - box.right)) < 2; })()'), "the remote is a card on a laptop"
        laptop.close()

    for engine, device in (("chromium", "Pixel 7"), ("webkit", "iPhone 15")):
        with scenario(f"a page whose live connection died catches up with the TV ({engine})"):
            stale = Phone(playwright, engine, device)
            sockets: list[dict[str, bool]] = []

            def live(route, sockets=sockets):
                server = route.connect_to_server()
                state = {"dead": False}
                sockets.append(state)
                server.on_message(lambda message: None if state["dead"] else route.send(message))

            def kill(sockets=sockets):
                for state in sockets:
                    state["dead"] = True

            stale.page.route_web_socket(lambda url: "/api/" not in url, live)
            TITLE = 'document.querySelector(".bar-text strong")?.textContent'
            LINE = 'document.querySelector(".bar-text span")?.textContent'
            tv_plays_film(phone)
            stale.goto("/")
            stale.until(f'{TITLE} === "Testfilm"')
            stale.page.wait_for_timeout(1500)
            kill()
            phone.api("POST", "/api/playback/stop", {})
            tv_plays_nothing()
            stale.page.wait_for_timeout(2500)
            assert stale.js(f'{TITLE} === "Testfilm"'), "while the live connection is dead, the page can't know the TV stopped"
            stale.page.evaluate(AWAY, True)
            stale.page.wait_for_timeout(4500)
            stale.page.evaluate(AWAY, False)
            stale.until(f'{LINE} === "Nothing is playing"', 6000)
            stale.until("!document.documentElement.dataset.live", 6000)
            tv_plays_film(phone)
            stale.page.reload(wait_until="commit")
            stale.until(f'{TITLE} === "Testfilm"')
            stale.page.wait_for_timeout(1500)
            kill()
            phone.api("POST", "/api/playback/stop", {})
            tv_plays_nothing()
            stale.until(f'{LINE} === "Nothing is playing"', 25000)
            stale.until("!document.documentElement.dataset.live", 6000)
            stale.close()

    phone.close()

assert not FAILED, "failed scenarios:\n" + "\n".join(FAILED)
