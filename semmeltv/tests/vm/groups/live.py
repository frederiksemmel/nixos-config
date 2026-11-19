for engine, device in (("chromium", "Pixel 7"), ("webkit", "iPhone 15")):
    with scenario(f"a page whose live connection died catches up with the TV ({engine})"):
        stale = Phone(playwright, engine, device)
        sockets: list[dict[str, bool]] = []

        def live(route, sockets=sockets):
            server = route.connect_to_server()
            state = {"dead": False}
            sockets.append(state)
            server.on_message(lambda message: None if state["dead"] else route.send(message))

        def kill(sockets=sockets):
            for state in sockets:
                state["dead"] = True

        stale.page.route_web_socket(lambda url: "/api/" not in url, live)
        TITLE = 'document.querySelector(".bar-text strong")?.textContent'
        LINE = 'document.querySelector(".bar-text span")?.textContent'
        tv_plays_film(phone)
        stale.goto("/")
        stale.until(f'{TITLE} === "Testfilm"')
        stale.page.wait_for_timeout(1500)
        kill()
        phone.api("POST", "/api/playback/stop", {})
        tv_plays_nothing()
        stale.page.wait_for_timeout(2500)
        assert stale.js(f'{TITLE} === "Testfilm"'), "while the live connection is dead, the page can't know the TV stopped"
        stale.page.evaluate(AWAY, True)
        stale.page.wait_for_timeout(4500)
        stale.page.evaluate(AWAY, False)
        stale.until(f'{LINE} === "Nothing is playing"', 6000)
        stale.until("!document.documentElement.dataset.live", 6000)
        tv_plays_film(phone)
        stale.page.reload(wait_until="commit")
        stale.until(f'{TITLE} === "Testfilm"')
        stale.page.wait_for_timeout(1500)
        kill()
        phone.api("POST", "/api/playback/stop", {})
        tv_plays_nothing()
        stale.until(f'{LINE} === "Nothing is playing"', 25000)
        stale.until("!document.documentElement.dataset.live", 6000)
        stale.close()
