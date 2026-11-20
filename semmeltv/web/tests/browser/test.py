import json
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
    until(page, engine, "Notifications say how to turn them on here, with a button only where it works",
          '(() => { const s = document.querySelector("main [data-notify=section]"); if (!s || !s.dataset.state) return false; const shown = !s.querySelector("[data-notify=toggle]").hidden; return shown === ["on", "off"].includes(s.dataset.state) && s.querySelector("[data-notify=detail]").textContent.length > 20; })()', 5000)
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


WATCH_SCRUB = """(ms) => new Promise((done) => {
    const scrub = document.querySelector(".scrub");
    let batches = 0;
    let fills = 0;
    const watcher = new MutationObserver((records) => {
        batches += 1;
        if (records.some((record) => record.target.classList && record.target.classList.contains("scrub-fill"))) fills += 1;
    });
    watcher.observe(scrub, { subtree: true, attributes: true, childList: true, characterData: true });
    setTimeout(() => { watcher.disconnect(); done({ batches, fills }); }, ms);
})"""


def scrub_rests_while_the_remote_is_shut(page, engine):
    shut = page.evaluate(WATCH_SCRUB, 2000)
    check(engine, "with the remote shut, its progress bar changes only when the TV reports, not every frame",
          shut["batches"] < 30, f"{shut} in 2 s")
    page.locator(".bar-open").tap()
    until(page, engine, "the remote opens over Home", 'document.documentElement.hasAttribute("data-sheet")', 3000)
    opened = page.evaluate(WATCH_SCRUB, 4500)
    check(engine, "and with it open the bar glides between the TV's reports", opened["fills"] > 15, f"{opened} in 4.5 s")
    page.locator(".sheet-close").tap()
    until(page, engine, "the remote shuts again", '!document.documentElement.hasAttribute("data-sheet")', 3000)
    page.wait_for_timeout(800)


def selectable(page):
    return page.evaluate("(() => { const style = getComputedStyle(document.body); return style.userSelect || style.webkitUserSelect; })()")


def text_selects_with_a_mouse_only(p, engine, page):
    check(engine, "a phone can't select page text", selectable(page) == "none", selectable(page))
    browser = getattr(p, engine).launch()
    desktop = browser.new_context(**p.devices["Desktop Chrome" if engine == "chromium" else "Desktop Safari"]).new_page()
    desktop.goto(BASE + "/profiles", wait_until="load")
    check(engine, "a mouse can select page text", selectable(desktop) == "text", selectable(desktop))
    box = desktop.evaluate("""(() => {
      const bar = document.createElement("section");
      bar.className = "bar";
      bar.innerHTML = '<button type="button" class="bar-open"><span class="bar-text">Dune · 1:02:03 left</span></button>';
      document.body.append(bar);
      getSelection().removeAllRanges();
      const box = bar.querySelector(".bar-text").getBoundingClientRect();
      return { x: box.x + 4, y: box.y + box.height / 2 };
    })()""")
    desktop.mouse.move(box["x"], box["y"])
    desktop.mouse.down()
    desktop.mouse.move(box["x"] + 200, 20, steps=10)
    desktop.mouse.up()
    picked = desktop.evaluate("getSelection().toString()")
    check(engine, "a mouse drag that starts on the bottom bar selects nothing", picked == "", repr(picked[:80]))
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


ZOOMED = """(() => {
  Object.defineProperty(visualViewport, "scale", { configurable: true, get: () => 1.6 });
  visualViewport.dispatchEvent(new Event("resize"));
})()"""
UNZOOMED = """(() => {
  delete visualViewport.scale;
  visualViewport.dispatchEvent(new Event("resize"));
})()"""
PINCH = """(() => {
  const event = new Event("gesturestart", { cancelable: true });
  document.dispatchEvent(event);
  return { cancelled: event.defaultPrevented, touch: getComputedStyle(document.documentElement).touchAction };
})()"""
VIEWPORT = "document.querySelector('meta[name=\"viewport\"]').content"


def zoom_comes_back(page, engine):
    check(engine, "a pinch on an unzoomed page is cancelled", page.evaluate(PINCH)["cancelled"])
    page.evaluate(ZOOMED)
    pinch = page.evaluate(PINCH)
    check(engine, "a zoomed page lets a pinch zoom back out", not pinch["cancelled"] and pinch["touch"] == "auto", str(pinch))
    page.evaluate(UNZOOMED)
    check(engine, "back at 1x, pinches are cancelled again", page.evaluate(PINCH)["cancelled"])
    page.locator("#feedback-open").tap()
    until(page, engine, "the feedback box opens with its field focused", 'document.activeElement?.name === "message"')
    check(engine, "while typing, iOS can't zoom into the field", "maximum-scale=1" in page.evaluate(VIEWPORT), page.evaluate(VIEWPORT))
    page.locator(".feedback-close").tap()
    until(page, engine, "after the field, zoom is unlocked again", f'!{VIEWPORT}.includes("maximum-scale")', 3000)


