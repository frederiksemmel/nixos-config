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

const CHUNK_SECONDS = 0.1;
const QUIET_AFTER_SPEECH = 1.5;
const NOTHING_SAID = 8;
const LONGEST = 12;
const ANSWER_WAIT = 10;
const TRY_AGAIN_LATER = 1013;

const api = document.documentElement.dataset.api || "";
let context = null;
let worklet = null;
let session = null;
let state = "idle";

function elements() {
  return {
    root: document.getElementById("microphone"),
    talk: document.getElementById("microphone-talk"),
    level: document.getElementById("microphone-level"),
  };
}

function show(next) {
  state = next;
  const { root, talk, level } = elements();
  if (!root) return;
  root.dataset.state = next;
  talk?.setAttribute("aria-label", next === "listening" ? "Stop listening" : "Talk to the TV");
  talk?.setAttribute("aria-pressed", String(next === "listening"));
  if (level && next !== "listening") level.style.transform = "scale(0)";
}

function say(text) {
  window.dispatchEvent(new CustomEvent("semmeltv:toast", { detail: { text, tone: "error" } }));
}

function audioContext() {
  if (!context || context.state === "closed") {
    context = new AudioContext();
    worklet = context.audioWorklet.addModule(URL.createObjectURL(new Blob([WORKLET], { type: "text/javascript" })));
  }
  return context;
}

function socketUrl() {
  const base = api ? new URL(api, location.href) : location;
  return `${base.protocol === "https:" ? "wss" : "ws"}://${base.host}/api/microphone`;
}

function voiceState() {
  return document.querySelector(".voice")?.dataset.state || null;
}

function problem(error) {
  switch (error?.name) {
    case "NotAllowedError":
    case "SecurityError":
      return "Microphone blocked: allow it in Settings";
    case "NotFoundError":
    case "OverconstrainedError":
      return "No microphone found";
    case "NotReadableError":
      return "The microphone is busy in another app";
    default:
      return "That didn't work, try again";
  }
}

function start() {
  const audio = audioContext();
  const resumed = audio.resume();
  const current = { audio, stream: null, node: null, socket: null, timers: [], heardServer: false, chunks: 0, speech: 0, quiet: 0, floor: 300 };
  session = current;
  show("starting");
  begin(current, resumed).catch((error) => {
    if (session !== current) return;
    end(current);
    show("idle");
    say(problem(error));
  });
}

async function begin(current, resumed) {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true },
  });
  current.stream = stream;
  if (session !== current) return end(current);
  await resumed;
  await worklet;
  if (session !== current) return end(current);
  const node = new AudioWorkletNode(current.audio, "downsample");
  current.node = node;
  current.source = current.audio.createMediaStreamSource(stream);
  current.source.connect(node);
  const socket = new WebSocket(socketUrl());
  socket.binaryType = "arraybuffer";
  current.socket = socket;
  node.port.onmessage = (event) => {
    if (session !== current || state !== "listening") return;
    if (socket.readyState === WebSocket.OPEN) socket.send(event.data);
    hear(current, new Int16Array(event.data));
  };
  socket.onopen = () => {
    if (session !== current) return;
    fetch(`${api}/api/listen`, { method: "POST" }).catch(() => {});
    show("listening");
    current.timers.push(setTimeout(() => finish(current), LONGEST * 1000));
  };
  socket.onclose = (event) => {
    if (session !== current || state === "processing") return;
    const opened = state === "listening";
    end(current);
    show("idle");
    if (event.code === TRY_AGAIN_LATER) say("Someone else is talking to the TV");
    else say(opened ? "Lost the connection to the TV" : "Can't reach the TV");
  };
}

function hear(current, samples) {
  let energy = 0;
  let peak = 0;
  for (const sample of samples) {
    energy += sample * sample;
    peak = Math.max(peak, Math.abs(sample));
  }
  const level = Math.sqrt(energy / samples.length);
  const { level: ring } = elements();
  if (ring) ring.style.transform = `scale(${Math.min(1, (peak / 32768) * 3)})`;
  current.chunks += 1;
  if (level > Math.max(500, current.floor * 3)) {
    current.speech += 1;
    current.quiet = 0;
  } else {
    current.quiet += 1;
    current.floor = Math.min(current.floor * 1.02, Math.max(level, 50));
  }
  const seconds = current.chunks * CHUNK_SECONDS;
  const spoke = current.speech >= 2;
  if ((spoke && current.quiet * CHUNK_SECONDS >= QUIET_AFTER_SPEECH) || (!spoke && seconds >= NOTHING_SAID)) {
    finish(current);
  }
}

function end(current) {
  current.timers.forEach(clearTimeout);
  current.timers = [];
  current.stream?.getTracks().forEach((track) => track.stop());
  current.source?.disconnect();
  current.node?.port.close();
  current.node?.disconnect();
  if (current.socket && current.socket.readyState <= WebSocket.OPEN) current.socket.close(1000, "done");
  if (current.audio.state === "running") current.audio.suspend().catch(() => {});
}

function finish(current) {
  if (session !== current || state !== "listening") return;
  end(current);
  show("processing");
  current.timers.push(setTimeout(() => settle(current), ANSWER_WAIT * 1000));
}

function settle(current) {
  if (session !== current) return;
  end(current);
  session = null;
  show("idle");
}

function cancel(current) {
  end(current);
  session = null;
  show("idle");
}

new MutationObserver(() => {
  const { root } = elements();
  if (root && root.dataset.state !== state) root.dataset.state = state;
  const current = session;
  if (!current) return;
  const voice = voiceState();
  if (state === "listening") {
    if (voice === "listening") current.heardServer = true;
    else if (current.heardServer && voice) finish(current);
  }
  if (state === "processing" && current.heardServer && voice === "done") settle(current);
  if (state === "processing" && voice === "transcribing") current.heardServer = true;
}).observe(document.documentElement, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ["data-state"] });

document.addEventListener("click", (event) => {
  if (!event.target.closest?.("#microphone-talk")) return;
  if (state === "idle") start();
  else if (state === "listening") finish(session);
  else if (state === "starting") cancel(session);
});

document.addEventListener("visibilitychange", () => {
  if (document.hidden && session && (state === "starting" || state === "listening")) cancel(session);
});
