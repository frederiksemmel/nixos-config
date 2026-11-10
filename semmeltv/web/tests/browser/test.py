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
    opens_title(page, engine, "cold load")
    answers(page, engine, "title page")
    scrolls(page, engine, "title page")
    page.locator("main a.back").first.tap()
    until(page, engine, "Back returns home", 'location.pathname === "/" && !document.querySelector("semmel-shell")')
    page.wait_for_timeout(500)
    answers(page, engine, "after Back")
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
    page.locator(".bar-toggle").tap()
    answers(page, engine, "after play/pause")
    opens_title(page, engine, "after play/pause")
    check(engine, "no script errors", not errors, "; ".join(errors))
    browser.close()


with sync_playwright() as p:
    run(p, "webkit", "iPhone 15")
    run(p, "chromium", "Pixel 7")

print("PASS" if not failures else "FAIL: " + ", ".join(failures))
sys.exit(1 if failures else 0)
