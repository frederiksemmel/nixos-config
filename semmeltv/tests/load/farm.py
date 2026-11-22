"""Phone streams without the app's limit, for load tests on a TV.

Runs on the TV as the semmeltv user. POST /open {"file", "start", "client"}
starts ffmpeg with the arguments of semmeltv/app/src/stream/remux.rs and
answers once three segments are on disk, like /api/stream/open. The stream's
files are served under /s/ID/, and DELETE /s/ID stops it.

    sudo -u semmeltv env PATH=FFMPEG_BIN:$PATH python3 farm.py --listen 100.x.y.z:18555 --dir /var/lib/semmeltv/loadtest
"""

import argparse
import json
import os
import secrets
import shutil
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

STEREO = "aformat=channel_layouts=stereo,alimiter=limit=-1dB:level=false"
SURROUND = "aformat=channel_layouts=5.1,alimiter=limit=-1dB:level=false"
READY_SEGMENTS = 3
BEHIND = 90.0
AHEAD = 180.0
SLACK = 30.0

streams = {}
fetched = {}
lock = threading.Lock()


def keep(id, out, process, window):
    """The app's keep window (stream/remux.rs): segments from BEHIND seconds
    before the newest one fetched, and ffmpeg stopped while it is more than
    AHEAD seconds past it."""
    dropped, held = 0, False
    while id in streams:
        time.sleep(1)
        try:
            lengths = [float(line[8:].split(",")[0]) for line in (out / "video.m3u8").read_text().splitlines() if line.startswith("#EXTINF:")]
        except OSError:
            continue
        if not lengths:
            continue
        newest = min(fetched.get(id, 0), len(lengths) - 1)
        ahead = sum(lengths[newest + 1 :])
        running = process.poll() is None
        if not held and running and ahead > AHEAD:
            process.send_signal(19)
            held = True
        elif held and (ahead < AHEAD - SLACK or not running):
            process.send_signal(18)
            held = False
        start = lambda index: sum(lengths[:index])
        keep_from = next((index for index in range(dropped, newest) if start(newest) - start(index + 1) < window), newest)
        for index in range(dropped, max(keep_from, dropped)):
            (out / f"{index}.m4s").unlink(missing_ok=True)
        dropped = max(keep_from, dropped)


def probe(path):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-show_streams", "-show_format", "-of", "json", path],
        capture_output=True,
        check=True,
        text=True,
    ).stdout
    data = json.loads(out)
    video = next(s for s in data["streams"] if s["codec_type"] == "video")
    audio = [s for s in data["streams"] if s["codec_type"] == "audio"]
    chosen = next((s for s in audio if s.get("disposition", {}).get("default")), audio[0] if audio else None)
    return video, chosen, float(data["format"].get("bit_rate", 0))


def arguments(path, out, start, client):
    video, audio, bitrate = probe(path)
    hevc = video["codec_name"] == "hevc"
    copy_audio = audio is not None and (
        audio["codec_name"] == "aac"
        or (audio["codec_name"] == "eac3" and client.get("eac3"))
        or (audio["codec_name"] == "ac3" and client.get("ac3"))
    )
    surround = audio is not None and audio.get("channels", 2) > 2
    args = ["ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error"]
    if start > 0:
        args += ["-ss", f"{start:.3f}", "-noaccurate_seek"]
    args += ["-copyts", "-readrate", "2", "-readrate_initial_burst", "60", "-i", path, "-map", "0:v:0"]
    if audio is not None:
        args += ["-map", f"0:{audio['index']}"]
    args += ["-c:v", "copy"]
    if hevc:
        args += ["-tag:v", "hvc1"]
    if copy_audio or audio is None:
        args += ["-c:a", "copy"]
        sound = "copied"
    elif surround:
        args += ["-af", SURROUND, "-c:a", "aac", "-b:a", "384k"]
        sound = "aac 5.1"
    else:
        args += ["-af", STEREO, "-c:a", "aac", "-b:a", "256k"]
        sound = "aac stereo"
    args += [
        "-sn", "-dn", "-max_muxing_queue_size", "4096",
        "-f", "hls", "-hls_time", "6", "-hls_list_size", "0",
        "-hls_playlist_type", "event", "-hls_segment_type", "fmp4",
        "-hls_flags", "independent_segments+temp_file",
        "-hls_fmp4_init_filename", "init.mp4",
        "-hls_segment_filename", str(out / "%d.m4s"), str(out / "video.m3u8"),
    ]
    return args, {"bitrate": bitrate, "sound": sound, "video": video["codec_name"]}


def segments(text):
    return sum(1 for line in text.splitlines() if line.endswith(".m4s") and not line.startswith("#"))


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    root: Path
    windowed = False

    def log_message(self, *_):
        pass

    def reply(self, status, body, kind="application/json"):
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        asked = json.loads(self.rfile.read(length) or b"{}")
        id = secrets.token_hex(8)
        out = self.root / id
        out.mkdir(parents=True)
        args, facts = arguments(asked["file"], out, float(asked.get("start", 0)), asked.get("client", {}))
        process = subprocess.Popen(
            args,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=open(out / "ffmpeg.log", "w"),
            preexec_fn=lambda: os.nice(10),
        )
        with lock:
            streams[id] = process
        if self.windowed:
            threading.Thread(target=keep, args=(id, out, process, BEHIND), daemon=True).start()
        began = time.monotonic()
        while time.monotonic() - began < 45:
            listed = (out / "video.m3u8").read_text() if (out / "video.m3u8").exists() else ""
            if segments(listed) >= READY_SEGMENTS or (process.poll() == 0 and segments(listed) > 0):
                break
            if process.poll() not in (None, 0):
                break
            time.sleep(0.1)
        body = {"id": id, "playlist": f"/s/{id}/video.m3u8", "pid": process.pid, **facts}
        self.reply(200, json.dumps(body).encode())

    def do_GET(self):
        parts = self.path.strip("/").split("/")
        if len(parts) != 3 or parts[0] != "s" or "/" in parts[2] or parts[2].startswith("."):
            return self.reply(404, b"{}")
        if parts[2].endswith(".m4s") and parts[2][:-4].isdigit():
            fetched[parts[1]] = max(fetched.get(parts[1], 0), int(parts[2][:-4]))
        file = self.root / parts[1] / parts[2]
        try:
            body = file.read_bytes()
        except OSError:
            return self.reply(404, b"{}")
        kind = "application/vnd.apple.mpegurl" if file.suffix == ".m3u8" else "video/mp4"
        self.reply(200, body, kind)

    def do_DELETE(self):
        parts = self.path.strip("/").split("/")
        with lock:
            process = streams.pop(parts[-1], None)
        if process:
            process.kill()
            process.wait()
            shutil.rmtree(self.root / parts[-1], ignore_errors=True)
        self.reply(204, b"")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", action="append", required=True)
    parser.add_argument("--dir", required=True)
    parser.add_argument("--window", action="store_true", help="keep only the app's window of segments")
    args = parser.parse_args()
    root = Path(args.dir)
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True)
    Handler.root = root
    Handler.windowed = args.window
    servers = []
    for address in args.listen:
        host, port = address.rsplit(":", 1)
        server = ThreadingHTTPServer((host, int(port)), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        servers.append(server)
    try:
        threading.Event().wait()
    finally:
        for process in streams.values():
            process.kill()
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
