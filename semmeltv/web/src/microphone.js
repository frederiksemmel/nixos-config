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

const api = document.documentElement.dataset.api || "";
let running = null;

function elements() {
  return {
    root: document.getElementById("microphone"),
    talk: document.getElementById("microphone-talk"),
    off: document.getElementById("microphone-off"),
    level: document.getElementById("microphone-level"),
    note: document.getElementById("microphone-note"),
  };
}

function show(state, note = "") {
  const { root, off, level, note: noteElement } = elements();
  if (!root) return;
  root.dataset.state = state;
  off.hidden = state === "off";
  noteElement.textContent = note;
  if (state === "off") level.style.transform = "scale(0)";
}

function socketUrl() {
  const base = api ? new URL(api, location.href) : location;
  const secure = base.protocol === "https:";
  return `${secure ? "wss" : "ws"}://${base.host}/api/microphone`;
}

function listen() {
  fetch(`${api}/api/listen`, { method: "POST" }).catch(() => {});
}

async function start() {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true },
  });
  const context = new AudioContext();
  await context.audioWorklet.addModule(URL.createObjectURL(new Blob([WORKLET], { type: "text/javascript" })));
  const source = context.createMediaStreamSource(stream);
  const node = new AudioWorkletNode(context, "downsample");
  const socket = new WebSocket(socketUrl());
  socket.binaryType = "arraybuffer";
  node.port.onmessage = (event) => {
    if (socket.readyState === WebSocket.OPEN) socket.send(event.data);
    const samples = new Int16Array(event.data);
    let peak = 0;
    for (const sample of samples) peak = Math.max(peak, Math.abs(sample));
    const { level } = elements();
    if (level) level.style.transform = `scale(${Math.min(1, (peak / 32768) * 3)})`;
  };
  socket.onopen = () => {
    show("on");
    listen();
  };
  socket.onclose = (event) => {
    if (!running) return;
    stop();
    show("off", event.reason || "Connection lost");
  };
  source.connect(node);
  running = { stream, context, socket };
  show("starting");
}

function stop() {
  if (!running) return;
  const current = running;
  running = null;
  current.stream.getTracks().forEach((track) => track.stop());
  current.context.close();
  current.socket.close();
  show("off");
}

document.addEventListener("click", (event) => {
  if (event.target.closest("#microphone-off")) {
    stop();
    return;
  }
  if (!event.target.closest("#microphone-talk")) return;
  if (running) {
    listen();
    return;
  }
  start().catch((error) => {
    stop();
    show("off", `Microphone failed: ${error.message}`);
  });
});

document.addEventListener("visibilitychange", () => {
  if (running && running.context.state === "suspended") running.context.resume();
});
