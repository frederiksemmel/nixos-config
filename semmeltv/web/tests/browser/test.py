import sys
import time

from playwright.sync_api import sync_playwright

BASE = sys.argv[1]
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
    page.wait_for_timeout(700)
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


def run(p, engine, device):
    browser = getattr(p, engine).launch()
    page = browser.new_context(**p.devices[device]).new_page()
    page.set_default_timeout(8000)
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.goto(BASE + "/", wait_until="commit")
    page.wait_for_timeout(2500)
    if not answers(page, engine, "cold load"):
        browser.close()
        return
    check(engine, "a film is playing with a chosen speed",
          page.evaluate('!!document.querySelector(".now-state") && !!document.querySelector(".choices .choice[aria-pressed=true]")'))
    page.wait_for_timeout(2500)
    answers(page, engine, "after the TV paused and resumed")
    scrolls(page, engine, "cold load")
    check(engine, "page chrome is not selectable", page.evaluate('getComputedStyle(document.body).userSelect === "none" || getComputedStyle(document.body).webkitUserSelect === "none"'))
    page.locator("main .chips .chip", has_text="Movies").tap()
    until(page, engine, "a tapped chip switches", '[...document.querySelectorAll("main .chips .chip.on")].some((chip) => chip.textContent.trim() === "Movies")', 3000)
    page.locator("main .chips .chip", has_text="Home").tap()
    until(page, engine, "Home chip switches back", '[...document.querySelectorAll("main .chips .chip.on")].some((chip) => chip.textContent.trim() === "Home")', 3000)
    page.wait_for_selector(TILE)
    page.wait_for_timeout(1000)
    opens_title(page, engine, "cold load")
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
    page.locator(".bar-open").tap()
    until(page, engine, "remote sheet opens", 'document.documentElement.hasAttribute("data-sheet")', 3000)
    page.wait_for_timeout(800)
    answers(page, engine, "sheet open")
    page.locator(".sheet-close").tap()
    until(page, engine, "remote sheet closes", '!document.documentElement.hasAttribute("data-sheet")', 3000)
    page.wait_for_timeout(800)
    answers(page, engine, "after the sheet")
    scrolls(page, engine, "after the sheet")
    settings_subpages(page, engine)
    page.locator(".bar-toggle").tap()
    answers(page, engine, "after play/pause")
    opens_title(page, engine, "after play/pause")
    check(engine, "no script errors", not errors, "; ".join(errors))
    browser.close()


def desktop(p):
    engine = "desktop"
    browser = p.chromium.launch()
    page = browser.new_page(viewport={"width": 760, "height": 800})
    page.set_default_timeout(8000)
    page.goto(BASE + "/", wait_until="commit")
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


with sync_playwright() as p:
    run(p, "webkit", "iPhone 15")
    run(p, "chromium", "Pixel 7")
    desktop(p)

print("PASS" if not failures else "FAIL: " + ", ".join(failures))
sys.exit(1 if failures else 0)
