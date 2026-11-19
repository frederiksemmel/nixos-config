for engine, device in (("chromium", "Pixel 7"), ("webkit", "iPhone 15")):
    with scenario(f"feedback from a phone lands on the TV with its screenshot and one of the TV ({engine})"):
        sender = phone if engine == "chromium" else Phone(playwright, engine, device)
        said = f"The list jumped back to the top ({engine})"
        sender.goto("/")
        sender.until('!!document.querySelector("#feedback-open")')
        sender.page.locator("#feedback-open").tap()
        sender.until('document.activeElement === document.querySelector(".feedback-panel textarea")')
        assert sender.js('parseFloat(getComputedStyle(document.querySelector(".feedback-panel textarea")).fontSize) >= 16'), "the box is 16 px, so iOS doesn't zoom"
        sender.page.locator(".feedback-panel textarea").fill(said)
        sender.page.locator(".feedback-attach input[type=file]").set_input_files(os.path.join(OUT, "tv-home.png"))
        sender.until('!document.querySelector(".feedback-shot").hidden && document.querySelector(".feedback-shot img").naturalWidth > 0')
        sender.page.locator(".feedback-send").tap()
        sender.until('document.querySelector(".feedback").hidden')
        report = eventually(
            "the report lands on the TV",
            lambda: (lambda last: last if last["message"] == said else None)(json.loads(tv.succeed(f"tail -n 1 {FEEDBACK}/index.jsonl"))),
        )
        assert report["person"]["name"] == "Frederik", report
        for picture in ("screenshot", "tv_screenshot"):
            assert report[picture], report
            tv.succeed(f"test $(stat -c %s {FEEDBACK}/{report[picture]}) -gt 5000")
        tv.copy_from_machine(f"{FEEDBACK}/{report['tv_screenshot']}", "")
        client = json.loads(tv.succeed(f"cat {FEEDBACK}/{report['id']}.json"))["client"]
        assert {"fetch", "tap"} <= {entry["kind"] for entry in client["trail"]}, client["trail"][-5:]
        if engine == "webkit":
            safari = sender.js('(navigator.userAgent.match(/Version\\/([\\d.]+)/) || [])[1]')
            assert client["agent"] == f"iOS {safari}, Safari tab", client["agent"]
        phone.goto("/feedback")
        phone.until(
            f"""[...document.querySelectorAll(".report")].some((report) =>
                  report.querySelector(".report-message")?.textContent === {said!r}
                  && [...report.querySelectorAll(".report-pictures img")].filter((img) => img.complete && img.naturalWidth > 0).length === 2)"""
        )
        if sender is not phone:
            sender.close()

with scenario("an admin and the service token set a report's status, and the page filters by it"):
    reports = [json.loads(line) for line in tv.succeed(f"tail -n 2 {FEEDBACK}/index.jsonl").splitlines()]
    by_hand, by_token = reports[-1]["id"], reports[-2]["id"]
    phone.goto("/feedback")
    row = phone.page.locator(".report").filter(has=phone.page.locator(".report-message", has_text=reports[-1]["message"]))
    assert row.locator(".report-status b").inner_text() == "New"
    row.get_by_role("button", name="Set status").tap()
    row.get_by_role("button", name="Needs review").tap()
    row.get_by_placeholder("Why it needs a person").fill("It is a design question")
    row.get_by_placeholder("What they have to decide").fill("Keep or drop the jump")
    row.get_by_role("button", name="Save").tap()
    phone.until(f"""[...document.querySelectorAll(".report")].some((report) =>
        report.querySelector(".report-copy").textContent.includes({by_hand!r})
        && report.querySelector(".report-status b").textContent === "Needs review")""")
    marked = json.loads(tv.succeed(f"cat {FEEDBACK}/{by_hand}.status.json"))
    assert marked["status"] == {"state": "needs_review", "why": "It is a design question", "decide": "Keep or drop the jump"}, marked
    assert marked["by"] == "Frederik", marked
    done = {"state": "done", "resolution": {"kind": "fixed", "commit": "abc1234", "what": "The list keeps its place"}}
    tv.succeed(f"printf '%s: %s %s\\n' Authorization Bearer {SERVICE_TOKEN} > /tmp/service-header")
    tv.succeed(
        "curl -sf -H @/tmp/service-header -H 'content-type: application/json' "
        f"-d '{json.dumps(done)}' http://127.0.0.1:8090/api/feedback/{by_token}/status"
    )
    new = json.loads(tv.succeed("curl -sf -H @/tmp/service-header 'http://127.0.0.1:8090/api/feedback?state=new'"))
    assert by_hand not in [report["id"] for report in new["reports"]] and by_token not in [report["id"] for report in new["reports"]], new
    phone.goto("/feedback")
    phone.page.locator(".report-filters").get_by_role("button", name="Done").tap()
    phone.until(f"""[...document.querySelectorAll(".report")].filter((report) => !report.hidden).every((report) =>
        report.querySelector(".report-status b").textContent.startsWith("Fixed") || report.querySelector(".report-status b").textContent.startsWith("Done"))
        && [...document.querySelectorAll(".report")].some((report) => !report.hidden && report.querySelector(".report-copy").textContent.includes({by_token!r}))""")

