for engine, device in (("chromium", "Pixel 7"), ("webkit", "iPhone 15")):
    with scenario(f"the remote opens, closes by a swipe without a reload, and a reload brings it back closed ({engine})"):
        remote = phone if engine == "chromium" else Phone(playwright, engine, device)
        remote.goto("/")
        switch_to(remote, "tv")
        remote.until(f'!!document.querySelector(".bar-open") && {LOADED}')
        remote.page.locator(".bar-open").tap()
        remote.until('document.documentElement.hasAttribute("data-sheet")', 3000)
        remote.until(SHEET_STILL, 3000)
        remote.page.locator(".sheet-close").tap()
        remote.until('!document.documentElement.hasAttribute("data-sheet")', 3000)
        remote.until(SHEET_STILL, 3000)
        remote.page.locator(".bar-open").tap()
        remote.until('document.documentElement.hasAttribute("data-sheet")', 3000)
        remote.page.reload(wait_until="commit")
        remote.until(f'!!document.querySelector(".bar-open") && {LOADED} && !document.documentElement.hasAttribute("data-sheet")')
        remote.page.locator(".bar-open").tap()
        remote.until('document.documentElement.hasAttribute("data-sheet")', 3000)
        remote.until(SHEET_STILL, 3000)
        remote.js("window.stillHere = true")
        grip = remote.page.locator(".sheet-top").bounding_box()
        assert grip
        x, y = grip["x"] + grip["width"] / 2, grip["y"] + grip["height"] / 2
        remote.page.mouse.move(x, y)
        remote.page.mouse.down()
        for step in range(1, 13):
            remote.page.mouse.move(x, y + 40 * step)
        remote.page.mouse.up()
        remote.until('!document.documentElement.hasAttribute("data-sheet") && window.stillHere === true', 3000)
        if remote is not phone:
            remote.close()

for engine, device in (("chromium", "Pixel 7"), ("webkit", "iPhone 15")):
    with scenario(f"a tap on the bar while the page still loads opens the remote, and it stays open once loading ends ({engine})"):
        switch_to(phone, "tv")
        early = Phone(playwright, engine, device)
        held: list[Any] = []
        holding = [True]

        def hold(route):
            kind = route.request.resource_type
            if holding[0] and (kind == "image" or (kind == "script" and "/player-" in route.request.url)):
                held.append(route)
            else:
                route.fallback()

        early.page.route("**/*", hold)
        early.goto("/")
        early.until('!!document.querySelector(".bar-open")')
        early.page.wait_for_timeout(300)
        assert not early.js("window.semmeltvSheetReady"), "the remote's script is still held back"
        early.page.locator(".bar-open").tap()
        early.page.wait_for_timeout(500)
        holding[0] = False
        for route in [route for route in held if route.request.resource_type == "script"]:
            route.continue_()
        early.until('document.documentElement.hasAttribute("data-sheet")', 5000)
        assert early.js('document.readyState !== "complete"'), "the images still hold the page's load"
        for route in [route for route in held if route.request.resource_type == "image"]:
            route.continue_()
        early.until('document.readyState === "complete"', 15000)
        early.page.wait_for_timeout(800)
        assert early.js('document.documentElement.hasAttribute("data-sheet")'), "the end of loading leaves the remote open"
        early.close()

for engine, device in (("chromium", "Pixel 7"), ("webkit", "iPhone 15")):
    with scenario(f"toasts stack and go away by a swipe, a tap or by themselves ({engine})"):
        toaster = phone if engine == "chromium" else Phone(playwright, engine, device)
        toaster.goto("/")
        at_home(toaster)
        toaster.until('document.documentElement.hasAttribute("data-phone-ready")')
        assert toaster.js('!document.getElementById("microphone-note")'), "the top bar has no microphone text"
        if engine == "chromium":
            toaster.page.locator("#microphone-talk").tap()
            toaster.until('!!document.querySelector(".toast[data-tone=error]") && document.getElementById("microphone").dataset.state === "idle"', 8000)
        toaster.js('document.querySelectorAll(".toast").forEach((toast) => toast.remove())')
        raise_toast(toaster, "first")
        raise_toast(toaster, "second")
        assert toaster.js('["backdropFilter", "webkitBackdropFilter"].every((name) => [undefined, "none"].includes(getComputedStyle(document.querySelector(".toast"))[name]))')
        toaster.until('[...document.querySelectorAll(".toast")].map((t) => t.textContent).join() === "second,first"', 2000)
        newest = toaster.page.locator(".toast").first.bounding_box()
        assert newest
        toaster.page.mouse.move(newest["x"] + 60, newest["y"] + 20)
        toaster.page.mouse.down()
        for step in range(1, 9):
            toaster.page.mouse.move(newest["x"] + 60 + 25 * step, newest["y"] + 20)
        toaster.page.mouse.up()
        toaster.until('[...document.querySelectorAll(".toast:not([data-leaving])")].map((t) => t.textContent).join() === "first"', 2000)
        toaster.page.locator(".toast:not([data-leaving])").first.click()
        toaster.until('!document.querySelector(".toast:not([data-leaving])")', 2000)
        raise_toast(toaster, "goes by itself")
        toaster.until('!document.querySelector(".toast")', 7000)
        if toaster is not phone:
            toaster.close()

with scenario("pages are revalidated, and a page from before a deploy reloads into the new build"):
    iphone = Phone(playwright, "webkit", "iPhone 15")
    try:
        iphone.goto("/")
        at_home(iphone)
        page = iphone.page.request.get(PHONE + "/")
        assert page.headers.get("cache-control") == "no-cache", page.headers
        script = iphone.js('document.querySelector("script[src*=\'/_topcoat/assets/live-\']").src')
        asset = iphone.page.request.get(script)
        assert "immutable" in asset.headers.get("cache-control", ""), asset.headers
        old_build = iphone.js("document.documentElement.dataset.build")
        iphone.js("window.oldPage = true")
        deploy_web(iphone, "next")
        goes_away(iphone, 5)
        iphone.until('document.documentElement.dataset.build === "next" && !window.oldPage && !!document.querySelector(".bar-open")')
        assert old_build != "next", old_build
        iphone.page.wait_for_timeout(2000)
        iphone.js("window.oldPage = true")
        film_page(iphone)
        deploy_web(iphone, "after-next")
        iphone.page.go_back()
        iphone.until('location.pathname === "/" && document.documentElement.dataset.build === "after-next" && !window.oldPage && !!document.querySelector(".bar-open")')
        iphone.js("window.settled = true")
        iphone.page.wait_for_timeout(3000)
        assert iphone.js("window.settled === true"), "a page of the serving build doesn't reload again"
    finally:
        deploy_web(iphone, None)
        iphone.close()