COUNTS = """JSON.stringify(Object.fromEntries([...document.querySelectorAll(".report-filters button")]
    .map((button) => [button.querySelector("b").textContent, Number(button.querySelector("span").textContent)])))"""
SHOWN = '[...document.querySelectorAll(".report")].filter((report) => !report.hidden).length'


def counted(**counts):
    return json.dumps(json.dumps({"All": 3, "New": counts["new"], "Needs review": counts["review"], "Done": counts["done"]}, separators=(",", ":")))


def feedback_follows_changes(page, engine):
    page.goto(BASE + "/feedback", wait_until="commit")
    until(page, engine, "the Feedback page counts the reports by status", f"{COUNTS} === {counted(new=1, review=1, done=1)}")
    page.wait_for_timeout(500)
    filters = page.locator(".report-filters button")
    filters.nth(1).tap()
    until(page, engine, "the New filter shows the one new report", f"{SHOWN} === 1", 3000)
    row = page.locator(".report", has_text="The list jumped")
    row.get_by_role("button", name="Set status").tap()
    row.locator(".report-edit textarea").nth(0).fill("It is a design question")
    row.locator(".report-edit textarea").nth(1).fill("Keep or drop the jump")
    row.locator(".report-edit button[type=submit]").tap()
    until(page, engine, "a status set on the page moves the counts at once", f"{COUNTS} === {counted(new=0, review=2, done=1)}", 3000)
    until(page, engine, "and the report leaves the New filter", f"{SHOWN} === 0", 3000)
    filters.nth(2).tap()
    until(page, engine, "and shows under Needs review", f"{SHOWN} === 2", 3000)
    filters.nth(3).tap()
    done = page.locator(".report", has_text="The sheet opened by itself")
    done.get_by_role("textbox", name="Comment").fill("Still opens")
    done.get_by_role("button", name="Comment").tap()
    until(page, engine, "a comment that reopens a done report moves it to New",
          f'{COUNTS} === {counted(new=1, review=2, done=0)} && {SHOWN} === 0', 3000)
    filters.nth(0).tap()


def wheel(page, at, deltas):
    page.mouse.move(*at)
    for delta in deltas:
        page.mouse.wheel(0, delta)
        page.wait_for_timeout(16)


def sheet_lift(page):
    return page.evaluate("""(() => {
      const match = /translateY\\(([-\\d.]+)px\\)/.exec(document.getElementById("sheet").style.transform);
      return { dragging: document.documentElement.hasAttribute("data-sheet-drag"), y: match ? parseFloat(match[1]) : null };
    })()""")


