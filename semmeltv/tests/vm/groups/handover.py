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
    eventually("the phone opens its stream again", lambda: phone.page.wait_for_timeout(200) or opens or None, timeout=60)
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
