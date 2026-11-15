import json
import re

STREAM_API = "http://127.0.0.1:8090"
STREAM_ORIGIN = "http://localhost:8080"
STREAM_DIR = "/var/lib/semmeltv/stream"
NO_DOLBY = {"hevc": False, "eac3": False, "ac3": False}


def stream_cookie():
    cli = tv.succeed("systemctl cat cage-tty1 | grep -o '/nix/store/[^ ]*/bin/semmeltv' | head -1").strip()
    printed = tv.succeed(
        f"runuser -u semmeltv -- env SEMMELTV_DB=/var/lib/semmeltv/semmeltv.db "
        f"SEMMELTV_ORIGINS={STREAM_ORIGIN} {cli} invite Frederik"
    )
    invited = re.search(r"/invite/(\S+)", printed)
    assert invited, printed
    token = invited.group(1)
    headers = tv.succeed(
        f"curl -sf -D - -o /dev/null -X POST -H 'Origin: {STREAM_ORIGIN}' {STREAM_API}/api/auth/invite/{token}"
    )
    cookie = re.search(r"(?i)set-cookie: (semmeltv_session=[^;]+)", headers)
    assert cookie, headers
    return cookie.group(1)


def stream_call(cookie, method, path, body=None, expect="-f"):
    data = ""
    if body is not None:
        data = f"-H 'content-type: application/json' --data '{json.dumps(body)}'"
    return tv.succeed(
        f"curl -s {expect} -X {method} -H 'Origin: {STREAM_ORIGIN}' -H 'Cookie: {cookie}' {data} '{STREAM_API}{path}'"
    )


def stream_get(path):
    return tv.succeed(f"curl -sf '{STREAM_API}{path}'")


def stream_status(path):
    return tv.succeed(f"curl -s -o /dev/null -w '%{{http_code}}' '{STREAM_API}{path}'").strip()


def stream_lengths(playlist):
    return [float(line[8:].split(",")[0]) for line in playlist.splitlines() if line.startswith("#EXTINF:")]


with subtest("a phone streams a film through ffmpeg's own HLS muxer"):
    tv.wait_for_open_port(8090)
    cookie = stream_cookie()
    opened = json.loads(stream_call(cookie, "POST", "/api/stream/open", {"movie": "testfilm-2021", "client": NO_DOLBY, "start": 0}))
    base = f"/api/stream/{opened['id']}"
    assert opened["offset"] < 0.5, opened
    assert abs(opened["duration"] - 240) < 1, opened
    assert not opened["converted_audio"], "AAC is copied to every browser"
    english = [subtitle for subtitle in opened["subtitles"] if subtitle["supported"]]
    assert [subtitle["language"] for subtitle in english] == ["en"], opened["subtitles"]

    master = stream_get(f"{base}/main.m3u8")
    assert 'CODECS="avc1.' in master and "mp4a.40.2" in master and "video.m3u8" in master, master

    tv.wait_until_succeeds(f"curl -sf '{STREAM_API}{base}/video.m3u8' | grep -q EXT-X-ENDLIST", timeout=240)
    playlist = stream_get(f"{base}/video.m3u8")
    assert "#EXT-X-PLAYLIST-TYPE:EVENT" in playlist, playlist
    assert abs(sum(stream_lengths(playlist)) - 240) < 2, playlist
    assert stream_status(f"{base}/init.mp4") == "200"
    names = [line for line in playlist.splitlines() if line.endswith(".m4s")]
    assert len(names) >= 20, names
    for name in names:
        assert stream_status(f"{base}/{name}") == "200", name
    assert stream_status(f"{base}/ffmpeg.log") == "404"

    cues = json.loads(stream_get(f"{base}/subtitles/{english[0]['id']}"))
    assert cues["complete"], cues
    assert cues["cues"][0] == {"start": 0.0, "end": 4.0, "text": "Film line 1"}, cues["cues"][:2]
    assert len(cues["cues"]) == 48, len(cues["cues"])

    stream_call(cookie, "POST", f"{base}/progress", {"position": 30, "played": 30, "state": "playing"})

with subtest("a seek outside the stream opens it again from the keyframe before, with its cues at once"):
    again = json.loads(stream_call(cookie, "POST", "/api/stream/open", {
        "movie": "testfilm-2021", "client": NO_DOLBY, "start": 165, "replaces": opened["id"],
    }))
    moved = f"/api/stream/{again['id']}"
    assert stream_status(f"{base}/video.m3u8") == "404", "the replaced stream ends"
    tv.succeed(f"test ! -e {STREAM_DIR}/{opened['id']}")
    assert 150 <= again["offset"] <= 165, again
    assert again["start"] == 165, again
    playlist = stream_get(f"{moved}/video.m3u8")
    assert f"#EXT-X-START:TIME-OFFSET={165 - again['offset']:.3f},PRECISE=YES" in playlist, playlist
    wanted = '"start":165.0,"end":169.0,"text":"Film line 34"'
    tv.wait_until_succeeds(
        f"curl -sf '{STREAM_API}{moved}/subtitles/{english[0]['id']}' | grep -qF '{wanted}'",
        timeout=10,
    )

    stream_call(cookie, "DELETE", moved)
    assert stream_status(f"{moved}/video.m3u8") == "404"
    tv.succeed(f"test ! -e {STREAM_DIR}/{again['id']}")