def scrolling_pulls_the_remote(page, engine):
    page.goto(BASE + "/", wait_until="commit")
    until(page, engine, "Home shows the bar", 'location.pathname === "/" && !!document.querySelector(".bar-open")')
    page.wait_for_timeout(500)
    opened = 'document.documentElement.hasAttribute("data-sheet")'
    shut = '!document.documentElement.hasAttribute("data-sheet") && !document.documentElement.hasAttribute("data-sheet-drag")'
    box = page.locator(".bar-open").bounding_box()
    bar = (box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    viewport = page.viewport_size
    middle = (viewport["width"] / 2, viewport["height"] / 2)
    wheel(page, middle, [20] * 5)
    page.wait_for_timeout(300)
    check(engine, "scrolling the page doesn't pull the remote up", not page.evaluate(opened) and not sheet_lift(page)["dragging"])
    page.evaluate("window.scrollTo(0, 0)")
    wheel(page, bar, [6] * 12)
    lift = sheet_lift(page)
    check(engine, "a touchpad scroll on the bar lifts the remote with the fingers", lift["dragging"] and lift["y"] is not None and not page.evaluate(opened), str(lift))
    wheel(page, bar, [20] * 40)
    until(page, engine, "and lets it come up when the fingers stop", opened, 2000)
    page.wait_for_timeout(800)
    wheel(page, middle, [-20] * 40)
    until(page, engine, "scrolling back on the remote's top pulls it down again", shut, 2000)
    page.wait_for_timeout(800)
    wheel(page, bar, [100])
    until(page, engine, "one notch of a mouse wheel on the bar brings it up", opened, 2000)
    page.wait_for_timeout(800)
    scroller = page.locator("#sheet .sheet-scroll")
    page.evaluate('document.querySelector("#sheet .sheet-scroll").scrollTop = 200')
    scrolled = page.evaluate('document.querySelector("#sheet .sheet-scroll").scrollTop')
    if scrolled > 0:
        wheel(page, middle, [-20] * 20)
        page.wait_for_timeout(300)
        check(engine, "scrolling up inside the remote first scrolls it to its top, and stops there", page.evaluate(opened), str(scroller.evaluate("e => e.scrollTop")))
    wheel(page, middle, [-100])
    until(page, engine, "a new scroll down from its top closes it", shut, 2000)
    page.wait_for_timeout(800)


def deploys_from_the_feedback_page(page, engine):
    page.goto(BASE + "/feedback", wait_until="commit")
    until(page, engine, "the Feedback page lists what main has that the TV doesn't",
          'document.querySelectorAll(".deploy-commits li").length === 1 && !!document.querySelector(".deploy-commits a[href^=\'#20261119-\']")')
    page.wait_for_timeout(500)
    page.get_by_role("button", name="Deploy now").tap()
    until(page, engine, "Deploy now asks beast and says so",
          'document.querySelector(".deploy-said").textContent.includes("asked for a deploy") && document.querySelector(".deploy-actions").hidden', 3000)


def below(page, upper, lower):
    return page.evaluate(
        "([upper, lower]) => document.querySelector(lower).getBoundingClientRect().top"
        " >= document.querySelector(upper).getBoundingClientRect().bottom",
        [upper, lower],
    )


def watches_together(page, engine):
    page.goto(BASE + "/shows/outlander", wait_until="commit")
    until(page, engine, "a series page shows", '!!document.querySelector("main .detail h1") && !!document.querySelector("main .together-add")')
    page.wait_for_timeout(500)
    check(engine, "Watch together sits under the play buttons", below(page, "main .detail-playback", "main .together"))
    toggle = page.locator("main .together-add summary")
    box = toggle.bounding_box() or {"height": 0}
    check(engine, "and is a button big enough to tap", box["height"] >= 40, f"{box['height']} px high")
    toggle.tap()
    until(page, engine, "tapping it lists the people", '!!document.querySelector("main .together-people .chip")'
          ' && document.querySelector("main .together-people .chip").getBoundingClientRect().height > 0', 3000)
    page.locator("main .together-people .chip", has_text="Mami").tap()
    until(page, engine, "picking Mami pairs the two of you", '[...document.querySelectorAll("main .together-row")]'
          '.some((row) => row.dataset.partner === "Mami")', 8000)
    page.locator("main .together-leave").first.tap()
    until(page, engine, "and × ends it", '!document.querySelector("main .together-row")', 8000)


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


def shares_a_title(page, engine):
    page.evaluate("""() => {
        window.shared = null;
        Object.defineProperty(navigator, "share", { configurable: true, value: (data) => { window.shared = data; return Promise.resolve(); } });
    }""")
    page.locator("main .pill.share").tap()
    until(page, engine, "Share hands the title and its link to the share sheet",
          'window.shared && window.shared.text.startsWith("I\'m watching ") && window.shared.text.endsWith("\\n" + location.origin + location.pathname)', 3000)
    page.evaluate('() => Object.defineProperty(navigator, "share", { configurable: true, value: undefined })')
    page.locator("main .pill.share").tap()
    until(page, engine, "without a share sheet Share copies the link",
          '[...document.querySelectorAll(".toast")].some((toast) => /Link copied|Copy this link/.test(toast.textContent))', 3000)


def opens_a_shared_episode(page, engine):
    page.goto(BASE + "/shows/outlander?season=1&episode=2", wait_until="commit")
    until(page, engine, "a shared episode link outlines that episode and only it",
          'document.querySelectorAll("main .episode-row.shared").length === 1'
          ' && document.querySelector("main .episode-row.shared .share-episode").dataset.share === "/shows/outlander?season=1&episode=2"')


def run(p, engine, device):
    browser = getattr(p, engine).launch()
    page = browser.new_context(**p.devices[device]).new_page()
    page.set_default_timeout(8000)
    errors = []
    page.on("pageerror", lambda error: errors.append(f"{error} at {page.url} {getattr(error, 'stack', '')}"))
    page.route("**/api/**", forward)
    fake("/fake/forget-subtitles")
    sign_in(page, engine)
    scrub_rests_while_the_remote_is_shut(page, engine)
    bar_blur_grows_downwards(page, engine)
    zoom_and_description(page, engine)
    zoom_comes_back(page, engine)
    text_selects_with_a_mouse_only(p, engine, page)
    opens_title(page, engine)
    finds_subtitles(page, engine)
    shares_a_title(page, engine)
    page.locator("main a.back").first.tap()
    until(page, engine, "Back returns home", 'location.pathname === "/" && !document.querySelector("semmel-shell")')
    page.wait_for_timeout(500)
    headphones(page, engine)
    feedback_follows_changes(page, engine)
    if engine == "chromium":
        scrolling_pulls_the_remote(page, engine)
    deploys_from_the_feedback_page(page, engine)
    back_never_loops(page, engine)
    watches_together(page, engine)
    opens_a_shared_episode(page, engine)
    check(engine, "no script errors", not errors, "; ".join(errors))
    browser.close()


with sync_playwright() as p:
    run(p, "webkit", "iPhone 15")
    run(p, "chromium", "Pixel 7")

print("PASS" if not failures else "FAIL: " + ", ".join(failures))
sys.exit(1 if failures else 0)
