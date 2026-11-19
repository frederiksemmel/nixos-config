import sys
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


def until(page, engine, step, condition, timeout=8000):
    try:
        page.wait_for_function(condition, timeout=timeout)
        check(engine, step, True)
    except Exception:
        check(engine, step, False, f"at {page.evaluate('location.pathname')}")


def forward(route):
    try:
        route.fulfill(response=route.fetch(url=FAKE + urlparse(route.request.url).path))
    except Exception:
        pass


def fake(path):
    urllib.request.urlopen(urllib.request.Request(FAKE + path, method="POST", data=b""))


def sign_in(page, engine):
    page.goto(BASE + "/", wait_until="commit")
    until(page, engine, "the profile picker shows", 'location.pathname === "/profiles" && !!document.querySelector(".profile-choice")')
    page.locator(".profile-choice", has_text="Frederik").click()
    until(page, engine, "Frederik is asked for a password", '!document.querySelector("#password-form").hidden', 5000)
    page.fill("#password-input", "frederik-pass")
    page.locator("#password-form button[type=submit]").click()
    until(page, engine, "and lands Home", 'location.pathname === "/" && !!document.querySelector(".bar-open")', 8000)


def opens_title(page, engine):
    tile = page.locator(TILE).first
    href = tile.get_attribute("href")
    tile.scroll_into_view_if_needed()
    tile.tap()
    until(page, engine, "a tapped title opens",
          f'location.pathname === {href!r} && !document.querySelector("semmel-shell") && document.querySelector("main .detail h1")')


def finds_subtitles(page, engine):
    until(page, engine, "a film without subtitles offers to find them", '!!document.querySelector("main .find-subtitles-row .pill")', 3000)
    page.wait_for_timeout(300)
    find = page.locator("main .find-subtitles-row .pill")
    if not find.count():
        return
    find.tap()
    until(page, engine, "finding subtitles says they were added and how well they fit",
          '[...document.querySelectorAll(".toast")].some((toast) => toast.textContent.includes("found by title only")) && document.querySelector("main .find-subtitles-row .pill").hidden', 5000)


def sound(page, engine):
    phones = page.locator(".sound .choice").nth(1)
    phones.scroll_into_view_if_needed()
    phones.tap()
    until(page, engine, "tapping AirPods shows it connecting",
          'document.querySelector(".sound .answer").textContent.startsWith("Connecting")', 3000)
    until(page, engine, "AirPods take over the sound",
          'document.querySelector(".sound").dataset.output === "headphones" && getComputedStyle(document.querySelector(".bar-phones")).display !== "none"', 5000)
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
    until(page, engine, "a failed connect shows as an error toast", '!!document.querySelector(".toast[data-tone=error]")', 5000)


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


def blur(layer):
    return f"""(() => {{
        const style = getComputedStyle(document.querySelector(".bar"), "{layer}");
        const filter = style.backdropFilter || style.webkitBackdropFilter || "";
        return {{ blur: parseFloat((/blur\\(([\\d.]+)px\\)/.exec(filter) || [0, 0])[1]), mask: style.maskImage || style.webkitMaskImage || "" }};
    }})()"""


def bar_blur_grows_downwards(page, engine):
    top, bottom = page.evaluate(blur("::before")), page.evaluate(blur("::after"))
    check(engine, "the bar blurs lightly over its whole height", 0 < top["blur"] and top["mask"] in ("", "none"), str(top))
    check(engine, "and much more towards the bottom, fading in from the top", bottom["blur"] >= 3 * top["blur"] and "gradient" in bottom["mask"], str(bottom))


def selectable(page):
    return page.evaluate("(() => { const style = getComputedStyle(document.body); return style.userSelect || style.webkitUserSelect; })()")


def text_selects_with_a_mouse_only(p, engine, page):
    check(engine, "a phone can't select page text", selectable(page) == "none", selectable(page))
    browser = getattr(p, engine).launch()
    desktop = browser.new_context(**p.devices["Desktop Chrome" if engine == "chromium" else "Desktop Safari"]).new_page()
    desktop.goto(BASE + "/profiles", wait_until="load")
    check(engine, "a mouse can select page text", selectable(desktop) == "text", selectable(desktop))
    browser.close()


def zoom_and_description(page, engine):
    head = page.evaluate("""(() => ({
      description: document.querySelector('meta[name="description"]')?.content || "",
      viewport: document.querySelector('meta[name="viewport"]').content,
      touch: getComputedStyle(document.documentElement).touchAction,
    }))()""")
    check(engine, "the page has a description", len(head["description"]) > 40, head["description"])
    check(engine, "the viewport doesn't lock zoom, so Lighthouse passes", "user-scalable" not in head["viewport"] and "maximum-scale" not in head["viewport"], head["viewport"])
    if engine == "chromium":
        check(engine, "a pinch still doesn't zoom the page", head["touch"] == "pan-x pan-y", head["touch"])


def back_never_loops(page, engine):
    page.goto(BASE + "/feedback", wait_until="commit")
    until(page, engine, "a page opened directly shows", 'location.pathname === "/feedback" && !!document.querySelector("main .feedback-page a.back")')
    page.wait_for_timeout(500)
    page.locator("main a.back").first.tap()
    until(page, engine, "its Back opens the page above it", 'location.pathname === "/settings" && !document.querySelector("semmel-shell")')
    page.wait_for_timeout(500)
    page.locator("main a.back").first.tap()
    until(page, engine, "and that page's Back goes on up, not back down", 'location.pathname === "/" && !document.querySelector("semmel-shell")')
    page.wait_for_timeout(500)
    page.go_back(wait_until="commit")
    page.wait_for_timeout(1000)
    where = page.evaluate("location.pathname")
    check(engine, "the browser's back skips the pages Back left", where not in ("/settings", "/feedback"), f"at {where}")


def run(p, engine, device):
    browser = getattr(p, engine).launch()
    page = browser.new_context(**p.devices[device]).new_page()
    page.set_default_timeout(8000)
    errors = []
    page.on("pageerror", lambda error: errors.append(f"{error} at {page.url} {getattr(error, 'stack', '')}"))
    page.route("**/api/**", forward)
    fake("/fake/forget-subtitles")
    sign_in(page, engine)
    bar_blur_grows_downwards(page, engine)
    zoom_and_description(page, engine)
    text_selects_with_a_mouse_only(p, engine, page)
    opens_title(page, engine)
    finds_subtitles(page, engine)
    page.locator("main a.back").first.tap()
    until(page, engine, "Back returns home", 'location.pathname === "/" && !document.querySelector("semmel-shell")')
    page.wait_for_timeout(500)
    headphones(page, engine)
    back_never_loops(page, engine)
    check(engine, "no script errors", not errors, "; ".join(errors))
    browser.close()


with sync_playwright() as p:
    run(p, "webkit", "iPhone 15")
    run(p, "chromium", "Pixel 7")

print("PASS" if not failures else "FAIL: " + ", ".join(failures))
sys.exit(1 if failures else 0)
