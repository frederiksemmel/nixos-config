"""Phone streams while the TV itself plays, in one go.

Runs on the TV as the semmeltv user. Starts FILM on the TV as the signed-in
test profile, mutes mpv the moment its socket answers, then runs phases of
farm.py viewers next to it and prints the player's health lines for each
phase. It stops the film at the end, and gives up (without touching the
TV) as soon as mpv plays anything else.

    sudo -u semmeltv python3 tv.py --cookie TOKEN --film KEY --file PATH... --phases 0:60,1:150,3:150,6:150
"""

import argparse
import json
import os
import socket
import subprocess
import sys
import threading
import time
import urllib.request

API = "http://127.0.0.1:8090"
MPV = "/run/user/1001/semmeltv-mpv.sock"
HERE = os.path.dirname(os.path.abspath(__file__))


def say(**fields):
    print(json.dumps({"t": round(time.time()), **fields}), flush=True)


def api(cookie, method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(API + path, data=data, method=method)
    request.add_header("Cookie", f"semmeltv_session={cookie}")
    if data:
        request.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status, response.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode()


def mpv(*command):
    try:
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(1)
            connection.connect(MPV)
            connection.sendall(json.dumps({"command": list(command)}).encode() + b"\n")
            buffer = b""
            while True:
                buffer += connection.recv(65536)
                for line in buffer.split(b"\n"):
                    if line.strip():
                        answer = json.loads(line)
                        if "error" in answer:
                            return answer
    except (OSError, ValueError):
        return None


def running():
    return subprocess.run(["pgrep", "-x", "mpv"], capture_output=True).returncode == 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cookie", required=True)
    parser.add_argument("--film", required=True)
    parser.add_argument("--match", required=True, help="a word in the film's path, to tell it is still ours")
    parser.add_argument("--file", action="append", required=True)
    parser.add_argument("--phases", default="0:60,1:150,3:150,6:150")
    args = parser.parse_args()
    if running():
        say(abort="mpv already runs; the TV is not free")
        sys.exit(2)
    began = time.time()
    muted = threading.Event()
    stop = threading.Event()

    def mute():
        while not stop.is_set():
            if running() and os.path.exists(MPV):
                answer = mpv("set_property", "mute", True)
                if answer and answer.get("error") == "success":
                    check = mpv("get_property", "mute")
                    if check and check.get("data") is True:
                        if not muted.is_set():
                            say(muted=round(time.time() - began, 2))
                        muted.set()
            time.sleep(0.02 if not muted.is_set() else 1)

    threading.Thread(target=mute, daemon=True).start()
    sampler = subprocess.Popen(
        [sys.executable, os.path.join(HERE, "sample.py"), "--seconds", "3600", "--interval", "5"],
        stdout=subprocess.PIPE,
        text=True,
    )
    threading.Thread(target=lambda: [say(sample=json.loads(line)) for line in sampler.stdout], daemon=True).start()
    status, answer = api(args.cookie, "POST", "/api/play", {"movie": args.film})
    say(play=status, answer=answer[:200])
    if not muted.wait(15):
        say(abort="mpv did not mute in 15 s; stopping")
        api(args.cookie, "POST", "/api/playback/stop")
        stop.set()
        sampler.kill()
        sys.exit(3)
    time.sleep(10)
    ours = True

    def still_ours():
        path = mpv("get_property", "path")
        return bool(path and args.match in str(path.get("data", "")))

    for phase in args.phases.split(","):
        viewers, seconds = (int(x) for x in phase.split(":"))
        if not still_ours():
            ours = False
            say(abort="mpv plays something else now; leaving the TV alone")
            break
        say(phase=viewers, seconds=seconds, position=(mpv("get_property", "time-pos") or {}).get("data"))
        if viewers:
            command = [sys.executable, os.path.join(HERE, "viewers.py"), "--farm", "http://127.0.0.1:18555",
                       "-n", str(viewers), "--seconds", str(seconds), "--label", f"tv+{viewers}"]
            for file in args.file:
                command += ["--file", file]
            run = subprocess.Popen(command, stdout=subprocess.PIPE, text=True)
            deadline = time.time() + seconds + 60
            while run.poll() is None and time.time() < deadline:
                time.sleep(2)
                if not still_ours():
                    run.kill()
                    break
            for line in run.stdout:
                say(viewer=json.loads(line))
        else:
            end = time.time() + seconds
            while time.time() < end and still_ours():
                time.sleep(2)
        drops = mpv("get_property", "frame-drop-count")
        decoder = mpv("get_property", "decoder-frame-drop-count")
        delayed = mpv("get_property", "vo-delayed-frame-count")
        say(
            phase_end=viewers,
            frame_drop_count=(drops or {}).get("data"),
            decoder_drop_count=(decoder or {}).get("data"),
            delayed_count=(delayed or {}).get("data"),
        )
    if ours and still_ours():
        status, _ = api(args.cookie, "POST", "/api/playback/stop")
        say(stopped=status)
    stop.set()
    time.sleep(6)
    sampler.kill()
    status, screen = api(args.cookie, "GET", "/api/screen")
    say(screen=screen[:300])
    journal = subprocess.run(
        ["journalctl", "-t", "cage", "--since", f"@{int(began)}", "-o", "short-unix", "--no-pager"],
        capture_output=True,
        text=True,
    ).stdout
    for line in journal.splitlines():
        if " player: " in line or " display: " in line:
            say(journal=line)


if __name__ == "__main__":
    main()
