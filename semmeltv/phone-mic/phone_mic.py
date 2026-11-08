"""Serves a page that streams a phone's microphone into an ALSA loopback, so SemmelTV hears through it."""

import argparse
import asyncio
import logging

from aiohttp import WSMsgType, web

LOG = logging.getLogger("phone-mic")
PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>SemmelTV microphone</title>
<style>
  body { font: 18px -apple-system, system-ui, sans-serif; background: #111; color: #eee;
         display: flex; flex-direction: column; align-items: center; gap: 1.2em; padding: 2em 1em; margin: 0; }
  button { font-size: 1.4em; padding: 1em 2em; border-radius: 1em; border: 0; background: #2a6; color: #fff; width: 100%; max-width: 22em; }
  button.on { background: #c33; }
  .bar { width: 100%; max-width: 22em; height: 1em; background: #333; border-radius: .5em; overflow: hidden; }
  .level { height: 100%; width: 0; background: #6c6; transition: width 60ms; }
  p { color: #aaa; text-align: center; max-width: 22em; margin: 0; }
</style>
</head>
<body>
<h1>SemmelTV microphone</h1>
<button id="toggle">Start microphone</button>
<div class="bar"><div class="level" id="level"></div></div>
<p id="status">Not streaming</p>
<p>While streaming, say “Hey Jarvis” and then a command, or press a colour button on the TV remote first.
Keep this page open and the screen on.</p>
<script>
const WORKLET = `
class Downsample extends AudioWorkletProcessor {
  constructor() { super(); this.ratio = sampleRate / 16000; this.sum = 0; this.count = 0; this.next = this.ratio; this.position = 0; this.out = []; }
  process(inputs) {
    const input = inputs[0][0];
    if (!input) return true;
    for (let i = 0; i < input.length; i++) {
      this.sum += input[i]; this.count++; this.position++;
      if (this.position >= this.next) {
        const value = Math.max(-1, Math.min(1, this.sum / this.count));
        this.out.push(value < 0 ? value * 32768 : value * 32767);
        this.sum = 0; this.count = 0; this.next += this.ratio;
      }
    }
    if (this.out.length >= 1600) { this.port.postMessage(Int16Array.from(this.out).buffer, []); this.out = []; }
    return true;
  }
}
registerProcessor("downsample", Downsample);`;

const button = document.getElementById("toggle");
const status = document.getElementById("status");
const level = document.getElementById("level");
let running = null;

async function start() {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true },
  });
  const context = new AudioContext();
  await context.audioWorklet.addModule(URL.createObjectURL(new Blob([WORKLET], { type: "text/javascript" })));
  const source = context.createMediaStreamSource(stream);
  const node = new AudioWorkletNode(context, "downsample");
  const socket = new WebSocket(`wss://${location.host}/stream`);
  socket.binaryType = "arraybuffer";
  node.port.onmessage = (event) => {
    if (socket.readyState === WebSocket.OPEN) socket.send(event.data);
    const samples = new Int16Array(event.data);
    let peak = 0;
    for (const sample of samples) peak = Math.max(peak, Math.abs(sample));
    level.style.width = `${Math.min(100, (peak / 32768) * 300)}%`;
  };
  socket.onopen = () => { status.textContent = "Streaming to the TV"; };
  socket.onclose = () => { if (running) { stop(); status.textContent = "Connection lost"; } };
  source.connect(node);
  running = { stream, context, socket };
  button.textContent = "Stop microphone";
  button.classList.add("on");
}

function stop() {
  if (!running) return;
  running.stream.getTracks().forEach((track) => track.stop());
  running.context.close();
  running.socket.close();
  running = null;
  level.style.width = "0";
  button.textContent = "Start microphone";
  button.classList.remove("on");
  status.textContent = "Not streaming";
}

button.onclick = () => {
  if (running) { stop(); return; }
  start().catch((error) => { status.textContent = `Microphone failed: ${error.message}`; stop(); });
};
</script>
</body>
</html>
"""


async def page(request: web.Request) -> web.Response:
    return web.Response(text=PAGE, content_type="text/html")


async def stream(request: web.Request) -> web.WebSocketResponse:
    socket = web.WebSocketResponse()
    await socket.prepare(request)
    lock: asyncio.Lock = request.app["lock"]
    if lock.locked():
        await socket.close(message=b"another phone is streaming")
        return socket
    async with lock:
        LOG.info("streaming from %s", request.remote)
        player = await asyncio.create_subprocess_exec(
            request.app["aplay"], "-q", "-D", request.app["device"],
            "-t", "raw", "-f", "S16_LE", "-c", "1", "-r", "16000", "-B", "200000",
            stdin=asyncio.subprocess.PIPE,
        )
        try:
            async for message in socket:
                if message.type == WSMsgType.BINARY:
                    player.stdin.write(message.data)
                    await player.stdin.drain()
        except (BrokenPipeError, ConnectionResetError) as error:
            LOG.warning("aplay stopped: %s", error)
        finally:
            player.stdin.close()
            await player.wait()
            LOG.info("stopped streaming")
    return socket


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8095)
    parser.add_argument("--device", required=True, help="ALSA playback device of the loopback")
    parser.add_argument("--aplay", default="aplay")
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    app = web.Application()
    app["lock"] = asyncio.Lock()
    app["device"] = args.device
    app["aplay"] = args.aplay
    app.add_routes([web.get("/", page), web.get("/stream", stream)])
    web.run_app(app, host="127.0.0.1", port=args.port, print=None)


if __name__ == "__main__":
    main()