with scenario("Claude answers in a report's thread, and a person's comment reopens it for triage"):
    settled = json.loads(tv.succeed(f"tail -n 2 {FEEDBACK}/index.jsonl").splitlines()[0])
    tv.succeed(
        "curl -sf -H @/tmp/service-header -H 'content-type: application/json' "
        f"""-d '{{"text": "Fixed in abc1234; the list keeps its place."}}' http://127.0.0.1:8090/api/feedback/{settled['id']}/comments"""
    )
    phone.goto("/feedback")
    row = phone.page.locator(".report").filter(has=phone.page.locator(".report-message", has_text=settled["message"]))
    assert row.locator(".report-status b").inner_text().startswith("Fixed"), "Claude's comment leaves the status alone"
    assert row.locator(".comment.claude .comment-who b").inner_text() == "Claude"
    field = row.get_by_role("textbox", name="Comment")
    assert phone.js('parseFloat(getComputedStyle(document.querySelector(".report-say textarea")).fontSize) >= 16'), "16 px, so iOS doesn't zoom"
    field.fill("Still jumps on the second visit")
    row.get_by_role("button", name="Comment").tap()
    phone.until(f"""[...document.querySelectorAll(".report")].some((report) =>
        report.querySelector(".report-copy").textContent.includes({settled['id']!r})
        && report.querySelector(".report-status b").textContent === "Reopened"
        && report.querySelector(".comment.person:not([hidden]) .comment-text")?.textContent === "Still jumps on the second visit")""")
    reopened = json.loads(tv.succeed("curl -sf -H @/tmp/service-header 'http://127.0.0.1:8090/api/feedback?state=reopened'"))["reports"]
    assert [report["id"] for report in reopened] == [settled["id"]], reopened
    assert reopened[0]["status"]["was"]["resolution"]["commit"] == "abc1234", reopened[0]["status"]
    assert [(comment["author"]["kind"], comment["text"]) for comment in reopened[0]["comments"]] == [
        ("claude", "Fixed in abc1234; the list keeps its place."),
        ("person", "Still jumps on the second visit"),
    ], reopened[0]["comments"]
    phone.goto("/feedback")
    phone.until(f"""[...document.querySelectorAll(".report")].some((report) =>
        report.querySelector(".report-copy").textContent.includes({settled['id']!r})
        && [...report.querySelectorAll(".comment:not([hidden]) .comment-who b")].map((name) => name.textContent).join() === "Claude,Frederik")""")

with scenario("a comment typed before the page's scripts ran survives them and a reload"):
    draft = f'textarea[data-draft="{settled["id"]}:comment"]'
    runtime = "**/_topcoat/assets/topcoat-*.js"
    held = []
    phone.context.route(runtime, lambda route: held.append(route))
    phone.goto("/feedback")
    phone.page.locator(draft).fill("Half a thought before the TV restarts")
    assert held and phone.js("document.readyState") != "complete", "typed while Topcoat's runtime was still loading"
    phone.context.unroute(runtime)
    for route in held:
        route.continue_()
    phone.until(f"""{LOADED} && document.querySelector({draft!r}).value === 'Half a thought before the TV restarts'
        && !document.querySelector({draft!r}).form.querySelector("button[type=submit]").disabled""")
    phone.goto("/feedback")
    phone.until(f"document.querySelector({draft!r})?.value === 'Half a thought before the TV restarts'")

