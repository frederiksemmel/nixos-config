HEADLINE = 'document.querySelector("#headline")?.textContent'
LATE = '!!document.querySelector("#late") && !document.querySelector("#late").hidden && document.querySelector("#late").textContent === "Dauert länger als gedacht."'


def front_answer(path):
    request = urllib.request.Request(f"{PHONE}{path}")
    try:
        with urllib.request.urlopen(request) as response:
            return response.status, response.headers, response.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, error.headers, error.read().decode()


def maintenance():
    status, _, body = front_answer("/api/maintenance")
    assert status == 200, f"/api/maintenance answered {status}: {body}"
    return json.loads(body)


def the_guess_moves_past(promised, timeout):
    def moved():
        now = maintenance()
        return now if now["state"] == "closed" and now["until"] > promised else None

    return eventually("the guess moves past the promised time", moved, timeout=timeout, every=1)


def reopened():
    tv.succeed("semmeltv-maintenance off")
    tv.systemctl("start cage-tty1.service")
    app_restarted()


CLEANUP.insert(0, reopened)

with scenario("planned work shows the bakery's closed screen with its reason, and a new guess once it runs over"):
    tv.succeed("semmeltv-maintenance on 20s neue Streaming-Limits")
    phone.goto("/")
    phone.until(f'{HEADLINE} === "Die Backstube ist kurz zu"')
    assert "neue Streaming-Limits" in phone.js('document.querySelector("#note").textContent')
    assert phone.js('document.querySelector("#eta").textContent').startswith("Wieder da in")
    assert phone.js('document.querySelector("#late").hidden'), "the screen says it is late before it is"
    phone.shot("maintenance-planned")

    status, headers, body = front_answer("/api/playback")
    assert status == 503, f"the API answered {status} during planned work"
    retry_after = headers.get("Retry-After")
    assert retry_after and int(retry_after) > 0, f"no Retry-After: {headers}"
    assert json.loads(body)["cause"] == "planned", body

    promised = maintenance()["until"]
    later = the_guess_moves_past(promised, timeout=60)
    assert later["overdue"] and later["until"] > later["now"], later
    phone.until(LATE, 10000)
    phone.shot("maintenance-planned-overdue")

    tv.succeed("semmeltv-maintenance off")
    at_home(phone)

with scenario("a stopped core shows the closed screen at once, guesses again once it runs over, and the page comes back by itself"):
    film_page(phone)
    stopped = time.monotonic()
    tv.systemctl("stop cage-tty1.service")
    first = maintenance()
    assert first["state"] == "closed" and first["cause"] == "restarting", first

    phone.goto(f"/movies/{FILM}")
    phone.until(f'{HEADLINE} === "Der Ofen wird neu angeheizt"')
    status, _, body = front_answer("/api/library")
    assert status == 503 and json.loads(body)["cause"] == "restarting", f"{status}: {body}"
    phone.shot("maintenance-restarting")

    the_guess_moves_past(first["promised"], timeout=100)
    phone.until(LATE, 10000)
    phone.shot("maintenance-restarting-overdue")

    tv.systemctl("start cage-tty1.service")
    app_restarted()
    phone.until('!!document.querySelector("main button.play[data-play-movie]")', 60000)
    took = time.monotonic() - stopped
    assert maintenance()["state"] == "open"

    said = tv.succeed("semmeltv-maintenance status")
    found = re.search(r"about (\d+) s", said)
    assert found, f"semmeltv-maintenance status names no guess: {said}"
    guess = int(found.group(1))
    assert took - 10 <= guess <= took + 10, f"the core was away {took:.0f} s, the next restart is guessed at {guess} s"

with scenario("semmeltv-maintenance refuses a duration it cannot read"):
    refused = tv.fail("semmeltv-maintenance on soon 2>&1")
    assert "not a duration" in refused, refused
    assert maintenance()["state"] == "open"
