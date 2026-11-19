with scenario("a phone stream plays at its pace with subtitles at the right time"):
    phone.goto(f"/movies/{FRESH_FILM}")
    phone.until('!!document.querySelector("main button.play[data-play-movie]")')
    switch_to(phone, "phone")
    phone.page.locator("main button.play[data-play-movie]").tap()
    phone.until(PLAYS, 30000)
    began = time.monotonic()
    assert phone_time(phone) < 5, f"a film never played starts at {phone_time(phone)}"
    controls(phone)
    phone.page.locator(".pp-pill", has_text="Audio").tap()
    phone.until('!document.querySelector(".pp-sheet").hidden')
    phone.page.locator(".pp-sheet section").nth(1).locator(".pp-choice", has_text="English").tap()
    phone.until(f"({ACTIVE_CUES})().length > 0", int(max(10 - (time.monotonic() - began), 1) * 1000))
    print(f"the first subtitle showed {time.monotonic() - began:.1f} s after playback began")
    checked = 0
    first = (time.monotonic(), phone_time(phone))
    while time.monotonic() - first[0] < 12:
        at = phone_time(phone)
        cues = phone.js(ACTIVE_CUES)
        if 0.3 < at % 5 < 3.7:
            assert cues == [f"Kino line {int(at // 5) + 1}"], f"at {at:.2f} s the subtitles show {cues}"
            checked += 1
        time.sleep(0.4)
    last = (time.monotonic(), phone_time(phone))
    pace = (last[1] - first[1]) / (last[0] - first[0])
    print(f"{checked} subtitle checks, the video ran at {pace:.2f}x")
    assert checked >= 10 and 0.9 < pace < 1.1, f"{checked} checks, pace {pace}"
    phone.shot("subtitles")

stop_everything(phone)
phone.api("POST", "/api/play", {"movie": FILM})
tv_starts_at()
phone.api("POST", "/api/playback/stop", {})
tv_plays_nothing()

with scenario("semmeltv-stats on the TV counts sign-ins and lists who watched what, on the TV and the phone"):
    def stats_listed():
        usage_report = tv.succeed("semmeltv-stats 1")
        plays = usage_report.split("Plays, newest first")[-1].splitlines()
        on_phone = any("Frederik" in line and "Kinofilm" in line and " TV " not in line for line in plays)
        on_tv = any("Frederik" in line and " TV " in line and "Testfilm" in line for line in plays)
        return usage_report if on_phone and on_tv else None

    usage_report = eventually("the stats list the phone's Kinofilm and the TV's Testfilm", stats_listed, timeout=60, every=3)
    print(usage_report)
    frederik = next(line for line in usage_report.splitlines() if line.startswith("Frederik")).split()
    assert int(frederik[1]) >= 1, f"Frederik's sign-in is not counted: {frederik}"


with scenario("a phone streams a film through ffmpeg's own HLS muxer"):
    tv.wait_for_open_port(8090)
    cookie = stream_cookie()
    opened = json.loads(stream_call(cookie, "POST", "/api/stream/open", {"movie": FRESH_FILM, "client": NO_DOLBY, "start": 0}))
    base = f"/api/stream/{opened['id']}"
    assert opened["offset"] < 0.5, opened
    assert abs(opened["duration"] - 90) < 1, opened
    assert not opened["converted_audio"], "AAC is copied to every browser"
    english = [subtitle for subtitle in opened["subtitles"] if subtitle["supported"]]
    assert [subtitle["language"] for subtitle in english] == ["en"], opened["subtitles"]

    master = stream_get(f"{base}/main.m3u8")
    assert 'CODECS="avc1.' in master and "mp4a.40.2" in master and "video.m3u8" in master, master

    tv.wait_until_succeeds(f"curl -sf '{STREAM_API}{base}/video.m3u8' | grep -q EXT-X-ENDLIST", timeout=60)
    playlist = stream_get(f"{base}/video.m3u8")
    assert "#EXT-X-PLAYLIST-TYPE:EVENT" in playlist, playlist
    assert abs(sum(stream_lengths(playlist)) - 90) < 2, playlist
    assert stream_status(f"{base}/init.mp4") == "200"
    names = [line for line in playlist.splitlines() if line.endswith(".m4s")]
    assert len(names) >= 8, names
    for name in names:
        assert stream_status(f"{base}/{name}") == "200", name
    assert stream_status(f"{base}/ffmpeg.log") == "404"

    cues = json.loads(stream_get(f"{base}/subtitles/{english[0]['id']}"))
    assert cues["complete"], cues
    assert cues["cues"][0] == {"start": 0.0, "end": 4.0, "text": "Kino line 1"}, cues["cues"][:2]
    assert len(cues["cues"]) == 18, len(cues["cues"])

    stream_call(cookie, "POST", f"{base}/progress", {"position": 30, "played": 30, "state": "playing"})

with scenario("a seek outside the stream opens it again from the keyframe before, with its cues at once"):
    again = json.loads(stream_call(cookie, "POST", "/api/stream/open", {
        "movie": FRESH_FILM, "client": NO_DOLBY, "start": 75, "replaces": opened["id"],
    }))
    moved = f"/api/stream/{again['id']}"
    assert stream_status(f"{base}/video.m3u8") == "404", "the replaced stream ends"
    tv.succeed(f"test ! -e {STREAM_DIR}/{opened['id']}")
    assert 60 <= again["offset"] <= 75, again
    assert again["start"] == 75, again
    playlist = stream_get(f"{moved}/video.m3u8")
    assert f"#EXT-X-START:TIME-OFFSET={75 - again['offset']:.3f},PRECISE=YES" in playlist, playlist
    wanted = '"start":75.0,"end":79.0,"text":"Kino line 16"'
    tv.wait_until_succeeds(
        f"curl -sf '{STREAM_API}{moved}/subtitles/{english[0]['id']}' | grep -qF '{wanted}'",
        timeout=10,
    )

    stream_call(cookie, "DELETE", moved)
    assert stream_status(f"{moved}/video.m3u8") == "404"
    tv.succeed(f"test ! -e {STREAM_DIR}/{again['id']}")
