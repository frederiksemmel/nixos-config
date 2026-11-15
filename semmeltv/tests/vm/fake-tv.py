import json
import os
import re
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from xml.sax.saxutils import unescape

STATE = sys.argv[1]
DESCRIPTION = """<root><device><manufacturer>SemmelTV fake TV</manufacturer><serviceList>
<service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
<controlURL>/AVTransport</controlURL></service>
<service><serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
<controlURL>/RenderingControl</controlURL></service>
</serviceList></device></root>"""

lock = threading.Lock()
player = {"uri": "", "state": "NO_MEDIA_PRESENT", "position": 0.0, "duration": 0.0, "muted": False}


def clock(seconds):
    seconds = int(seconds)
    return f"{seconds // 3600}:{seconds // 60 % 60:02}:{seconds % 60:02}"


def element(body, name):
    found = re.search(f"<{name}>(.*?)</{name}>", body, re.S)
    return unescape(found.group(1)) if found else ""


def record(name, value):
    path = os.path.join(STATE, name)
    with open(path + ".new", "w") as file:
        json.dump(value, file)
    os.replace(path + ".new", path)


def probe(uri):
    result = subprocess.run(
        ["ffprobe", "-v", "error", "-show_streams", "-show_format", "-of", "json", uri],
        capture_output=True,
        text=True,
        timeout=60,
    )
    probed = json.loads(result.stdout or "null")
    record("probe.json", probed)
    duration = float(((probed or {}).get("format") or {}).get("duration") or 0)
    with lock:
        if player["uri"] == uri and duration and not player["duration"]:
            player["duration"] = duration


def play():
    while True:
        time.sleep(1)
        with lock:
            if player["state"] == "PLAYING":
                player["position"] += 1
                if player["duration"] and player["position"] >= player["duration"]:
                    player["state"] = "STOPPED"
                    player["position"] = 0
            record("player.json", player)


class Tv(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, text):
        data = text.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/xml")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self.reply(DESCRIPTION)

    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"])).decode()
        action = self.headers["SOAPAction"].strip('"').split("#")[1]
        with open(os.path.join(STATE, "actions"), "a") as log:
            log.write(f"{action} {element(body, 'CurrentURI') or element(body, 'Target')}\n")
        answer = ""
        with lock:
            if action == "SetAVTransportURI":
                player.update(uri=element(body, "CurrentURI"), state="STOPPED", position=0.0, duration=0.0)
                threading.Thread(target=probe, args=(player["uri"],), daemon=True).start()
            elif action == "Play":
                player["state"] = "PLAYING"
            elif action == "Pause":
                player["state"] = "PAUSED_PLAYBACK"
            elif action == "Stop":
                player["state"] = "STOPPED"
            elif action == "Seek":
                hours, minutes, seconds = element(body, "Target").split(":")
                player["position"] = int(hours) * 3600 + int(minutes) * 60 + float(seconds)
            elif action == "GetTransportInfo":
                answer = f"<CurrentTransportState>{player['state']}</CurrentTransportState>"
            elif action == "GetPositionInfo":
                answer = f"<TrackDuration>{clock(player['duration'])}</TrackDuration><RelTime>{clock(player['position'])}</RelTime>"
            elif action == "SetMute":
                player["muted"] = element(body, "DesiredMute") == "1"
            elif action == "GetMute":
                answer = f"<CurrentMute>{int(player['muted'])}</CurrentMute>"
            elif action == "GetVolume":
                answer = "<CurrentVolume>20</CurrentVolume>"
            record("player.json", player)
        self.reply(f"<s:Envelope><s:Body><u:{action}Response>{answer}</u:{action}Response></s:Body></s:Envelope>")


threading.Thread(target=play, daemon=True).start()
ThreadingHTTPServer(("127.0.0.1", 9197), Tv).serve_forever()
