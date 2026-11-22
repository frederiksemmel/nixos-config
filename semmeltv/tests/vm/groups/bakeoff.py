def cookie_of(name):
    return f"semmeltv_session={session_cookie(name)}"


def open_stream(cookie, device, expect="-f"):
    body = {"movie": FILM, "client": NO_DOLBY, "device": {"id": device, "name": device}}
    return json.loads(stream_call(cookie, "POST", "/api/stream/open", body, expect=expect))


def hold_places(cookie, count, prefix):
    return [open_stream(cookie, f"{prefix}-{number}")["id"] for number in range(count)]


def let_go(ids):
    for id in ids:
        tv.succeed(f"curl -s -X DELETE {STREAM_API}/api/stream/{id}")


def press_perfectly(cookie, path):
    return json.loads(stream_call(cookie, "POST", path, {"input": "touch", "presses": [50, 50, 50]}))


def watch_here(someone):
    time.sleep(1)
    film_page(someone)
    someone.until(LOADED)
    someone.until('document.documentElement.hasAttribute("data-phone-ready") && !document.documentElement.classList.contains("pp-open")')
    someone.page.locator("main button.play[data-play-movie]").tap()


SHEET = 'document.querySelector(".pp-bake:not([hidden]) h2")?.textContent'
frederik = cookie_of("Frederik")
mami = cookie_of("Mami")
ramu = cookie_of("Ramu")

with scenario("a fourth viewer finds the oven full and takes the place of someone who stops watching"):
    held = hold_places(frederik, 3, "held")
    try:
        full = open_stream(mami, "mami-phone", expect="")
        assert full["full"]["heading"] == "Der Ofen ist voll", full
        assert sorted(watcher["seat"]["id"] for watcher in full["full"]["watchers"]) == sorted(held), full
        assert all(watcher["person"] == "Frederik" and watcher["title"] == "Testfilm" for watcher in full["full"]["watchers"]), full
        made = json.loads(stream_call(mami, "POST", "/api/challenges", {"seat": held[0], "device": {"id": "mami-phone", "name": "Mamis phone"}}))
        asked = json.loads(stream_get(f"/api/stream/{held[0]}/challenge"))
        assert asked["stage"] == "asked" and asked["challenger"] == "Mami" and asked["side"] == "defender", asked
        tv.succeed(f"curl -sf -X POST {STREAM_API}/api/stream/{held[0]}/challenge/drop-off")
        assert stream_status(f"/api/stream/{held[0]}/main.m3u8") == "404", "the stream that dropped off still plays"
        done = json.loads(stream_call(mami, "GET", f"/api/challenges/{made['id']}"))
        assert (done["stage"], done["winner"], done["ending"]) == ("done", "challenger", "dropped-off"), done
        held.append(open_stream(mami, "mami-phone")["id"])
    finally:
        let_go(held)

with scenario("a challenger on a phone sees the full oven, bakes, loses and rests"):
    held = hold_places(mami, 3, "mami")
    miriam = Phone(playwright, who="Miriam")
    try:
        watch_here(miriam)
        miriam.until(f'{SHEET} === "Der Ofen ist voll" && document.querySelectorAll(".pp-bake button.pp-bake-row").length === 3')
        miriam.shot("oven-full")
        miriam.page.locator(".pp-bake button.pp-bake-row").first.tap()
        miriam.until(f'{SHEET} === "Waiting for Mami…"')
        seat = next(id for id in held if stream_status(f"/api/stream/{id}/challenge") == "200")
        accepted = json.loads(tv.succeed(f"curl -sf -X POST {STREAM_API}/api/stream/{seat}/challenge/accept"))
        assert accepted["stage"] == "baking" and len(accepted["golden_ms"]) == 3, accepted
        miriam.until('!!document.querySelector(".pp-bake-oven")', 5000)
        miriam.shot("oven")
        press_perfectly("", f"/api/stream/{seat}/challenge/presses")
        miriam.until(f'{SHEET} === "Mami bakes better" && !!document.querySelector(".pp-bake-table")', 25000)
        miriam.shot("bake-off-lost")
        miriam.page.locator(".pp-bake .pp-pill", has_text="Close").tap()
        miriam.until('!document.querySelector(".pp")')
        miriam.page.locator("main button.play[data-play-movie]").tap()
        miriam.until(f'{SHEET} === "Der Ofen ist voll"')
        rest = miriam.js('document.querySelector(".pp-bake-ask").textContent')
        assert "Your dough is still resting" in rest, rest
        assert miriam.js('document.querySelectorAll(".pp-bake button.pp-bake-row").length') == 0, "a resting baker can still pick someone"
        miriam.page.locator(".pp-bake .pp-pill", has_text="Close").tap()
        miriam.until('!document.querySelector(".pp")')
    finally:
        let_go(held)

with scenario("a phone that is challenged while it plays pauses, bakes and hands over the place"):
    held = hold_places(mami, 2, "mami-again")
    try:
        watch_here(miriam)
        miriam.until(PLAYS, 30000)
        full = open_stream(ramu, "ramu-phone", expect="")
        seat = next(watcher["seat"]["id"] for watcher in full["full"]["watchers"] if watcher["person"] == "Miriam")
        made = json.loads(stream_call(ramu, "POST", "/api/challenges", {"seat": seat, "device": {"id": "ramu-phone", "name": "Ramus phone"}}))
        miriam.until(f'{SHEET} === "Bake-off!" && {VIDEO}.paused', 8000)
        miriam.shot("challenged")
        miriam.page.locator(".pp-bake .pp-pill", has_text="Bake").tap()
        miriam.until('!!document.querySelector(".pp-bake-oven")', 5000)
        press_perfectly(ramu, f"/api/challenges/{made['id']}/presses")
        miriam.until(f'{SHEET} === "Ramu bakes better"', 25000)
        assert stream_status(f"/api/stream/{seat}/main.m3u8") == "404", "the stream that lost still plays"
        done = json.loads(stream_call(ramu, "GET", f"/api/challenges/{made['id']}"))
        assert (done["winner"], done["ending"]) == ("challenger", "baked"), done
        assert done["baked"][1]["crust"] == [0, 0, 0], done
        held.append(open_stream(ramu, "ramu-phone")["id"])
        miriam.page.locator(".pp-bake .pp-pill", has_text="Close").tap()
        miriam.until('!document.querySelector(".pp")')
    finally:
        let_go(held)
        miriam.close()
