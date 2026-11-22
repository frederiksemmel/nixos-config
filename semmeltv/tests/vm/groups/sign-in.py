with scenario("an invite link signs a phone in, makes it set up a sign-in, and Back never returns to it"):
    guest = Phone(playwright, who=None)
    guest.goto("/invite/made-up")
    guest.until('(document.querySelector("main")?.textContent || "").includes("not valid")')
    phone.goto("/")
    phone.until('location.pathname === "/"')
    assert phone.api("POST", "/api/auth/members", {"name": "Gast", "role": "viewer"})["status"] == 200
    token = invite("Gast")
    guest.goto(f"/invite/{token}")
    guest.until('!!document.querySelector("#redeem") && document.querySelector("main h1").textContent.includes("Gast")')
    guest.page.locator("#redeem").tap()
    guest.until('location.pathname === "/welcome" && document.querySelector("#welcome-passkey")?.hidden === false && !!document.querySelector("#password-set") && !document.querySelector("#welcome-done")')
    assert guest.api("GET", "/api/auth/me")["body"]["person"]["name"] == "Gast"
    guest.page.go_back(wait_until="commit")
    guest.until('location.pathname === "/profiles" && !!document.querySelector(".profile-choice") && !document.querySelector("#redeem")')
    guest.goto("/")
    guest.until('location.pathname === "/welcome"')
    guest.goto(f"/invite/{token}")
    guest.until('location.pathname === "/welcome"')
    guest.page.fill("#password-set input[name=password]", "guest-pass-1")
    guest.page.locator("#password-set [type=submit]").tap()
    guest.until('!!document.querySelector("#welcome-done")')
    guest.page.locator("#welcome-done").tap()
    guest.until('location.pathname === "/" && !!document.querySelector(".profile-link")')
    guest.shot("signed-in")
    guest.close()

with scenario("signed-out visitors land on the picker and sign in with a password"):
    assert phone.api("POST", "/api/auth/account/password", {"password": "frederik-pass"})["status"] == 200
    visitor = Phone(playwright, who=None)
    visitor.goto("/")
    assert visitor.api("GET", "/api/library")["status"] == 401
    visitor.goto(f"/movies/{FILM}")
    visitor.until('location.pathname === "/profiles"')
    assert visitor.js('!document.querySelector(".bar-open")'), "a signed-out visitor gets no player or remote"
    visitor.until('[...document.querySelectorAll(".profile-choice")].some((choice) => choice.textContent.includes("Ramu") && choice.textContent.includes("Not set up yet"))')
    visitor.page.locator(".profile-choice", has_text="Ramu").click()
    visitor.until('document.querySelector("#sign-in-note").textContent.includes("invite link")', 5000)
    password_sign_in(visitor, "Frederik", "wrong-pass")
    visitor.until('document.querySelector("#sign-in-note").textContent.toLowerCase().includes("wrong")', 5000)
    visitor.page.fill("#password-input", "frederik-pass")
    visitor.page.locator("#password-form button[type=submit]").click()
    visitor.until(f'location.pathname === "/movies/{FILM}"')
    visitor.close()

watched_a_while(phone)
tv.succeed(
    "curl -sf -H 'content-type: application/json' "
    """-d '{"proposal": "season-2", "choice": 0, "by": "Frederik"}' http://127.0.0.1:8092/api/downloads/confirm"""
)

