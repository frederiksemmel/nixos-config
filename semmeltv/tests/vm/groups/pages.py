watched_a_while(phone)
watched_a_while(phone, {"show": SHOW, "season": 1, "episode": 1})

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

with scenario("Settings opens Feedback and comes Back, and keeps scrolling"):
    small = Phone(playwright, "chromium", "Galaxy S5")
    small.goto("/")
    at_home(small)
    small.page.locator('a[href="/settings"]').first.tap()
    small.until('location.pathname === "/settings" && !!document.querySelector("main .settings")')
    small.page.wait_for_timeout(500)
    centred(small, "Settings")
    for sub in ("/feedback",):
        link = small.page.locator(f'main a[href="{sub}"]')
        link.scroll_into_view_if_needed()
        link.tap()
        small.until(f'location.pathname === "{sub}" && !document.querySelector("semmel-shell")')
        answers(small, sub)
        small.page.locator("main a.back").first.tap()
        small.until('location.pathname === "/settings" && !document.querySelector("semmel-shell") && !!document.querySelector("main .settings")')
        small.page.wait_for_timeout(500)
        answers(small, f"Settings after Back from {sub}")
        scrolls(small, f"Settings after Back from {sub}")
    small.page.locator("main a.back").first.tap()
    at_home(small)
    scrolls(small, "Home after Back from Settings")
    small.close()

with scenario("a title tapped while the web app restarts opens once it is back"):
    phone.goto("/")
    at_home(phone)
    tv.systemctl("stop semmeltv-web.service")
    try:
        tile = phone.page.locator(TILE).first
        href = tile.get_attribute("href")
        tile.tap()
        phone.until('[...document.querySelectorAll(".toast")].some((toast) => toast.textContent.includes("nicht gebacken"))', 10000)
        assert phone.js("location.pathname") == "/"
    finally:
        tv.systemctl("start semmeltv-web.service")
    phone.until(f'location.pathname === {href!r} && !!document.querySelector("main .detail h1") && !document.querySelector(".toast:not([data-leaving])")', 30000)
    phone.js("history.back()")
    at_home(phone)

with scenario("a page asks to open as an app with the standard meta tag, so Chrome logs no deprecation"):
    phone.goto("/")
    at_home(phone)
    tags = phone.js('[...document.querySelectorAll("meta[name$=\'mobile-web-app-capable\']")].map((meta) => meta.name + "=" + meta.content)')
    assert sorted(tags) == ["apple-mobile-web-app-capable=yes", "mobile-web-app-capable=yes"], tags

with scenario("a title's details say who downloaded it, and nothing when nobody is known"):
    downloaded_by = """[...document.querySelectorAll("main .detail-facts > div")].filter((row) => row.querySelector("dt").textContent === "Downloaded by").map((row) => row.querySelector("dd").textContent).join("|")"""
    phone.goto(f"/movies/{FRESH_FILM}")
    phone.until('!!document.querySelector("main .detail-facts")')
    assert phone.js(downloaded_by) == "Frederik", phone.js(downloaded_by)
    phone.goto(f"/shows/{SHOW}")
    phone.until('!!document.querySelector("main .detail-facts")')
    assert phone.js(downloaded_by) == "Mami", phone.js(downloaded_by)
    phone.goto(f"/movies/{FILM}")
    phone.until('!!document.querySelector("main .detail-facts")')
    assert phone.js(downloaded_by) == "", phone.js(downloaded_by)

with scenario("a title that isn't there says so inside the app"):
    phone.goto("/movies/not-in-the-library")
    phone.until('document.querySelector("main h1")?.textContent === "Nicht in der Auslage" && !!document.querySelector("header.top") && !!document.querySelector("main .trouble a[href=\'/\']")')

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

with scenario("two people watch a series together: each sees where the other is, and one jumps to the other's place"):
    miriam = session_cookie("Miriam")

    def as_miriam(method, path):
        request = urllib.request.Request(
            f"{PHONE}{path}", data=b"" if method != "GET" else None, method=method,
            headers={"Origin": PHONE, "Cookie": f"semmeltv_session={miriam}"},
        )
        try:
            with urllib.request.urlopen(request) as response:
                return response.status, json.loads(response.read() or b"null")
        except urllib.error.HTTPError as error:
            return error.code, json.loads(error.read() or b"null")

    partner_row = '[...document.querySelectorAll("main .together-row")].find((row) => row.dataset.partner === "Miriam")'
    phone.goto(f"/shows/{SHOW}")
    phone.until('!!document.querySelector("main .together-add summary")')
    phone.page.locator("main .together-add summary").tap()
    phone.page.locator("main .together-add .chip", has_text="Miriam").tap()
    phone.until(f'!!{partner_row} && {partner_row}.textContent.includes("Hasn\'t started yet") && !{partner_row}.querySelector(".together-catch-up")')
    frederik_id = 1
    status, answer = as_miriam("POST", f"/api/shows/{SHOW}/together/{frederik_id}/catch-up")
    assert status == 200 and answer["ok"], (status, answer)
    mine = phone.api("GET", f"/api/shows/{SHOW}")["body"]
    _, hers = as_miriam("GET", f"/api/shows/{SHOW}")
    def place(body):
        return body["next"] and (body["next"]["season"], body["next"]["episode"])

    assert place(hers) == place(mine), (hers["next"], mine["next"])
    assert [partner["name"] for partner in hers["together"]["partners"]] == ["Frederik"], hers["together"]
    assert hers["together"]["partners"][0]["with_me"], hers["together"]
    phone.page.reload(wait_until="commit")
    phone.until(f'!!{partner_row}?.querySelector(".together-same")')
    phone.page.locator("main .together-row .together-leave").tap()
    phone.until('!document.querySelector("main .together-row") && !!document.querySelector("main .together-add")')
    assert as_miriam("GET", f"/api/shows/{SHOW}")[1]["together"]["partners"] == []
