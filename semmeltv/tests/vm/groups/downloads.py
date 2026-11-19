with scenario("an admin downloads a season the library lacks from the show page, through the usual proposal"):
    phone.goto(f"/shows/{SHOW}")
    phone.until('[...document.querySelectorAll("main .season-downloads button")].map((button) => button.textContent.trim()).join("|") === "Download season 2"')
    assert phone.js('document.querySelector("main .season-downloads").getBoundingClientRect().top > document.querySelector("main .detail-episodes").getBoundingClientRect().bottom'), "the season buttons come after the episodes"
    assert phone.js('document.querySelector("main .season-downloads").textContent.includes("Season 3 is downloading")'), "a season already downloading has no button"
    button = phone.page.locator("main .season-downloads button", has_text="Download season 2")
    button.scroll_into_view_if_needed()
    button.tap()
    phone.until('!!document.querySelector("main .season-downloads .download-choice.best")')
    assert phone.js('document.querySelector("main .season-downloads .settings h2")?.textContent === "Testserie season 2"'), "the proposal names the season"
    phone.page.locator("main .season-downloads .download-choice.best button").tap()
    phone.until('document.querySelector("main .season-downloads .download-choice .controls-said")?.textContent === "Downloading Testserie season 2"')
    phone.shot("season-download")
    phone.page.reload(wait_until="commit")
    phone.until('!!document.querySelector("main .episode-row")')
    phone.until('document.querySelector("main .season-downloads")?.textContent.includes("Season 2 is downloading")')
    assert phone.js('!document.querySelector("main .season-downloads button")'), "no button once every missing season is downloading"
    phone.goto("/downloads")
    phone.until('[...document.querySelectorAll("main .download-job button")].some((button) => button.textContent.includes("Stop"))')

with scenario("the search box offers to download what the library lacks"):
    phone.goto("/")
    at_home(phone)
    assert phone.js('document.querySelector("main .search-download").hidden'), "no download offer before typing"
    phone.page.fill("main .search input", "Testserie season 2")
    phone.until('!document.querySelector("main .search-download").hidden')
    assert phone.js("""!!document.querySelector('main .search-download a[href="/downloads"]')"""), "the offer links to Downloads"
    phone.page.locator("main .search-download-actions button").tap()
    phone.until('!!document.querySelector("main .search-download .download-choice.best")')
    phone.shot("search-download")
    phone.page.fill("main .search input", "")

with scenario("a title typed without its season lists the namesakes and the seasons, and a tap searches that one"):
    phone.goto("/")
    at_home(phone)
    phone.page.fill("main .search input", "Testserie")
    phone.until('!document.querySelector("main .search-download").hidden')
    phone.page.locator("main .search-download-actions button").tap()
    phone.until('[...document.querySelectorAll("main .search-download .tracks button")].map((button) => button.textContent.trim()).join("|") === "Season 2|Season 4|Testserie (2001, film)"')
    phone.page.locator("main .search-download .tracks button", has_text="Season 4").tap()
    phone.until('!!document.querySelector("main .search-download .download-choice.best")')
    phone.page.fill("main .search input", "")

with scenario("typing a title lists TMDB's films and series, and tapping a series asks its season"):
    phone.goto("/")
    at_home(phone)
    phone.page.fill("main .search input", "Testserie")
    phone.until('[...document.querySelectorAll("main .search-download .download-match")].map((button) => [...button.querySelectorAll(".download-match-text > *")].map((part) => part.textContent.trim()).join(" / ")).join("|") === "Testserie / Series · 2020 · in the library|Testserie / Film · 2001"')
    phone.shot("download-matches")
    phone.page.locator("main .search-download .download-match", has_text="Series").tap()
    phone.until('[...document.querySelectorAll("main .search-download .tracks button")].map((button) => button.textContent.trim()).join("|") === "Season 2|Season 4"')
    phone.page.locator("main .search-download .tracks button", has_text="Season 4").tap()
    phone.until('!!document.querySelector("main .search-download .download-choice.best")')
    phone.page.fill("main .search input", "")
    phone.goto("/downloads")
    phone.until(f'location.pathname === "/downloads" && !!document.querySelector("main .download-job") && {LOADED}')
    phone.page.wait_for_timeout(500)
    phone.page.fill("main .download-search input", "Testserie")
    phone.until('document.querySelectorAll("main .download-match").length === 2')

