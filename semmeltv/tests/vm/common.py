import contextlib
import json
import re
import traceback
import shlex
import time
import urllib.error
import urllib.request
from typing import Any

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
EXPECTED_FAILURE = ""
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
        cookies = [header.split(";")[0] for header in response.headers.get_all("Set-Cookie") or []]
    session = next((cookie.split("=", 1)[1] for cookie in cookies if cookie.startswith("semmeltv_session=")), None)
    assert session, f"redeeming an invite for {name} set no session"
    setup = urllib.request.Request(
        f"{PHONE}/api/auth/account/password",
        data=json.dumps({"password": f"{name.lower()}-vm-pass"}).encode(),
        method="POST",
        headers={"Origin": PHONE, "Content-Type": "application/json", "Cookie": "; ".join(cookies)},
    )
    urllib.request.urlopen(setup).close()
    return session


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


NO_REMOTE = '[...document.querySelectorAll(".bar, #sheet, .sheet-dim")].every((element) => getComputedStyle(element).display === "none")'


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
    phone.until(f"{VIDEO}.seekable.length > 0 && {end} > 58", 30000)
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
    assert phone.js(f'{NO_REMOTE} && !document.elementFromPoint(innerWidth / 2, innerHeight - 30)?.closest(".pp")'), "watching here with nothing left playing shows no bar and no TV remote"
    switch_to(phone, "tv")
    phone.until('getComputedStyle(document.querySelector(".bar")).display !== "none"', 3000)


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


def wheel(phone, dy, times):
    for _ in range(times):
        phone.page.mouse.wheel(0, dy)
        phone.page.wait_for_timeout(16)


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


STREAM_API = "http://127.0.0.1:8090"
STREAM_ORIGIN = "http://localhost:8080"
STREAM_DIR = "/var/lib/semmeltv/stream"
NO_DOLBY = {"hevc": False, "eac3": False, "ac3": False}


def stream_cookie():
    cli = tv.succeed("systemctl cat cage-tty1 | grep -o '/nix/store/[^ ]*/bin/semmeltv' | head -1").strip()
    printed = tv.succeed(
        f"runuser -u semmeltv -- env SEMMELTV_DB=/var/lib/semmeltv/semmeltv.db "
        f"SEMMELTV_ORIGINS={STREAM_ORIGIN} {cli} invite Frederik"
    )
    invited = re.search(r"/invite/(\S+)", printed)
    assert invited, printed
    token = invited.group(1)
    headers = tv.succeed(
        f"curl -sf -D - -o /dev/null -X POST -H 'Origin: {STREAM_ORIGIN}' {STREAM_API}/api/auth/invite/{token}"
    )
    cookie = re.search(r"(?i)set-cookie: (semmeltv_session=[^;]+)", headers)
    assert cookie, headers
    return cookie.group(1)


def stream_call(cookie, method, path, body=None, expect="-f"):
    data = ""
    if body is not None:
        data = f"-H 'content-type: application/json' --data '{json.dumps(body)}'"
    return tv.succeed(
        f"curl -s {expect} -X {method} -H 'Origin: {STREAM_ORIGIN}' -H 'Cookie: {cookie}' {data} '{STREAM_API}{path}'"
    )


def stream_get(path):
    return tv.succeed(f"curl -sf '{STREAM_API}{path}'")


def stream_status(path):
    return tv.succeed(f"curl -s -o /dev/null -w '%{{http_code}}' '{STREAM_API}{path}'").strip()


def stream_lengths(playlist):
    return [float(line[8:].split(",")[0]) for line in playlist.splitlines() if line.startswith("#EXTINF:")]


def watched_a_while(phone, title=None):
    opened = phone.api(
        "POST",
        "/api/stream/open",
        (title or {"movie": FILM}) | {"client": NO_DOLBY, "device": {"id": "vm-setup", "name": "setup"}},
    )
    assert opened["status"] == 200, opened
    stream = opened["body"]["id"]
    played = WATCHING_SECONDS + 2
    reported = phone.api("POST", f"/api/stream/{stream}/progress", {"position": played, "played": played, "state": "stopped"})
    assert reported["status"] == 200, reported
    phone.api("DELETE", f"/api/stream/{stream}")
    eventually("it is in Continue Watching", lambda: phone.api("GET", "/api/library")["body"]["watching"] or None, timeout=15)


start_all()
tv.wait_for_unit("cage-tty1.service")
tv.wait_for_open_port(8090)
tv.wait_for_unit("semmeltv-web.service")
tv.wait_for_open_port(8091)
tv.wait_for_open_port(8080)
tv.forward_port(8080, 8080)
tv.screenshot("tv-home")

playwright = sync_playwright().start()
phone = Phone(playwright)
phone.goto("/")
phone.until(LOADED)
CLEANUP.append(lambda: phone.shot(f"failed-{len(FAILED)}"))
CLEANUP.append(lambda: stop_everything(phone))
CLEANUP.append(lambda: phone.goto("/"))


