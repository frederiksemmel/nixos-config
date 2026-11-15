from playwright.sync_api import sync_playwright

PHONE = "http://localhost:8080"

start_all()

with subtest("the TV boots into the app and shows the library"):
    tv.wait_for_unit("cage-tty1.service")
    tv.wait_for_open_port(8090)
    tv.wait_for_text("Testfilm", timeout=180)
    tv.screenshot("tv-home")

with subtest("the web app answers a phone"):
    tv.wait_for_unit("semmeltv-web.service")
    tv.wait_for_open_port(8091)
    tv.wait_for_open_port(80)
    tv.forward_port(8080, 80)
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(args=["--disable-gpu"])
        page = browser.new_page(viewport={"width": 390, "height": 844})
        page.goto(PHONE)
        page.get_by_text("Frederik", exact=True).first.wait_for()
        page.screenshot(path=os.path.join(os.environ["out"], "phone-profiles.png"))
        browser.close()