with scenario("typing a director's name finds their films in the library and on TMDB, and says why"):
    phone.goto("/")
    at_home(phone)
    phone.page.fill("main .search input", "Regie")
    phone.until('[...document.querySelectorAll("main .grid .tile")].map((tile) => tile.querySelector(".tile-name").textContent.trim() + " / " + tile.querySelector(".tile-why")?.textContent.trim()).join("|") === "Archiv 01 / Director: Erika Regie"')
    phone.until('[...document.querySelectorAll("main .search-download .download-match")].map((button) => [...button.querySelectorAll(".download-match-text > *")].map((part) => part.textContent.trim()).join(" / ")).join("|") === "Regiefilm / Film · 2019 · directed by Erika Regie"')
    phone.shot("search-by-person")
    phone.page.fill("main .search input", "kommissar")
    phone.until('[...document.querySelectorAll("main .grid .tile .tile-why")].map((why) => why.textContent.trim()).join("|") === "With Max Muster as Kommissar"')
    phone.page.fill("main .search input", "Archiv 01")
    phone.until('document.querySelectorAll("main .grid .tile").length >= 1 && !document.querySelector("main .grid .tile .tile-why")')
    phone.page.fill("main .search input", "")

with scenario("Storage shows pCloud's space, and an admin deletes a whole film from pCloud after a second tap"):
    phone.goto("/storage")
    numbers = 'document.querySelector("main .pcloud .storage-numbers")?.textContent.trim()'
    phone.until(f'{numbers} === "1.4 TB free of 2.2 TB"')
    assert phone.js('document.querySelector("main .storage").textContent.includes("This TV keeps no downloads")'), "a TV without downloads still shows pCloud"
    rows = "[...document.querySelectorAll('main .pcloud-titles .storage-title')]"
    phone.until(f"{rows}.length === 23")
    assert phone.js(f"{rows}.filter((row) => row.offsetParent !== null).length === 20"), "the 20 biggest titles show first"
    assert phone.js(f"{rows}[0].textContent.includes('Testserie') && {rows}[0].textContent.includes('Series · 3 episodes')"), "the biggest title leads"
    archive = phone.page.locator("main .pcloud-titles .storage-title", has_text="Archiv 01")
    archive.get_by_role("button", name="Delete", exact=True).tap()
    archive.get_by_role("button", name="Keep", exact=True).tap()
    film = phone.page.locator("main .pcloud-titles .storage-title", has_text="Kinofilm (2024)")
    film.get_by_role("button", name="Delete", exact=True).tap()
    phone.shot("storage-pcloud-confirm")
    film.get_by_role("button", name="Delete from pCloud", exact=True).tap()
    gone = f"{rows}.find((row) => row.textContent.includes('Kinofilm (2024)'))"
    phone.until(f"{gone}?.classList.contains('gone') && {gone}.querySelector('.controls-said').textContent.startsWith('Deleted Kinofilm (2024) from the library')")
    deleted = json.loads(tv.succeed("curl -sf http://127.0.0.1:8092/api/downloads/pcloud/deleted"))
    assert deleted == [{"kind": "movie", "key": "kinofilm-2024"}], f"only the confirmed film is deleted: {deleted}"
    phone.page.locator("main .pcloud-more").tap()
    phone.until(f"{rows}.every((row) => row.offsetParent !== null)")
    phone.page.reload(wait_until="commit")
    phone.until(f"{rows}.length === 22 && !{gone}")