with scenario("profiles on one phone: each has its own Home, a viewer is limited, and switching back to the admin asks again"):
    phone.goto("/")
    phone.until(WATCHING)
    assert phone.js(AVATAR) == "F"
    phone.goto(f"/invite/{invite('Mami')}")
    phone.page.locator("#redeem").tap()
    phone.until('location.pathname === "/welcome" && !!document.querySelector("#password-set")')
    phone.page.fill("#password-set input[name=password]", "mami-pass-1")
    phone.page.locator("#password-set [type=submit]").tap()
    phone.until('!!document.querySelector("#welcome-done")')
    phone.goto("/")
    phone.until(f'{AVATAR} === "M"')
    phone.page.wait_for_timeout(800)
    assert phone.js(f"!({WATCHING})"), "Mami's Home has no Currently Watching of her own"
    phone.goto("/settings")
    phone.until('!!document.querySelector("main .settings")')
    assert phone.js("""!document.querySelector('main a[href="/storage"], main a[href="/downloads"], [data-post="/api/tv/pair"]')"""), "a viewer has no Storage, Downloads or TV pairing"
    assert phone.js("""!document.querySelector('[data-post="/api/tv-profile"], #mode, .player')"""), "a viewer without TV control sees no TV controls"
    assert phone.api("POST", "/api/play", {"movie": FILM})["status"] == 403, "a viewer without TV control can't play on the TV"
    flip = {"action": "flip", "to": "tv", "device": {"id": "vm-mami", "name": "phone"}}
    assert phone.api("POST", "/api/screen", flip)["status"] == 403, "nor move what plays to the TV"
    assert phone.api("POST", "/api/audio/pair")["status"] == 403, "a viewer without TV control can't pair headphones"
    phone.goto(f"/shows/{SHOW}")
    phone.until('!!document.querySelector("main .episode-row")')
    assert phone.js('!document.querySelector("main .season-downloads")'), "a viewer is offered no season downloads"
    phone.goto("/storage")
    phone.until('location.pathname === "/"')
    phone.goto("/downloads")
    phone.until('location.pathname === "/"')
    phone.page.locator(".profile-link").tap()
    phone.until('location.pathname === "/profiles" && document.querySelector(".profile-choice.active .profile-name")?.textContent === "Mami"')
    centred(phone, "the signed-in picker")
    password_sign_in(phone, "Frederik", "frederik-pass")
    phone.until(f'location.pathname === "/" && {AVATAR} === "F"')
    phone.until(WATCHING)
    phone.goto("/people")
    mami_row = '[...document.querySelectorAll(".member")].find((row) => row.textContent.includes("Mami"))'
    phone.until(f'{mami_row}?.textContent.includes("can\'t control the TV")')
    assert phone.js('!document.querySelector(\'[data-capability$="/headphones"]\')'), "headphones are no capability of their own"
    phone.page.locator(".member", has_text="Mami").locator('[data-capability$="/control-tv"]').tap()
    phone.until(f'{mami_row}.textContent.includes("controls the TV")')
    phone.goto("/profiles")
    phone.until('[...document.querySelectorAll(".profile-choice")].some((choice) => choice.textContent.includes("Mami") && choice.textContent.includes("On this device"))')
    phone.page.locator(".profile-choice", has_text="Mami").tap()
    phone.until(f'location.pathname === "/" && {AVATAR} === "M"')
    phone.goto("/settings")
    phone.until('!!document.querySelector("#mode") && !!document.querySelector(\'[data-post="/api/tv-profile"]\')')
    assert phone.api("POST", "/api/audio/pair")["status"] == 404, "TV control brings her own headphones with it: only this TV's lack of Bluetooth refuses the pairing"
    phone.goto("/profiles")
    password_sign_in(phone, "Frederik", "frederik-pass")
    phone.until(f'location.pathname === "/" && {AVATAR} === "F"')
    phone.goto("/people")
    phone.until(f'{mami_row}?.textContent.includes("no downloads")')
    phone.page.locator(".member", has_text="Mami").locator('[data-capability$="/download"]').tap()
    phone.until(f'{mami_row}?.textContent.includes("downloads") && !{mami_row}?.textContent.includes("no downloads")')
    phone.goto("/profiles")
    phone.until('[...document.querySelectorAll(".profile-choice")].some((choice) => choice.textContent.includes("Mami") && choice.textContent.includes("On this device"))')
    phone.page.locator(".profile-choice", has_text="Mami").tap()
    phone.until(f'location.pathname === "/" && {AVATAR} === "M"')
    phone.goto("/settings")
    phone.until("""!!document.querySelector('main a[href="/downloads"]')""")
    assert phone.js("""!document.querySelector('main a[href="/storage"], main a[href="/feedback"]')"""), "downloading brings no Storage or Reports with it"
    phone.goto(f"/shows/{SHOW}")
    phone.until('!!document.querySelector("main .season-downloads")')
    phone.goto("/downloads")
    phone.until('location.pathname === "/downloads" && !!document.querySelector("main .download-job")')
    assert phone.js('![...document.querySelectorAll("main .download-job button")].some((button) => button.textContent.includes("Stop"))'), "only an admin stops a download"
    phone.goto("/profiles")
    password_sign_in(phone, "Frederik", "frederik-pass")
    phone.until(f'location.pathname === "/" && {AVATAR} === "F"')