with scenario("a comment whose send fails stays and says so, and is forgotten only once the TV confirmed it"):
    draft = f'textarea[data-draft="{settled["id"]}:comment"]'
    text = "Half a thought before the TV restarts"
    kept = f"localStorage.getItem('semmeltv.draft.{settled['id']}:comment')"
    shown = f'[...document.querySelectorAll(".comment.person:not([hidden]) .comment-text")].some((said) => said.textContent === {text!r})'
    procedures = "**/_topcoat/runtime/procedures/*"
    row = phone.page.locator(".report").filter(has=phone.page.locator(".report-message", has_text=settled["message"]))
    phone.context.route(procedures, lambda route: route.abort("internetdisconnected"))
    row.get_by_role("button", name="Comment").tap()
    phone.until('[...document.querySelectorAll(".toast")].some((toast) => toast.textContent.includes("Not sent"))')
    phone.context.unroute(procedures)
    assert phone.js(f"document.querySelector({draft!r}).value") == text, "a failed send leaves the text in the field"
    assert phone.js(kept) == text, "a failed send keeps the draft"
    assert not phone.js(shown), "a failed send shows no comment"
    phone.goto("/feedback")
    phone.until(f"{LOADED} && document.querySelector({draft!r}).value === {text!r}")
    held = []
    phone.context.route(procedures, lambda route: held.append(route))
    with phone.page.expect_request(lambda request: "/_topcoat/runtime/procedures/" in request.url):
        row.get_by_role("button", name="Comment").tap()
    assert phone.js(kept) == text and held, "a send on its way keeps the draft"
    phone.context.unroute(procedures)
    for route in held:
        route.continue_()
    phone.until(f"{shown} && document.querySelector({draft!r}).value === '' && {kept} === null")
    phone.goto("/feedback")
    phone.until(LOADED)
    assert phone.js(f"document.querySelector({draft!r}).value") == "", "a sent comment is no draft"

with scenario("a review's questions are radio buttons, and Answer sends the picks as a comment"):
    asked = json.loads(tv.succeed(f"tail -n 1 {FEEDBACK}/index.jsonl"))
    review = {
        "state": "needs_review",
        "why": "A new feature",
        "decide": "Pick the two below",
        "questions": [
            {"ask": "Where do the options come from?", "options": ["Structured", "Parsed", "Buttons"], "recommended": 0},
            {"ask": "Radios or checkboxes?", "options": ["Radios", "Checkboxes"], "recommended": 0},
        ],
    }
    tv.succeed(
        "curl -sf -H @/tmp/service-header -H 'content-type: application/json' "
        f"-d '{json.dumps(review)}' http://127.0.0.1:8090/api/feedback/{asked['id']}/status"
    )
    phone.goto("/feedback")
    phone.until(LOADED)
    row = phone.page.locator(".report").filter(has=phone.page.locator(".report-copy", has_text=asked["id"]))
    assert row.locator(".report-question").count() == 2
    assert row.get_by_role("radio", name="a: Structured").is_checked(), "the recommendation is picked"
    assert row.get_by_role("radio", name="a: Radios").is_checked()
    row.get_by_role("radio", name="b: Parsed").tap()
    assert row.get_by_role("radio", name="b: Parsed").is_checked() and not row.get_by_role("radio", name="a: Structured").is_checked()
    row.get_by_role("textbox", name="Anything to add").fill("and soon")
    row.get_by_role("button", name="Answer").tap()
    said = "Where do the options come from?\n→ b: Parsed\n\nRadios or checkboxes?\n→ a: Radios\n\nand soon"
    phone.until(f"""[...document.querySelectorAll(".report")].some((report) =>
        report.querySelector(".report-copy").textContent.includes({asked['id']!r})
        && report.querySelector(".report-status b").textContent === "Reopened"
        && report.querySelector(".report-answer").hidden
        && report.querySelector(".comment.person:not([hidden]) .comment-text")?.textContent === {said!r})""")
    report = json.loads(tv.succeed(f"curl -sf -H @/tmp/service-header http://127.0.0.1:8090/api/feedback/{asked['id']}"))
    assert report["status"]["state"] == "reopened" and report["status"]["was"]["questions"][0]["options"][1] == "Parsed", report["status"]
    assert report["comments"][-1]["text"] == said, report["comments"]
