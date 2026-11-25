const api = document.documentElement.dataset.api || "";
const HLS_JS = "https://cdn.jsdelivr.net/npm/hls.js@1.5.17/dist/hls.min.js";
const REPORT_EVERY = 10000;
const CUES_EVERY = 20000;
const HIDE_AFTER = 3200;
const HIDE_AFTER_MOUSE = 1500;
const RESUME_KEY = "semmeltv.resume";
const RESUME_WITHIN = 15000;
const GIVE_UP = 120000;
const FIRST_RETRY = 1000;
const LONGEST_RETRY = 8000;
const COUNTDOWN = 10;
const JUMP = 10;
const FLICK = 0.5;
const START_DRAG = 8;
const SETTLE_MS = 320;
const WHEEL_IDLE = 140;
const WHEEL_FRAME = 16;

const HOVER_KEEPS = ".pp-top, .pp-middle button, .pp-bottom, .pp-skip, .pp-upnext, .pp-sheet";

const icons = {
  close: '<svg viewBox="0 0 24 24"><path d="m6 9 6 6 6-6"/></svg>',
  play: '<svg viewBox="0 0 24 24"><path class="fill" d="M8 5.5v13l10.5-6.5z"/></svg>',
  pause: '<svg viewBox="0 0 24 24"><path class="fill" d="M7 5h3.5v14H7zM13.5 5H17v14h-3.5z"/></svg>',
  back: '<svg viewBox="0 0 24 24"><path d="M11 5.5 4.5 12l6.5 6.5"/><path d="M19.5 5.5 13 12l6.5 6.5"/></svg>',
  ahead: '<svg viewBox="0 0 24 24"><path d="M13 5.5 19.5 12 13 18.5"/><path d="M4.5 5.5 11 12l-6.5 6.5"/></svg>',
  tracks: '<svg viewBox="0 0 24 24"><rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="M7 11h4M13 11h4M7 15h7"/></svg>',
  next: '<svg viewBox="0 0 24 24"><path class="fill" d="M5.5 5.5v13l9-6.5z"/><path d="M18 5.5v13"/></svg>',
  tv: '<svg viewBox="0 0 24 24"><rect x="2.5" y="4.5" width="19" height="12.5" rx="2"/><path d="M8.5 20h7"/></svg>',
  full: '<svg viewBox="0 0 24 24"><path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/></svg>',
  pip: '<svg viewBox="0 0 24 24"><rect x="2.5" y="4.5" width="19" height="15" rx="2"/><rect class="fill" x="12" y="12" width="7" height="5" rx="1"/></svg>',
  stop: '<svg viewBox="0 0 24 24"><path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/></svg>',
  airplay: '<svg viewBox="0 0 24 24"><path d="M6 17H4.5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h15a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H18"/><path class="fill" d="m12 14 5 6H7z"/></svg>',
};

const html = document.documentElement;
const DEVICE_KEY = "semmeltv.device";
const RECONNECT = 2000;
const controlsTv = html.dataset.controlsTv === "true";
let player = null;
let screen = { kind: "tv", device: null, play: null, notice: null };
let seenNotice = undefined;
let socket = null;
let pending = null;
const PAGE = Math.random().toString(36).slice(2);

// The player's history entry, made by this page load: one left by a load
// before a reload or a back/forward restore is someone else's.
function ownEntry() {
  return history.state?.player === PAGE;
}

function pushEntry() {
  history.pushState({ ...(history.state || {}), player: PAGE }, "");
}
let boot = null;
let reloaded = takeResume();

function takeResume() {
  try {
    const saved = JSON.parse(sessionStorage.getItem(RESUME_KEY) || "null");
    sessionStorage.removeItem(RESUME_KEY);
    return saved && Date.now() - saved.at < RESUME_WITHIN ? saved : null;
  } catch {
    return null;
  }
}

function stored(key) {
  try { return localStorage.getItem(key); } catch { return null; }
}

function store(key, value) {
  try { localStorage.setItem(key, value); } catch {}
}

function element(tag, className, attributes = {}) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  for (const [name, value] of Object.entries(attributes)) {
    if (name === "html") node.innerHTML = value;
    else if (name === "text") node.textContent = value;
    else node.setAttribute(name, value);
  }
  return node;
}

function button(className, label, icon) {
  return element("button", className, { type: "button", "aria-label": label, html: icon });
}

function clock(seconds) {
  if (!Number.isFinite(seconds)) seconds = 0;
  seconds = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor(seconds / 60) % 60;
  const rest = String(seconds % 60).padStart(2, "0");
  return hours ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}` : `${minutes}:${rest}`;
}

async function request(method, path, body, options = {}) {
  const response = await fetch(`${api}${path}`, {
    method,
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    ...options,
  });
  const text = await response.text();
  let data = null;
  try { data = text ? JSON.parse(text) : null; } catch {}
  if (!response.ok) {
    const error = new Error((data && (data.error || data.message)) || `The TV box answered ${response.status}`);
    error.status = response.status;
    error.data = data;
    throw error;
  }
  return data;
}

function toast(text, tone = "info") {
  window.dispatchEvent(new CustomEvent("semmeltv:toast", { detail: { text, tone } }));
}

function capabilities() {
  const probe = document.createElement("video");
  const MSE = window.ManagedMediaSource || window.MediaSource;
  const apple = /Apple/.test(navigator.vendor || "");
  const native = probe.canPlayType("application/vnd.apple.mpegurl") !== "" && (apple || !MSE);
  const supports = (type) => native ? probe.canPlayType(type) !== "" : Boolean(MSE && MSE.isTypeSupported(type));
  return {
    native,
    playable: native || Boolean(MSE),
    client: {
      hevc: supports('video/mp4; codecs="hvc1.2.4.L120.90"') || supports('video/mp4; codecs="hvc1.1.6.L120.90"'),
      eac3: supports('audio/mp4; codecs="ec-3"'),
      ac3: supports('audio/mp4; codecs="ac-3"'),
    },
  };
}

function isLaptop() {
  const pointer = matchMedia("(hover: hover) and (pointer: fine)").matches;
  return pointer && Math.min(window.screen.width, window.screen.height) >= 600;
}

function deviceName() {
  const agent = navigator.userAgent;
  if (/iPhone/.test(agent)) return "iPhone";
  if (/iPad/.test(agent) || (/Macintosh/.test(agent) && navigator.maxTouchPoints > 1)) return "iPad";
  if (/Android/.test(agent)) return "Android phone";
  if (/Macintosh/.test(agent)) return "Mac";
  if (/Windows/.test(agent)) return "Windows PC";
  return isLaptop() ? "laptop" : "phone";
}

function thisDevice() {
  let id = stored(DEVICE_KEY);
  if (!id) {
    const bytes = new Uint8Array(16);
    crypto.getRandomValues(bytes);
    id = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
    store(DEVICE_KEY, id);
  }
  document.cookie = `semmeltv-device=${id}; path=/; max-age=31536000; samesite=lax`;
  return { id, name: deviceName() };
}

const device = thisDevice();

// The screen as this device reads it: `here` says what its player does.
function screenPath() {
  return `/api/screen?device=${encodeURIComponent(device.id)}`;
}

function ownsSpace(target) {
  return Boolean(target.closest?.(".pp button") && !target.closest(".pp-layer"));
}

function samePick(a, b) {
  if (!a || !b) return false;
  if (a.movie || b.movie) return a.movie === b.movie;
  return a.show === b.show && a.season === b.season && a.episode === b.episode;
}

function pickOf(pick) {
  if (pick.movie || pick.season) return pick;
  return player && player.session && player.session.key === pick.show ? player.pickOf(player.session) : pick;
}

function pickOfPlay(play) {
  return play.kind === "movie" ? { movie: play.movie } : { show: play.show, season: play.season, episode: play.episode };
}

function tvPick() {
  const state = document.querySelector(".now-state");
  if (!state || !state.dataset.key) return null;
  const [movie, show, season, episode] = state.dataset.key.split("|");
  if (movie) return { movie };
  return show ? { show, season: Number(season), episode: Number(episode) } : null;
}

function ours(view) {
  return Boolean(view.here && view.here.do !== "close");
}

function render() {
  const kind = isLaptop() ? "laptop" : "phone";
  if (html.dataset.device !== kind) html.dataset.device = kind;
  const mode = screen.kind === "tv" && controlsTv ? "tv" : "phone";
  if (html.dataset.mode !== mode) html.dataset.mode = mode;
  if (!html.hasAttribute("data-phone-ready")) html.dataset.phoneReady = "";
  if (player && !player.closed) {
    if (!player.root.isConnected) document.body.append(player.root);
    if (!player.mini && !html.classList.contains("pp-open")) html.classList.add("pp-open");
  }
  paintSwitch();
}

function paintSwitch() {
  const button = document.getElementById("mode");
  if (!button) return;
  const mode = pending === "phone" ? "moving" : pending === "tv" ? "tv" : screen.kind === "tv" ? "tv" : screen.kind;
  if (button.dataset.mode !== mode) button.dataset.mode = mode;
  const pressed = String(mode !== "tv");
  if (button.getAttribute("aria-pressed") !== pressed) button.setAttribute("aria-pressed", pressed);
}

function sleep(milliseconds) {
  return new Promise((done) => setTimeout(done, milliseconds));
}

function transient(error) {
  return !error.status || error.status >= 500;
}

function closedForMaintenance() {
  return html.hasAttribute("data-maintenance");
}

function adopt(view) {
  const restarted = boot !== null && view.boot !== undefined && view.boot !== boot;
  if (!restarted && view.seq !== undefined && screen.seq !== undefined && view.seq < screen.seq) return;
  if (view.boot !== undefined) boot = view.boot;
  screen = view;
  const notice = view.notice;
  if (seenNotice === undefined) {
    seenNotice = notice ? notice.id : null;
  } else if (notice && notice.id !== seenNotice) {
    seenNotice = notice.id;
    toast(notice.text, notice.tone === "error" ? "error" : "info");
  }
  render();
  if (restarted && player && !player.closed && player.resumeHere()) return;
  reconcile();
}

function reconcile() {
  const view = screen;
  const here = view.here || { do: "close" };
  if (reloaded) {
    const saved = reloaded;
    reloaded = null;
    if (here.do === "hold") {
      playHere(saved.pick);
      if (saved.mini && player) player.minimize();
      return;
    }
  }
  if (here.do === "open") {
    const pick = pickOfPlay(here);
    if (player && !player.closed && player.shows(pick)) return;
    openPlayer(pick, { start: here.at });
    return;
  }
  if (!player || player.closed || player.fresh() || player.resuming || player.awaiting || player.bakeoff.shown()) return;
  const play = view.play;
  const why = `screen is ${view.kind}${view.device ? ` on ${view.device.name}` : ""}${play ? `, ${play.state}` : ", nothing playing"}${view.notice ? `; last notice: ${view.notice.text}` : ""}`;
  if (here.do === "close") {
    note("player closes: the screen moved away", why);
    player.close(false);
  } else if (player.playing()) {
    note(`the TV dropped this phone's play at ${clock(player.now())}; it keeps playing and reports`, why);
    player.report("playing");
  } else {
    note(`player stops at ${clock(player.now())}: the TV dropped this phone's play`, why);
    player.idle();
  }
}

function listen() {
  if (socket && socket.readyState <= 1) return;
  const base = api ? api.replace(/^http/, "ws") : location.origin.replace(/^http/, "ws");
  const mine = new WebSocket(`${base}/api/events?device=${encodeURIComponent(device.id)}`);
  socket = mine;
  mine.addEventListener("open", () => {
    request("GET", screenPath()).then(adopt).catch(() => {});
  });
  mine.addEventListener("message", (message) => {
    let event;
    try { event = JSON.parse(message.data); } catch { return; }
    if (event.type === "screen") adopt(event.screen);
  });
  mine.addEventListener("close", () => {
    if (socket !== mine) return;
    socket = null;
    setTimeout(listen, RECONNECT);
  });
}

function relisten() {
  const old = socket;
  socket = null;
  if (old) old.close();
  listen();
}

function note(message, detail) {
  if (window.semmeltvReport) window.semmeltvReport("info", message, detail);
}

async function tell(body) {
  const asked = `screen ${body.action}${body.to ? ` to ${body.to}` : ""}`;
  note(asked, `screen was ${screen.kind}`);
  try {
    await request("POST", "/api/screen", { ...body, device });
    const view = await request("GET", screenPath());
    note(`${asked}: TV agreed`, `screen is ${view.kind}${view.play ? `, ${view.play.state}` : ""}`);
    pending = null;
    adopt(view);
  } catch (error) {
    pending = null;
    paintSwitch();
    if (player && player.waiting) player.close(false);
    request("GET", screenPath()).then(adopt).catch(() => {});
    throw error;
  }
}

async function command(body) {
  try {
    await tell(body);
  } catch (error) {
    toast(error.message, "error");
  }
}

function switchTo(target, body) {
  window.semmeltvAct({
    key: "screen",
    mark: "#mode",
    optimistic: () => {
      pending = target;
      paintSwitch();
    },
    send: () => tell(body),
  });
}

function canWatchHere() {
  if (!capabilities().playable) {
    toast("This browser can't play the videos", "error");
    return false;
  }
  return true;
}

function flip() {
  if (screen.kind === "tv") {
    if (!canWatchHere()) return;
    const pick = tvPick();
    if (pick) openPlayer(pick, { waiting: true });
    switchTo("phone", { action: "flip", to: "phone" });
  } else {
    const position = player && !player.closed && ours(screen) && player.placed() ? player.now() : undefined;
    switchTo("tv", { action: "flip", to: "tv", position });
  }
}

function playHere(pick) {
  if (!canWatchHere()) return;
  if (player && !player.closed && player.session && !player.idling && samePick(player.pick, pickOf(pick))) {
    player.expand();
    return;
  }
  openPlayer(pick, { waiting: true });
  command({ action: "play", ...pick });
}

// bake-off: begin
const BELL_EVERY = 2000;
const BAKE_POLL = 1000;
const WINDOW_MS = 250;
const BURNS_AFTER_MS = 600;
const REST_MS = 500;
const DOUGH = [243, 227, 192];
const GOLDEN = [217, 154, 58];
const BURNT = [59, 36, 18];

function mix(from, to, share) {
  const amount = Math.min(1, Math.max(0, share));
  return `rgb(${from.map((value, index) => Math.round(value + (to[index] - value) * amount)).join(",")})`;
}

function crustOf(error) {
  return Math.max(0, Math.round(100 * (1 - (error / WINDOW_MS) ** 2)));
}

// The challenge for a place to watch, on this phone: the full oven's sheet,
// a challenge to answer, the bake itself and how it came out. The rules are
// in docs/streaming.md; the TV decides, the phone only shows and presses.
class Bakeoff {
  constructor(player) {
    this.player = player;
    this.root = element("div", "pp-bake", { role: "dialog", "aria-label": "Semmeln" });
    this.root.hidden = true;
    this.bellTimer = 0;
    this.pollTimer = 0;
    this.frame = 0;
    this.challenge = null;
    this.handled = new Set();
    this.held = false;
    this.onKey = () => {};
  }

  show(...children) {
    this.root.replaceChildren(...children);
    this.root.hidden = false;
  }

  hide() {
    this.root.hidden = true;
    this.root.replaceChildren();
  }

  shown() {
    return !this.root.hidden;
  }

  stop() {
    clearInterval(this.bellTimer);
    clearTimeout(this.pollTimer);
    cancelAnimationFrame(this.frame);
    window.removeEventListener("keydown", this.onKey, true);
    this.challenge = null;
    this.hide();
  }

  heading(text, detail) {
    const box = element("div", "pp-bake-head");
    box.append(element("h2", "", { text }));
    if (detail) box.append(element("p", "", { text: detail }));
    return box;
  }

  pill(text, strong, action) {
    const node = element("button", strong ? "pp-pill pp-strong" : "pp-pill", { type: "button", text });
    node.addEventListener("click", action);
    return node;
  }

  // A sheet as the TV words it: its heading, what goes between, its buttons.
  sheet(sheet, ...middle) {
    const children = [this.heading(sheet.title, sheet.detail), ...middle];
    const pills = sheet.actions.map((action) => this.pill(action.label, action.strong, () => this.act(action.does)));
    if (pills.length > 1) {
      const row = element("div", "pp-row");
      row.append(...pills);
      children.push(row);
    } else {
      children.push(...pills);
    }
    if (sheet.note) children.push(element("p", "pp-bake-what", { text: sheet.note }));
    this.show(...children);
  }

  act(does) {
    const challenge = this.challenge;
    if (does === "accept") return this.accept();
    if (does === "drop-off") return this.dropOff();
    if (does === "give-up") return this.giveUp();
    if (does === "resume") return this.resume();
    if (does === "again") return this.practise();
    this.stop();
    if (does === "watch" && challenge) this.player.open(challenge.pick);
    else this.player.close(false);
  }

  // The oven is full: who can be challenged, as the TV lists them.
  full(full, pick) {
    this.stop();
    this.player.root.classList.remove("pp-loading");
    const list = element("ul", "pp-bake-list");
    for (const watcher of full.watchers) {
      const item = element("li");
      const row = watcher.pickable ? element("button", "pp-bake-row", { type: "button" }) : element("div", "pp-bake-row");
      row.append(element("span", "pp-bake-who", { text: watcher.person }), element("span", "pp-bake-what", { text: watcher.line }));
      if (watcher.note) row.append(element("span", "pp-bake-what", { text: watcher.note }));
      if (watcher.pickable) row.addEventListener("click", () => this.dare(watcher, pick));
      item.append(row);
      list.append(item);
    }
    this.show(this.heading(full.heading, full.message), element("p", "pp-bake-ask", { text: full.question }), list,
      this.pill("Close", false, () => this.player.close(false)));
  }

  async dare(watcher, pick) {
    let made;
    try {
      made = await request("POST", "/api/challenges", { seat: watcher.seat.kind === "tv" ? "tv" : watcher.seat.id, device });
    } catch (error) {
      toast(error.message, "error");
      return;
    }
    this.challenge = { pick, path: `/api/challenges/${made.id}` };
    this.poll(0);
  }

  // A practice bake against the Bäckermeister: the real game, but nobody's
  // place is at stake and nobody rests after it.
  async practise() {
    this.stop();
    let made;
    try {
      made = await request("POST", "/api/challenges/practice");
    } catch (error) {
      toast(error.message, "error");
      return;
    }
    this.challenge = { path: `/api/challenges/${made.id}` };
    this.poll(0);
  }

  giveUp() {
    const challenge = this.challenge;
    this.stop();
    if (challenge) request("DELETE", challenge.path).catch(() => {});
    this.player.close(false);
  }

  // While a stream plays: asks every few seconds whether someone challenges it.
  watch(session) {
    clearInterval(this.bellTimer);
    this.bellTimer = setInterval(() => this.ring(session), BELL_EVERY);
  }

  async ring(session) {
    if (this.challenge || this.player.session !== session || this.player.closed) return;
    let seen;
    try {
      seen = await request("GET", `/api/stream/${session.id}/challenge`);
    } catch {
      return;
    }
    if (!seen || seen.stage !== "asked" || this.challenge) return;
    this.challenge = { path: `/api/stream/${session.id}/challenge` };
    this.held = !this.player.video.paused;
    this.player.pause();
    this.see(seen);
    this.poll();
  }

  async accept() {
    try {
      this.see(await request("POST", `${this.challenge.path}/accept`));
    } catch (error) {
      toast(error.message, "error");
    }
  }

  async dropOff() {
    const challenge = this.challenge;
    this.stop();
    if (challenge) await request("POST", `${challenge.path}/drop-off`).catch(() => {});
    this.player.close(false);
  }

  poll(delay = BAKE_POLL) {
    clearTimeout(this.pollTimer);
    this.pollTimer = setTimeout(async () => {
      const challenge = this.challenge;
      if (!challenge) return;
      try {
        const seen = await request("GET", challenge.path);
        if (this.challenge === challenge && seen) this.see(seen);
      } catch (error) {
        if (error.status === 404 && this.challenge === challenge) this.stop();
      }
      if (this.challenge === challenge) this.poll();
    }, delay);
  }

  see(seen) {
    const challenge = this.challenge;
    if (!challenge) return;
    if (seen.stage === "baking") {
      if (!challenge.oven) this.bake(seen);
      else if (seen.pressed) this.sheet(seen.sheet);
    } else if (seen.stage === "done") {
      if (this.handled.has(challenge.path)) return;
      this.handled.add(challenge.path);
      this.done(seen);
    } else {
      this.sheet(seen.sheet);
    }
  }

  // The bake: three Semmeln on this phone's own clock. A press counts in
  // milliseconds after the moment its Semmel turned golden on this screen.
  bake(seen) {
    const challenge = this.challenge;
    const begins = performance.now() + Math.max(0, seen.starts_in_ms);
    const slots = [];
    let at = 0;
    for (const golden of seen.golden_ms) {
      slots.push({ start: at, golden });
      at += golden + BURNS_AFTER_MS + REST_MS;
    }
    const oven = { presses: [null, null, null], input: null, sent: false };
    challenge.oven = oven;
    const count = element("div", "pp-bake-count");
    const semmel = element("div", "pp-bake-semmel");
    const bar = element("div", "pp-bake-bar");
    const band = element("div", "pp-bake-band");
    const mark = element("div", "pp-bake-mark");
    bar.append(band, mark);
    const scores = element("div", "pp-bake-scores");
    const stage = element("div", "pp-bake-oven");
    stage.append(count, semmel, bar, scores, element("p", "pp-bake-what", { text: "Tap when the Semmel is golden" }));
    this.show(this.heading(seen.sheet.title, seen.sheet.detail), stage);
    const bias = (input) => (seen.bias_ms && seen.bias_ms[input]) || 0;
    const burnsAt = (slot) => slot.golden + BURNS_AFTER_MS;
    const pull = (time, input) => {
      const elapsed = time - begins;
      const index = slots.findIndex((slot) => elapsed >= slot.start && elapsed < slot.start + burnsAt(slot));
      if (index < 0 || oven.presses[index] !== null) return;
      oven.input = oven.input || input;
      const error = elapsed - slots[index].start - slots[index].golden;
      oven.presses[index] = Math.round(error);
      scores.append(element("span", "", { text: `+${crustOf(error - bias(oven.input))}` }));
    };
    stage.addEventListener("pointerdown", (event) => {
      event.preventDefault();
      pull(event.timeStamp, event.pointerType === "mouse" ? "mouse" : "touch");
    });
    this.onKey = (event) => {
      if (event.key !== " " && event.key !== "Enter") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (!event.repeat) pull(event.timeStamp, "keys");
    };
    window.addEventListener("keydown", this.onKey, true);
    const burnt = new Set();
    const paint = () => {
      if (this.challenge !== challenge) return;
      const elapsed = performance.now() - begins;
      if (elapsed < 0) {
        count.textContent = String(Math.ceil(-elapsed / 1000));
        semmel.style.background = mix(DOUGH, GOLDEN, 0);
      } else {
        count.textContent = "";
        const index = slots.findLastIndex((slot) => elapsed >= slot.start);
        const slot = slots[index];
        const into = elapsed - slot.start;
        const pressed = oven.presses[index];
        const stopped = pressed === null ? burnsAt(slot) : slot.golden + pressed;
        const shown = Math.min(into, stopped);
        semmel.style.background = shown <= slot.golden
          ? mix(DOUGH, GOLDEN, shown / slot.golden)
          : mix(GOLDEN, BURNT, (shown - slot.golden) / BURNS_AFTER_MS);
        semmel.classList.toggle("pp-bake-out", into >= stopped);
        mark.style.left = `${Math.min(100, (into / burnsAt(slot)) * 100)}%`;
        band.style.left = `${((slot.golden - WINDOW_MS / 2) / burnsAt(slot)) * 100}%`;
        band.style.width = `${(WINDOW_MS / burnsAt(slot)) * 100}%`;
        if (into >= burnsAt(slot) && pressed === null && !burnt.has(index)) {
          burnt.add(index);
          scores.append(element("span", "", { text: "burnt" }));
        }
      }
      if (elapsed >= at) {
        this.send(challenge);
        return;
      }
      this.frame = requestAnimationFrame(paint);
    };
    this.frame = requestAnimationFrame(paint);
  }

  async send(challenge) {
    window.removeEventListener("keydown", this.onKey, true);
    const oven = challenge.oven;
    if (oven.sent) return;
    oven.sent = true;
    try {
      const seen = await request("POST", `${challenge.path}/presses`, { input: oven.input || "touch", presses: oven.presses });
      if (this.challenge === challenge && seen) this.see(seen);
    } catch {}
  }

  done(seen) {
    clearTimeout(this.pollTimer);
    cancelAnimationFrame(this.frame);
    window.removeEventListener("keydown", this.onKey, true);
    const both = seen.baked && seen.baked[0] && seen.baked[1];
    this.sheet(seen.sheet, ...(both ? [this.table(seen)] : []));
  }

  table(seen) {
    const table = element("table", "pp-bake-table");
    const head = element("tr");
    head.append(element("th"), element("th", "", { text: seen.challenger }), element("th", "", { text: seen.defender }));
    table.append(head);
    const [first, second] = seen.baked;
    first.crust.forEach((_, index) => {
      const row = element("tr");
      row.append(
        element("td", "", { text: `Semmel ${index + 1}` }),
        element("td", "", { text: `${first.crust[index]} crust · ${first.rise[index]} rise` }),
        element("td", "", { text: `${second.crust[index]} crust · ${second.rise[index]} rise` }),
      );
      table.append(row);
    });
    const total = element("tr", "pp-bake-total");
    total.append(element("td", "", { text: "Total" }), element("td", "", { text: String(first.total) }), element("td", "", { text: String(second.total) }));
    table.append(total);
    return table;
  }

  resume() {
    const held = this.held;
    this.held = false;
    this.stop();
    if (held && !this.player.closed) this.player.video.play()?.catch(() => {});
    if (this.player.session && !this.player.closed) this.watch(this.player.session);
  }
}
// A practice bake on its own sheet, opened by the address ending in
// #semmelstechen; there is no button for it.
const PRACTICE = "#semmelstechen";

function practiceBake() {
  if (document.querySelector(".pp-practice")) return;
  const root = element("div", "pp pp-in pp-practice", { role: "dialog", "aria-modal": "true", "aria-label": "Practice" });
  const host = {
    root,
    closed: false,
    session: null,
    video: { paused: true },
    pause() {},
    open() {},
    close() {
      host.closed = true;
      bakeoff.stop();
      root.remove();
      html.classList.remove("pp-open");
      if (location.hash === PRACTICE) history.replaceState(null, "", location.pathname + location.search);
    },
  };
  const bakeoff = new Bakeoff(host);
  root.append(bakeoff.root);
  document.body.append(root);
  html.classList.add("pp-open");
  bakeoff.practise();
}

// bake-off: end

class Player {
  constructor() {
    this.capabilities = capabilities();
    this.root = element("div", "pp", { role: "dialog", "aria-modal": "true", "aria-label": "Player", tabindex: "-1" });
    this.video = element("video", "pp-video", { playsinline: "", "webkit-playsinline": "", "x-webkit-airplay": "allow", preload: "auto" });
    this.video.disablePictureInPicture = false;
    this.layer = element("div", "pp-layer");
    this.bakeoff = new Bakeoff(this);
    this.build();
    document.body.append(this.root);
    this.root.focus({ preventScroll: true });
    this.mini = false;
    pushEntry();
    this.onPop = () => { if (!ownEntry()) this.minimize(); };
    window.addEventListener("popstate", this.onPop);
    html.classList.add("pp-open");
    this.session = null;
    this.hls = null;
    this.base = 0;
    this.ready = false;
    this.subtitleTracks = new Map();
    this.cuesAdded = new Map();
    this.played = 0;
    this.lastTime = null;
    this.started = false;
    // A stream that replaced a playing one while it loads: the play goes on,
    // so the player keeps reporting it.
    this.carrying = false;
    this.opening = null;
    this.reportTimer = 0;
    this.hideTimer = 0;
    this.countdown = null;
    this.closed = false;
    this.waiting = false;
    this.idling = false;
    this.marks = { segments: [], credits: null };
    this.bind();
    this.show(true);
    this.video.play()?.catch(() => {});
    requestAnimationFrame(() => this.root.classList.add("pp-in"));
  }

  build() {
    this.closeButton = button("pp-icon pp-close", "Minimize", icons.close);
    this.titleText = element("div", "pp-title");
    this.airplay = button("pp-icon pp-airplay", "AirPlay", icons.airplay);
    this.pip = button("pp-icon pp-pip", "Picture in Picture", icons.pip);
    this.airplay.hidden = true;
    this.pip.hidden = !(document.pictureInPictureEnabled || this.video.webkitSupportsPresentationMode);
    this.tvButton = button("pp-icon pp-tv", "Continue on TV", icons.tv);
    this.tvButton.hidden = !controlsTv;
    const top = element("div", "pp-top");
    top.append(this.closeButton, this.titleText, this.airplay, this.pip, this.tvButton);

    this.backButton = button("pp-icon pp-jump", `Back ${JUMP} seconds`, `${icons.back}<span>${JUMP}</span>`);
    this.playButton = button("pp-icon pp-play", "Play", icons.play);
    this.aheadButton = button("pp-icon pp-jump", `Ahead ${JUMP} seconds`, `${icons.ahead}<span>${JUMP}</span>`);
    const middle = element("div", "pp-middle");
    middle.append(this.backButton, this.playButton, this.aheadButton);

    this.elapsed = element("span", "pp-time", { text: "0:00" });
    this.remaining = element("span", "pp-time", { text: "–:––" });
    this.scrub = element("input", "pp-scrub", { type: "range", min: "0", max: "1", step: "any", value: "0", "aria-label": "Position" });
    this.bubble = element("span", "pp-bubble");
    const line = element("div", "pp-line");
    const track = element("div", "pp-track");
    track.append(this.scrub, this.bubble);
    line.append(this.elapsed, track, this.remaining);

    this.tracksButton = element("button", "pp-pill", { type: "button", html: `${icons.tracks}<span>Audio &amp; subtitles</span>` });
    this.nextButton = element("button", "pp-pill", { type: "button", "aria-label": "Next episode", html: `${icons.next}<span>Next</span>` });
    this.fullButton = button("pp-icon pp-full", "Full screen", icons.full);
    this.nextButton.hidden = true;
    const actions = element("div", "pp-actions");
    actions.append(this.tracksButton, this.nextButton, this.fullButton);

    const bottom = element("div", "pp-bottom");
    bottom.append(line, actions);
    this.layer.append(top, middle, bottom);

    this.skip = element("button", "pp-skip", { type: "button" });
    this.skip.hidden = true;
    this.upNext = element("div", "pp-upnext");
    this.upNext.hidden = true;
    this.sheet = element("div", "pp-sheet", { role: "dialog", "aria-label": "Audio and subtitles" });
    this.sheet.hidden = true;
    this.message = element("div", "pp-message");
    this.spinner = element("div", "pp-spinner");
    this.ripple = element("div", "pp-ripple");
    this.miniText = element("div", "pp-mini-text");
    this.miniPlay = button("pp-icon pp-mini-play", "Play", icons.play);
    this.miniStop = button("pp-icon pp-mini-stop", "Stop", icons.stop);
    this.miniBar = element("div", "pp-minibar", { role: "button", "aria-label": "Open the player" });
    this.miniBar.append(this.miniText, this.miniPlay, this.miniStop);
    this.root.append(this.video, this.spinner, this.ripple, this.layer, this.skip, this.upNext, this.sheet, this.message, this.bakeoff.root, this.miniBar);
  }

  bind() {
    const video = this.video;
    this.closeButton.addEventListener("click", () => this.minimize());
    this.miniPlay.addEventListener("click", () => this.toggle());
    this.miniStop.addEventListener("click", () => this.close());
    this.miniBar.addEventListener("click", (event) => {
      if (this.dragged || event.target.closest("button")) return;
      this.expand();
    });
    this.playButton.addEventListener("click", () => this.toggle());
    this.backButton.addEventListener("click", () => this.jump(-JUMP));
    this.aheadButton.addEventListener("click", () => this.jump(JUMP));
    this.tracksButton.addEventListener("click", () => this.openSheet());
    this.nextButton.addEventListener("click", () => this.playNext());
    this.tvButton.addEventListener("click", () => this.continueOnTv());
    this.fullButton.addEventListener("click", () => this.fullscreen());
    this.pip.addEventListener("click", () => this.pictureInPicture());
    this.airplay.addEventListener("click", () => video.webkitShowPlaybackTargetPicker?.());
    this.skip.addEventListener("click", () => {
      const target = Number(this.skip.dataset.to);
      if (this.skip.dataset.next) this.playNext();
      else if (Number.isFinite(target)) this.seekTo(target);
    });
    if (window.WebKitPlaybackTargetAvailabilityEvent) {
      video.addEventListener("webkitplaybacktargetavailabilitychanged", (event) => {
        this.airplay.hidden = event.availability !== "available";
      });
    }

    this.scrub.addEventListener("input", () => {
      this.scrubbing = true;
      const at = Number(this.scrub.value);
      this.bubble.textContent = clock(at);
      this.bubble.style.left = `${(at / (Number(this.scrub.max) || 1)) * 100}%`;
      this.bubble.classList.add("pp-shown");
      this.elapsed.textContent = clock(at);
      this.show(true);
    });
    this.scrub.addEventListener("change", () => {
      this.scrubbing = false;
      this.bubble.classList.remove("pp-shown");
      this.seekTo(Number(this.scrub.value));
      this.show();
    });

    video.addEventListener("timeupdate", () => {
      const now = this.now();
      if (this.lastTime !== null && !video.seeking && !video.paused) {
        const step = now - this.lastTime;
        if (step > 0 && step < 2) {
          this.played += step;
          this.stopAt(this.lastTime, now);
        }
      }
      this.lastTime = now;
      if (!this.scrubbing) this.paint();
      this.marksAt(now);
      this.positionState();
    });
    video.addEventListener("seeking", () => { this.lastTime = null; });
    video.addEventListener("waiting", () => {
      this.root.classList.add("pp-loading");
      clearTimeout(this.waitingTimer);
      const at = this.now();
      this.waitingTimer = setTimeout(() => {
        const ranges = Array.from({ length: video.buffered.length }, (_, index) => `${clock(video.buffered.start(index))}-${clock(video.buffered.end(index))}`).join(" ");
        note(`video has waited 8 s for data at ${clock(at)}`, `buffered ${ranges || "nothing"}, network state ${video.networkState}, ready state ${video.readyState}`);
      }, 8000);
    });
    video.addEventListener("canplay", () => {
      clearTimeout(this.waitingTimer);
      this.root.classList.remove("pp-loading");
    });
    video.addEventListener("playing", () => {
      this.root.classList.remove("pp-loading");
      for (const each of [this.playButton, this.miniPlay]) {
        each.innerHTML = icons.pause;
        each.setAttribute("aria-label", "Pause");
      }
      this.begin();
      this.show();
      this.mediaState("playing");
    });
    video.addEventListener("pause", () => {
      if (!this.pausing && this.started && !video.ended && !this.closed) note(`video paused by the browser at ${clock(this.now())}`, `page ${document.visibilityState}`);
      this.pausing = false;
      for (const each of [this.playButton, this.miniPlay]) {
        each.innerHTML = icons.play;
        each.setAttribute("aria-label", "Play");
      }
      this.show(true);
      if (this.started && !video.ended && !this.closed) this.report("paused");
      this.mediaState("paused");
    });
    video.addEventListener("ended", () => this.ended());
    video.addEventListener("error", () => { if (video.getAttribute("src")) this.trouble(video.error && video.error.message); });

    this.reportTimer = setInterval(() => {
      if (this.opening || this.idling) return;
      if (this.started) this.report(video.paused ? "paused" : "playing");
      else if (this.carrying) this.report("playing");
    }, REPORT_EVERY);
    this.onHide = () => {
      if (document.visibilityState === "hidden" && this.started) this.report(video.paused ? "paused" : "playing", true);
    };
    document.addEventListener("visibilitychange", this.onHide);
    this.onLeave = () => {
      if (!this.started && !this.carrying) return;
      this.report("stopped", true);
      try { sessionStorage.setItem(RESUME_KEY, JSON.stringify({ pick: this.pick, mini: this.mini, at: Date.now() })); } catch {}
    };
    window.addEventListener("pagehide", this.onLeave);

    let taps = [];
    this.root.addEventListener("pointerdown", (event) => {
      if (this.drag || (event.pointerType === "mouse" && event.button !== 0)) return;
      if (event.target.closest("input, .pp-sheet, .pp-message, .pp-bake, .pp-upnext, .pp-skip")) return;
      if (this.mini ? event.target.closest(".pp-minibar button") : !event.target.closest(".pp-layer") || event.target.closest("button")) return;
      this.drag = { pointer: event.pointerId, start: event.clientY, moved: 0, live: false, opening: this.mini, samples: [{ y: event.clientY, t: performance.now() }] };
    });
    this.onDragMove = (event) => {
      if (this.drag && this.drag.pointer === event.pointerId) this.dragTo(event.clientY);
    };
    this.onDragEnd = (event) => {
      if (this.drag && this.drag.pointer === event.pointerId) this.dragEnd();
    };
    window.addEventListener("pointermove", this.onDragMove);
    window.addEventListener("pointerup", this.onDragEnd);
    window.addEventListener("pointercancel", this.onDragEnd);
    let wheelTimer = 0;
    this.root.addEventListener("wheel", (event) => {
      if (event.ctrlKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) return;
      if (this.drag && this.drag.pointer !== "wheel") return;
      const delta = event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? window.innerHeight : 1);
      if (!this.drag) {
        if (this.closed || this.leaving || !this.sheet.hidden || (this.mini ? delta <= 0 : delta >= 0)) return;
        this.drag = { pointer: "wheel", start: 0, y: 0, moved: 0, live: false, opening: this.mini, samples: [{ y: 0, t: performance.now() - WHEEL_FRAME }] };
      }
      event.preventDefault();
      this.drag.y -= delta;
      this.dragTo(this.drag.y);
      clearTimeout(wheelTimer);
      wheelTimer = setTimeout(() => {
        if (this.drag && this.drag.pointer === "wheel") this.dragEnd(false);
      }, WHEEL_IDLE);
    }, { passive: false });
    let pointer = "touch";
    this.root.addEventListener("pointerdown", (event) => { pointer = event.pointerType; }, true);
    this.root.addEventListener("pointermove", (event) => {
      if (event.pointerType !== "mouse") return;
      pointer = "mouse";
      if (event.movementX === 0 && event.movementY === 0) return;
      this.show();
    });
    this.root.addEventListener("pointerover", (event) => {
      if (event.pointerType === "mouse") this.hovering = Boolean(event.target.closest(HOVER_KEEPS));
    });
    this.root.addEventListener("mouseleave", () => {
      this.hovering = false;
      this.hide();
    });
    this.layer.addEventListener("dblclick", (event) => {
      if (pointer !== "mouse" || !this.sheet.hidden || event.target.closest("button, input")) return;
      this.fullscreen();
    });
    this.layer.addEventListener("click", (event) => {
      if (this.dragged || event.target.closest("button, input")) return;
      if (!this.sheet.hidden) {
        this.closeSheet();
        return;
      }
      if (pointer === "mouse") {
        this.toggle();
        return;
      }
      const now = Date.now();
      taps = taps.filter((tap) => now - tap < 320);
      taps.push(now);
      if (taps.length >= 2) {
        const side = event.clientX < this.root.clientWidth / 2 ? -1 : 1;
        this.jump(side * JUMP, side);
        clearTimeout(this.tapTimer);
        return;
      }
      clearTimeout(this.tapTimer);
      this.tapTimer = setTimeout(() => (this.root.classList.contains("pp-controls") ? this.hide() : this.show()), 260);
    });
    this.onKey = (event) => {
      if (event.ctrlKey || event.metaKey || event.altKey || this.closed || this.mini) return;
      const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
      const keys = {
        " ": () => this.toggle(),
        k: () => this.toggle(),
        ArrowLeft: () => this.jump(-JUMP),
        ArrowRight: () => this.jump(JUMP),
        ArrowDown: () => this.jump(-60),
        ArrowUp: () => this.jump(60),
        f: () => this.fullscreen(),
        m: () => { video.muted = !video.muted; this.show(); },
        Escape: () => (this.sheet.hidden ? this.minimize() : this.closeSheet()),
      };
      const act = keys[key];
      if (!act || (key === " " && ownsSpace(event.target))) return;
      event.preventDefault();
      event.stopPropagation();
      if (!event.repeat || key.startsWith("Arrow")) act();
    };
    this.onKeyUp = (event) => {
      if (event.key === " " && !this.closed && !this.mini && !ownsSpace(event.target)) event.preventDefault();
    };
    document.addEventListener("keydown", this.onKey, true);
    document.addEventListener("keyup", this.onKeyUp, true);
  }

  title(name, caption) {
    for (const place of [this.titleText, this.miniText]) {
      place.innerHTML = "";
      place.append(element("strong", "", { text: name }));
      if (caption) place.append(element("span", "", { text: caption }));
    }
  }

  closeSheet() {
    this.sheet.hidden = true;
    this.show();
  }

  dragTo(y) {
    const drag = this.drag;
    const now = performance.now();
    drag.moved = y - drag.start;
    drag.samples.push({ y, t: now });
    while (drag.samples.length > 2 && now - drag.samples[0].t > 100) drag.samples.shift();
    if (!drag.live) {
      if (Math.abs(drag.moved) < START_DRAG) return;
      if (drag.opening ? drag.moved > 0 : drag.moved < 0) {
        this.drag = null;
        return;
      }
      drag.live = true;
      drag.height = window.innerHeight;
      try { this.root.setPointerCapture(drag.pointer); } catch {}
      clearTimeout(this.settleTimer);
      this.root.classList.add("pp-dragging");
      if (drag.opening) this.layout(false);
    }
    const base = drag.opening ? drag.height : 0;
    const at = Math.min(Math.max(base + drag.moved, 0), drag.height);
    this.root.style.transform = `translateY(${at}px)`;
  }

  dragEnd(heldStill = true) {
    const drag = this.drag;
    this.drag = null;
    if (!drag.live) return;
    this.dragged = true;
    setTimeout(() => { this.dragged = false; }, 0);
    const first = drag.samples[0];
    const last = drag.samples[drag.samples.length - 1];
    const held = heldStill && performance.now() - last.t > 100;
    const speed = !held && last.t > first.t ? (last.y - first.y) / (last.t - first.t) : 0;
    const travelled = Math.abs(drag.moved) / drag.height;
    const open = drag.opening
      ? speed < -FLICK || (travelled > 0.25 && speed <= FLICK)
      : !(speed > FLICK || (travelled > 0.25 && speed >= -FLICK));
    this.root.classList.remove("pp-dragging");
    if (open) this.expand(true);
    else this.minimize(true);
  }

  layout(mini) {
    this.mini = mini;
    this.root.classList.toggle("pp-mini", mini);
    this.root.setAttribute("aria-modal", String(!mini));
    html.classList.toggle("pp-open", !mini);
  }

  minimize(dragged = false) {
    if (this.closed || (this.mini && !dragged) || this.leaving) return;
    this.leaving = true;
    clearTimeout(this.settleTimer);
    if (ownEntry()) history.back();
    this.closeSheet();
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    if (!dragged) this.root.style.transform = "translateY(0px)";
    void this.root.offsetWidth;
    this.root.style.transform = "translateY(100%)";
    this.settleTimer = setTimeout(() => {
      this.leaving = false;
      if (this.closed) return;
      this.layout(true);
      this.root.style.transform = "";
      this.root.classList.remove("pp-mini-in");
      void this.root.offsetWidth;
      this.root.classList.add("pp-mini-in");
      if (this.root.contains(document.activeElement)) document.activeElement.blur();
    }, SETTLE_MS);
  }

  expand(dragged = false) {
    if (this.closed || (!this.mini && !dragged)) return;
    clearTimeout(this.settleTimer);
    this.leaving = false;
    if (this.mini) {
      this.layout(false);
      if (!dragged) {
        this.root.style.transform = "translateY(100%)";
        void this.root.offsetWidth;
      }
    }
    if (!ownEntry()) pushEntry();
    this.root.style.transform = "";
    this.root.classList.remove("pp-mini-in");
    this.root.focus({ preventScroll: true });
    this.show();
  }

  show(stay = false) {
    this.root.classList.add("pp-controls");
    clearTimeout(this.hideTimer);
    if (stay || this.video.paused) return;
    const after = html.dataset.device === "laptop" ? HIDE_AFTER_MOUSE : HIDE_AFTER;
    const later = () => {
      if (this.hovering) this.hideTimer = setTimeout(later, after);
      else this.hide();
    };
    this.hideTimer = setTimeout(later, after);
  }

  hide() {
    clearTimeout(this.hideTimer);
    if (!this.video.paused && this.sheet.hidden && !this.scrubbing) this.root.classList.remove("pp-controls");
  }

  paint() {
    const duration = this.duration();
    const at = this.now();
    this.scrub.max = String(duration || 1);
    this.scrub.value = String(at);
    const done = `${duration ? (at / duration) * 100 : 0}%`;
    this.scrub.style.setProperty("--done", done);
    this.miniBar.style.setProperty("--done", done);
    this.elapsed.textContent = clock(at);
    this.remaining.textContent = duration ? `-${clock(duration - at)}` : "–:––";
  }

  duration() {
    return this.session ? this.session.duration : 0;
  }

  // timeline: begin
  // The place in the file. The stream starts at session.offset in the file,
  // where the video's own clock reads `base`.
  now() {
    const session = this.session;
    const at = this.video.currentTime || 0;
    return session ? session.offset + at - this.base : at;
  }

  mediaTime(at) {
    return at - this.session.offset + this.base;
  }

  // A place the remux has passed is a seek in the video; anywhere else opens
  // the stream again from there.
  seekTo(at) {
    const session = this.session;
    if (!session) return;
    const target = Math.max(0, Math.min(this.duration() - 1, at));
    const time = this.mediaTime(target);
    const seekable = this.video.seekable;
    const end = seekable.length ? seekable.end(seekable.length - 1) : 0;
    if (this.ready && time >= this.base && target >= (this.keptFrom ?? 0) && time <= end - 1) {
      this.video.currentTime = time;
      return;
    }
    note(`seeks to ${clock(target)}, outside what the stream has, so it opens there`, `stream from ${clock(session.offset)} to ${clock(session.offset + end - this.base)}`);
    this.open(this.pickOf(session), { start: target, audio: session.audio_index ?? undefined });
  }
  // timeline: end

  toggle() {
    if (this.video.paused) this.video.play().catch(() => {});
    else this.pause();
  }

  playing() {
    return Boolean(this.session && this.started && !this.idling && !this.video.paused && !this.video.ended);
  }

  pause() {
    this.pausing = true;
    this.video.pause();
  }

  jump(seconds, side = 0) {
    this.seekTo(this.now() + seconds);
    if (side) {
      this.ripple.textContent = `${side < 0 ? "−" : "+"}${Math.abs(seconds)}`;
      this.ripple.dataset.side = side < 0 ? "left" : "right";
      this.ripple.classList.remove("pp-ripple-on");
      void this.ripple.offsetWidth;
      this.ripple.classList.add("pp-ripple-on");
    }
    this.show();
  }

  shows(pick) {
    return !this.idling && samePick(this.pick, pick) && !(this.waiting && !this.session);
  }

  idle() {
    if (this.video.ended || !this.session) {
      this.close(false);
      return;
    }
    if (this.idling) return;
    this.idling = true;
    this.started = false;
    this.carrying = false;
    this.pause();
    this.root.classList.remove("pp-loading");
    this.root.classList.add("pp-failed");
    this.message.innerHTML = "";
    this.message.append(element("p", "", { text: `Stopped at ${clock(this.now())}` }));
    const row = element("div", "pp-row");
    const resume = element("button", "pp-pill pp-strong", { type: "button", text: "Resume" });
    resume.addEventListener("click", () => {
      this.video.play()?.catch(() => {});
      command({ action: "play", ...this.pick });
    });
    const shut = element("button", "pp-pill", { type: "button", text: "Close" });
    shut.addEventListener("click", () => this.close(false));
    row.append(resume, shut);
    this.message.append(row);
    this.show(true);
  }

  fresh() {
    return this.waiting && !this.session && Date.now() - this.waitingSince < 5000;
  }

  wait(pick) {
    this.pick = pick;
    this.waiting = true;
    this.waitingSince = Date.now();
    this.root.classList.add("pp-loading");
    this.show(true);
  }

  // Opens the stream of `pick`. A call for the same place while one is on
  // the way is the same open and does nothing; any other call takes over, and
  // the stream the earlier call gets is closed again.
  async open(pick, options = {}) {
    const pending = this.opening;
    if (pending && samePick(pending.pick, pick) && pending.audio === options.audio
      && Math.abs((pending.start ?? 0) - (options.start ?? 0)) < 2) return;
    const opening = { pick, start: options.start, audio: options.audio };
    this.opening = opening;
    this.pick = pick;
    this.waiting = false;
    this.idling = false;
    this.root.classList.add("pp-loading");
    this.message.textContent = "";
    this.root.classList.remove("pp-failed");
    const replaces = this.session ? this.session.id : this.replacing;
    const carrying = Boolean(replaces) && (this.started || this.carrying);
    this.replacing = null;
    const body = { ...pick, client: this.capabilities.client, device };
    if (options.start !== undefined) body.start = options.start;
    if (options.audio !== undefined) body.audio = options.audio;
    if (replaces) body.replaces = replaces;
    let session;
    try {
      session = await request("POST", "/api/stream/open", body);
    } catch (error) {
      if (this.opening !== opening) return;
      this.opening = null;
      if (transient(error) && !this.closed) {
        this.awaitTv(`opening the stream failed: ${error.message}`);
        return;
      }
      if (error.status === 429 && error.data && error.data.full) {
        request("POST", "/api/screen", { action: "stop", device }).catch(() => {});
        this.bakeoff.full(error.data.full, pick);
        return;
      }
      command({ action: "failed", reason: error.message });
      this.failed(error.message);
      return;
    }
    if (this.closed || this.opening !== opening) {
      request("DELETE", `/api/stream/${session.id}`).catch(() => {});
      return;
    }
    this.opening = null;
    this.session = session;
    this.keptFrom = session.offset;
    this.bakeoff.watch(session);
    this.ready = false;
    this.played = 0;
    this.started = false;
    this.carrying = carrying;
    this.lastTime = null;
    this.marks = { segments: [], credits: session.credits };
    this.cancelCountdown();
    this.title(session.title, session.kind === "show" ? session.caption : "");
    this.nextButton.hidden = !session.next;
    this.media(session);
    this.load(session);
    request("GET", `/api/stream/${session.id}/marks`).then((marks) => {
      if (this.session === session && marks) this.marks = marks;
    }).catch(() => {});
    if (session.heavy) this.toast(`This file streams at ${(session.bitrate / 1e6).toFixed(1)} Mbit/s, about ${(session.bitrate * 3600 / 8e9).toFixed(1)} GB an hour on mobile data`);
    if (session.converted_audio) this.toast("The sound is converted to stereo for this browser");
  }

  // timeline: begin
  load(session) {
    const video = this.video;
    const source = `${api}${session.playlist}`;
    this.base = 0;
    this.subtitlesFor = null;
    this.clearSubtitles();
    const begin = () => {
      if (this.session !== session) return;
      this.base = video.seekable.length ? video.seekable.start(0) : 0;
      this.ready = true;
      if (Math.abs(this.now() - session.start) > 2) video.currentTime = this.mediaTime(session.start);
      const kept = this.chosen && samePick(this.chosen.pick, this.pick) ? this.chosen.id : undefined;
      if (kept !== undefined) this.chooseSubtitle(kept);
      else this.applyDefaultSubtitles(session);
      this.subtitlesFor = session;
      video.play().catch(() => this.blocked(session));
    };
    this.detach();
    if (this.capabilities.native) {
      video.src = source;
      video.addEventListener("loadedmetadata", begin, { once: true });
      video.load();
      return;
    }
    loadHls().then((Hls) => {
      if (this.closed || this.session !== session) return;
      const hls = new Hls({
        startPosition: Math.max(0, session.start - session.offset),
        maxBufferLength: 30,
        backBufferLength: 30,
        fLoader: initTrimmingLoader(Hls),
      });
      this.hls = hls;
      hls.on(Hls.Events.ERROR, (_, data) => { if (data.fatal && this.hls === hls) this.trouble(data.details); });
      video.addEventListener("loadedmetadata", begin, { once: true });
      hls.loadSource(source);
      hls.attachMedia(video);
    }).catch(() => this.failed("The player library didn't load"));
  }
  // timeline: end

  // Stops loading the stream the video plays.
  detach() {
    if (this.hls) { this.hls.destroy(); this.hls = null; }
    this.video.removeAttribute("src");
    this.video.load();
  }

  clearSubtitles() {
    clearTimeout(this.cueTimer);
    for (const track of this.subtitleTracks.values()) {
      track.mode = "hidden";
      for (const cue of Array.from(track.cues || [])) track.removeCue(cue);
      track.mode = "disabled";
    }
    this.cuesAdded = new Map();
  }

  // The text track for a subtitle's name and language, the same one for
  // every stream this player opens.
  subtitleTrack(subtitle) {
    const key = `${subtitle.name}\n${subtitle.language || ""}`;
    let track = this.subtitleTracks.get(key);
    if (!track) {
      track = this.video.addTextTrack("subtitles", subtitle.name, subtitle.language || "");
      track.mode = "disabled";
      this.subtitleTracks.set(key, track);
    }
    return track;
  }

  // Adds the cues the TV has for this subtitle, in the video's own time, and
  // asks again while a track inside the video is still being written out.
  async fetchCues(session, subtitle, track) {
    let answer = null;
    try {
      answer = await request("GET", `/api/stream/${session.id}/subtitles/${subtitle.id}`);
    } catch {}
    if (this.session !== session || this.subtitle !== subtitle.id) return;
    if (answer) {
      const added = this.cuesAdded.get(subtitle.id) || 0;
      for (const cue of answer.cues.slice(added)) {
        const end = this.mediaTime(cue.end);
        if (end > 0) track.addCue(new VTTCue(Math.max(0, this.mediaTime(cue.start)), end, cue.text));
      }
      this.cuesAdded.set(subtitle.id, answer.cues.length);
      if (answer.complete) return;
    }
    this.cueTimer = setTimeout(() => this.fetchCues(session, subtitle, track), CUES_EVERY);
  }

  applyDefaultSubtitles(session) {
    const wanted = session.subtitles.find((subtitle) => subtitle.default && subtitle.supported);
    if (!wanted) return;
    this.chooseSubtitle(wanted.id);
  }

  // A subtitle the viewer picked stays picked when the stream opens again for
  // the same title.
  pickSubtitle(id) {
    this.chosen = { pick: this.pick, id };
    this.chooseSubtitle(id);
  }

  chooseSubtitle(id) {
    const session = this.session;
    const wanted = id === null ? null : session.subtitles.find((subtitle) => subtitle.id === id && subtitle.supported);
    this.subtitle = wanted ? wanted.id : null;
    clearTimeout(this.cueTimer);
    const chosen = wanted ? this.subtitleTrack(wanted) : null;
    for (const track of this.subtitleTracks.values()) {
      if (track !== chosen) track.mode = "disabled";
    }
    if (!chosen) return;
    chosen.mode = "showing";
    this.fetchCues(session, wanted, chosen);
  }

  openSheet() {
    const session = this.session;
    if (!session) return;
    this.sheet.innerHTML = "";
    const grab = element("div", "pp-grab");
    const columns = element("div", "pp-columns");
    const audio = element("section", "");
    audio.append(element("h3", "", { text: "Audio" }));
    if (!session.audio.length) audio.append(element("p", "pp-quiet", { text: "No sound" }));
    for (const track of session.audio) {
      const choice = element("button", "pp-choice", { type: "button", text: track.name, "aria-pressed": String(track.index === session.audio_index) });
      choice.addEventListener("click", () => {
        this.sheet.hidden = true;
        if (track.index !== session.audio_index) this.open(this.pickOf(session), { audio: track.index, start: this.now() });
      });
      audio.append(choice);
    }
    const subtitles = element("section", "");
    subtitles.append(element("h3", "", { text: "Subtitles" }));
    const off = element("button", "pp-choice", { type: "button", text: "Off", "aria-pressed": String(this.subtitle == null) });
    off.addEventListener("click", () => { this.pickSubtitle(null); this.sheet.hidden = true; });
    subtitles.append(off);
    for (const subtitle of session.subtitles) {
      const choice = element("button", "pp-choice", { type: "button", text: subtitle.name, "aria-pressed": String(this.subtitle === subtitle.id) });
      choice.disabled = !subtitle.supported;
      choice.addEventListener("click", () => { this.pickSubtitle(subtitle.id); this.sheet.hidden = true; });
      subtitles.append(choice);
    }
    if (session.subtitle_delay) subtitles.append(element("p", "pp-quiet", { text: `Shifted ${session.subtitle_delay > 0 ? "later" : "earlier"} by ${Math.abs(session.subtitle_delay).toFixed(1)} s, as on the TV` }));
    columns.append(audio, subtitles);
    const done = element("button", "pp-pill pp-strong pp-done", { type: "button", text: "Done" });
    done.addEventListener("click", () => this.closeSheet());
    this.sheet.append(grab, columns, done);
    this.sheet.hidden = false;
    this.show(true);
  }

  pickOf(session) {
    return session.kind === "movie" ? { movie: session.key } : { show: session.key, season: session.season, episode: session.episode };
  }

  marksAt(now) {
    const session = this.session;
    if (!session) return;
    const segment = this.marks.segments.find((mark) => now >= mark.start && now < mark.end - 1);
    const credits = this.marks.credits;
    if (segment) {
      this.skip.textContent = segment.label;
      this.skip.dataset.to = String(segment.end);
      delete this.skip.dataset.next;
      this.skip.hidden = false;
    } else if (credits && now >= credits && session.next) {
      this.skip.hidden = true;
      this.startCountdown();
    } else if (credits && now >= credits && now < this.duration() - 3 && !session.next) {
      this.skip.textContent = "Skip credits";
      this.skip.dataset.to = String(this.duration() - 0.5);
      delete this.skip.dataset.next;
      this.skip.hidden = false;
    } else {
      this.skip.hidden = true;
      if (this.countdown && credits && now < credits) this.cancelCountdown();
    }
  }

  // Pauses once where someone this series is watched together with stopped.
  stopAt(from, now) {
    const stops = this.marks.stops || [];
    const stop = stops.find((each) => from < each.at && each.at <= now);
    if (!stop) return;
    this.marks.stops = stops.filter((each) => each !== stop);
    note(stop.said, `at ${clock(now)}`);
    this.pause();
    this.toast(stop.said);
  }

  startCountdown() {
    const session = this.session;
    if (this.countdown || this.countdownDismissed === session.id || !session.next) return;
    let left = COUNTDOWN;
    const next = session.next;
    this.upNext.innerHTML = "";
    const still = element("img", "", { src: `${api}${next.still}`, alt: "" });
    still.addEventListener("error", () => still.remove());
    const text = element("div", "pp-upnext-text");
    text.append(element("span", "pp-quiet", { text: `Next · ${next.label}` }), element("strong", "", { text: next.title || next.label }));
    const go = element("button", "pp-pill pp-strong", { type: "button", text: `Play in ${left}` });
    const stay = element("button", "pp-pill", { type: "button", text: "Watch credits" });
    go.addEventListener("click", () => this.playNext());
    stay.addEventListener("click", () => {
      this.countdownDismissed = session.id;
      this.cancelCountdown();
    });
    const row = element("div", "pp-row");
    row.append(go, stay);
    this.upNext.append(still, text, row);
    this.upNext.hidden = false;
    this.countdown = setInterval(() => {
      if (this.video.paused) return;
      left -= 1;
      go.textContent = `Play in ${left}`;
      if (left <= 0) this.playNext();
    }, 1000);
  }

  cancelCountdown() {
    clearInterval(this.countdown);
    this.countdown = null;
    this.upNext.hidden = true;
  }

  async playNext() {
    const session = this.session;
    if (!session || !session.next) return;
    this.cancelCountdown();
    const finished = this.video.ended ? "ended" : "stopped";
    const next = { show: session.key, season: session.next.season, episode: session.next.episode };
    this.started = false;
    this.carrying = false;
    this.pause();
    this.detach();
    this.session = null;
    this.replacing = session.id;
    this.skip.hidden = true;
    this.title(session.title, session.next.title ? `${session.next.label} · ${session.next.title}` : session.next.label);
    this.wait(next);
    await this.reportFor(session, finished);
    if (!this.closed) await command({ action: "play", ...next });
  }

  ended() {
    const session = this.session;
    if (!session) return;
    this.started = false;
    this.carrying = false;
    if (session.next && this.countdownDismissed !== session.id) {
      this.playNext();
    } else {
      this.report("ended");
      this.show(true);
    }
  }

  begin() {
    if (this.started || !this.session) return Promise.resolve();
    this.started = true;
    this.carrying = false;
    return this.report("start");
  }

  async blocked(session) {
    if (this.session !== session || this.closed) return;
    note("autoplay was blocked, waiting for a tap", `at ${clock(this.now())}`);
    this.show(true);
    await this.begin();
    if (this.session === session && this.video.paused) this.report("paused");
  }

  // Whether the video's clock says where it is: it reads 0 until it loads.
  placed() {
    return this.ready || (this.video.currentTime || 0) > 0;
  }

  position(session) {
    const current = session === this.session || this.session === null;
    return this.placed() && current ? session.offset + (this.video.currentTime || 0) - this.base : session.start;
  }

  async report(state, keepalive = false) {
    if (this.session) await this.reportFor(this.session, state, keepalive);
  }

  async reportFor(session, state, keepalive = false) {
    const body = { position: this.position(session), played: this.played, state };
    if (this.subtitlesFor === session) body.subtitles = { id: this.subtitle ?? null };
    try {
      const answer = await request("POST", `/api/stream/${session.id}/progress`, body, { keepalive });
      if (answer && Number.isFinite(answer.kept_from) && session === this.session) this.keptFrom = answer.kept_from;
      if (answer && answer.reopen && session === this.session && !this.closed && ["playing", "paused"].includes(state)) {
        note(`the sound setting changed; the stream opens again at ${clock(this.now())}`, answer.reopen);
        this.open(this.pickOf(session), { start: this.now(), audio: session.audio_index ?? undefined });
      }
    } catch (error) {
      const gone = error.status === 404 || transient(error);
      if (gone && !keepalive && session === this.session && !this.closed && !this.opening) this.awaitTv(`progress: ${error.message}`);
    }
  }

  trouble(detail) {
    if (this.resuming || this.awaiting || this.closed || this.waiting) return;
    if (!this.session) {
      this.failed(detail);
      return;
    }
    this.awaitTv(detail || "the video stopped loading");
  }

  // Asks the TV what it plays, again and again while it doesn't answer. A TV
  // app that restarted has a new boot, which adopt() answers by resuming
  // here; a TV that never restarted gets the stream reopened instead.
  async awaitTv(why) {
    if (this.awaiting || this.resuming || this.closed) return;
    this.awaiting = true;
    note("lost the TV's stream, asking the TV", why);
    let delay = FIRST_RETRY;
    const until = Date.now() + GIVE_UP;
    while (!this.closed && (Date.now() < until || closedForMaintenance())) {
      let view = null;
      try {
        view = await request("GET", screenPath());
      } catch {
        await sleep(delay);
        delay = Math.min(delay * 2, LONGEST_RETRY);
        continue;
      }
      this.awaiting = false;
      const before = boot;
      adopt(view);
      if (this.closed || this.resuming || this.opening || boot !== before) return;
      if (this.session && view.here.do === "open" && !this.recover()) this.failed(why);
      return;
    }
    this.awaiting = false;
    if (!this.closed) this.failed("The TV isn't answering");
  }

  // The TV app restarted under this phone, so its stream and its place on the
  // screen are gone: play the same title here again from where the video is,
  // unless something else plays by now.
  resumeHere() {
    if (this.resuming) return true;
    if (!this.pick || this.idling || this.closed || this.root.classList.contains("pp-failed")) return false;
    const session = this.session;
    const position = session ? this.position(session) : 0;
    const pick = this.pick;
    this.resuming = true;
    this.session = null;
    this.started = false;
    this.carrying = false;
    this.waiting = true;
    this.waitingSince = Date.now();
    note(`the TV app restarted; ${clock(position)} plays on here`, JSON.stringify(pick));
    (async () => {
      let delay = FIRST_RETRY;
      const until = Date.now() + GIVE_UP;
      while (!this.closed) {
        try {
          await request("POST", "/api/screen", { action: "resume", ...pick, position, device });
          this.resuming = false;
          adopt(await request("GET", screenPath()));
          return;
        } catch (error) {
          if (!transient(error) || (Date.now() > until && !closedForMaintenance())) {
            note("playing on here after the restart was refused", error.message);
            this.resuming = false;
            this.waiting = false;
            request("GET", screenPath()).then(adopt).catch(() => this.close(false));
            return;
          }
          await sleep(delay);
          delay = Math.min(delay * 2, LONGEST_RETRY);
        }
      }
      this.resuming = false;
    })();
    return true;
  }

  continueOnTv() {
    this.pause();
    command({ action: "flip", to: "tv", position: this.placed() ? this.now() : undefined });
  }

  fullscreen() {
    const video = this.video;
    if (video.webkitEnterFullscreen && !document.fullscreenEnabled) video.webkitEnterFullscreen();
    else if (document.fullscreenElement) document.exitFullscreen();
    else this.root.requestFullscreen?.().catch(() => video.webkitEnterFullscreen?.());
  }

  pictureInPicture() {
    const video = this.video;
    if (document.pictureInPictureElement) document.exitPictureInPicture();
    else if (video.requestPictureInPicture) video.requestPictureInPicture().catch(() => {});
    else video.webkitSetPresentationMode?.("picture-in-picture");
  }

  media(session) {
    if (!("mediaSession" in navigator)) return;
    const artwork = [{ src: `${location.origin}${api}${session.poster}`, sizes: "342x513", type: "image/jpeg" }];
    navigator.mediaSession.metadata = new MediaMetadata({
      title: session.kind === "show" ? session.caption : session.title,
      artist: session.kind === "show" ? session.title : "SemmelTV",
      album: "SemmelTV",
      artwork,
    });
    const handlers = {
      play: () => this.video.play(),
      pause: () => this.pause(),
      seekbackward: (details) => this.jump(-(details.seekOffset || JUMP)),
      seekforward: (details) => this.jump(details.seekOffset || JUMP),
      seekto: (details) => this.seekTo(details.seekTime),
      nexttrack: session.next ? () => this.playNext() : null,
    };
    for (const [action, handler] of Object.entries(handlers)) {
      try { navigator.mediaSession.setActionHandler(action, handler); } catch {}
    }
  }

  mediaState(state) {
    if ("mediaSession" in navigator) navigator.mediaSession.playbackState = state;
  }

  positionState() {
    if (!("mediaSession" in navigator) || !navigator.mediaSession.setPositionState) return;
    const now = Date.now();
    if (this.positionAt && now - this.positionAt < 4000) return;
    this.positionAt = now;
    const duration = this.duration();
    if (!duration) return;
    try {
      navigator.mediaSession.setPositionState({ duration, position: Math.max(0, Math.min(this.now(), duration)), playbackRate: this.video.playbackRate || 1 });
    } catch {}
  }

  recover() {
    const session = this.session;
    if (!session || this.closed || Date.now() - (this.recovered || 0) < 30000) return false;
    this.recovered = Date.now();
    this.open(this.pickOf(session), { start: this.position(session), audio: session.audio_index ?? undefined });
    return true;
  }

  toast(text) {
    const node = element("div", "pp-toast", { text });
    this.root.append(node);
    setTimeout(() => node.remove(), 6000);
  }

  failed(text) {
    if (window.semmeltvReport) window.semmeltvReport("error", "player failed", text || "no reason");
    this.root.classList.remove("pp-loading");
    this.root.classList.add("pp-failed");
    this.message.innerHTML = "";
    const words = text || "This video didn't play";
    this.message.append(element("p", "", { text: words }));
    const row = element("div", "pp-row");
    if (this.pick) {
      const tv = element("button", "pp-pill pp-strong", { type: "button", text: "Play on TV instead" });
      tv.addEventListener("click", async () => {
        const pick = this.pick;
        this.close(false);
        try { await request("POST", "/api/play", pick); } catch (error) { toast(error.message, "error"); }
      });
      row.append(tv);
    }
    const shut = element("button", "pp-pill", { type: "button", text: "Close" });
    shut.addEventListener("click", () => this.close(false));
    row.append(shut);
    this.message.append(row);
    this.show(true);
  }

  close(reportStop = true) {
    if (this.closed) return;
    this.closed = true;
    this.bakeoff.stop();
    const session = this.session;
    const reported = session && reportStop && (this.started || this.carrying) ? this.report(this.video.ended ? "ended" : "stopped", true) : Promise.resolve();
    const ending = session ? session.id : this.replacing;
    reported.finally(() => {
      if (reportStop) request("POST", "/api/screen", { action: "stop", device }, { keepalive: true }).catch(() => {});
      if (ending) request("DELETE", `/api/stream/${ending}`, undefined, { keepalive: true }).catch(() => {});
    });
    clearInterval(this.reportTimer);
    this.cancelCountdown();
    document.removeEventListener("visibilitychange", this.onHide);
    window.removeEventListener("pagehide", this.onLeave);
    window.removeEventListener("popstate", this.onPop);
    if (ownEntry()) history.back();
    document.removeEventListener("keydown", this.onKey, true);
    document.removeEventListener("keyup", this.onKeyUp, true);
    window.removeEventListener("pointermove", this.onDragMove);
    window.removeEventListener("pointerup", this.onDragEnd);
    window.removeEventListener("pointercancel", this.onDragEnd);
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    this.pause();
    clearTimeout(this.cueTimer);
    if (this.hls) this.hls.destroy();
    this.video.removeAttribute("src");
    this.video.load();
    if ("mediaSession" in navigator) navigator.mediaSession.metadata = null;
    html.classList.remove("pp-open");
    this.root.classList.remove("pp-in");
    setTimeout(() => this.root.remove(), 300);
    if (player === this) player = null;
  }
}

let hlsLoading = null;

function loadHls() {
  if (window.Hls) return Promise.resolve(window.Hls);
  if (!hlsLoading) {
    hlsLoading = new Promise((resolve, reject) => {
      const script = element("script", "", { src: HLS_JS, crossorigin: "anonymous" });
      script.addEventListener("load", () => (window.Hls && window.Hls.isSupported() ? resolve(window.Hls) : reject(new Error("hls.js unsupported"))));
      script.addEventListener("error", reject);
      document.head.append(script);
    });
  }
  return hlsLoading;
}

// timeline functions: begin
// An hls.js fragment loader that passes each init segment through
// trimEmptyEdits.
function initTrimmingLoader(Hls) {
  return class extends Hls.DefaultConfig.loader {
    load(context, config, callbacks) {
      if (context.frag?.sn === "initSegment") {
        const { onSuccess } = callbacks;
        callbacks = {
          ...callbacks,
          onSuccess: (response, ...rest) => {
            if (response.data instanceof ArrayBuffer) trimEmptyEdits(new Uint8Array(response.data));
            onSuccess(response, ...rest);
          },
        };
      }
      super.load(context, config, callbacks);
    }
  };
}

// A stream that opens in the middle of a file keeps the file's times: ffmpeg
// writes them into the init segment as edit lists that start every track with
// an empty edit, as long as the part of the file before the stream. Firefox
// plays those edits and hls.js does not, so the two would place the video
// minutes apart. This shortens each track's empty edit, in place, by the
// part all tracks share, which starts the stream at 0 for both and keeps the
// tracks' times relative to each other.
function trimEmptyEdits(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const children = (start, end) => {
    const found = [];
    for (let at = start; at + 8 <= end;) {
      const size = view.getUint32(at);
      if (size < 8 || at + size > end) break;
      found.push({ type: String.fromCharCode(...bytes.subarray(at + 4, at + 8)), body: at + 8, end: at + size });
      at += size;
    }
    return found;
  };
  const moov = children(0, bytes.length).find((box) => box.type === "moov");
  if (!moov) return;
  const leads = [];
  for (const trak of children(moov.body, moov.end).filter((box) => box.type === "trak")) {
    const edts = children(trak.body, trak.end).find((box) => box.type === "edts");
    const elst = edts && children(edts.body, edts.end).find((box) => box.type === "elst");
    const wide = elst && bytes[elst.body] === 1;
    const first = elst && view.getUint32(elst.body + 4) > 0 ? elst.body + 8 : null;
    const empty = first !== null && (wide ? view.getBigInt64(first + 8) === -1n : view.getInt32(first + 4) === -1);
    if (!empty) {
      leads.push({ length: 0 });
      continue;
    }
    const length = wide ? Number(view.getBigUint64(first)) : view.getUint32(first);
    leads.push({ length, at: first, wide });
  }
  const shared = Math.min(...leads.map((lead) => lead.length));
  if (!(shared > 0)) return;
  for (const lead of leads) {
    if (lead.wide) view.setBigUint64(lead.at, BigInt(lead.length - shared));
    else view.setUint32(lead.at, lead.length - shared);
  }
}
// timeline functions: end

function openPlayer(pick, options = {}) {
  if (!player || player.closed) player = new Player();
  else if (player.mini) player.expand();
  if (options.waiting) player.wait(pick);
  else player.open(pick, options);
}

function pickFrom(target) {
  const movie = target.dataset.playMovie;
  const show = target.dataset.playShow;
  if (movie) return { movie };
  if (!show) return null;
  const pick = { show };
  if (target.dataset.season && target.dataset.episode) {
    pick.season = Number(target.dataset.season);
    pick.episode = Number(target.dataset.episode);
  }
  return pick;
}

window.addEventListener("click", (event) => {
  if (event.target.closest("#mode") || event.target.closest('[data-handoff="phone"]')) {
    event.preventDefault();
    event.stopImmediatePropagation();
    flip();
    return;
  }
  if (screen.kind === "tv" && controlsTv) return;
  const target = event.target.closest("[data-play-movie], [data-play-show]");
  if (!target) return;
  const pick = pickFrom(target);
  if (!pick) return;
  event.preventDefault();
  event.stopImmediatePropagation();
  playHere(pick);
}, true);

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState !== "visible") return;
  listen();
  request("GET", screenPath()).then(adopt).catch(() => {});
});

window.addEventListener("semmeltv:reconnect", relisten);

window.addEventListener("semmeltv:maintenance", (event) => {
  if (!player || player.closed) return;
  if (event.detail.closed) {
    player.heldForMaintenance = player.playing();
    if (player.heldForMaintenance) player.pause();
  } else if (player.heldForMaintenance) {
    player.heldForMaintenance = false;
    player.video.play()?.catch(() => {});
  }
});

new MutationObserver(render).observe(html, { attributes: true, childList: true });
new MutationObserver(render).observe(document.body, { childList: true, subtree: true });
render();
listen();
request("GET", screenPath()).then(adopt).catch(() => {});
if (location.hash === PRACTICE) practiceBake();
window.addEventListener("hashchange", () => { if (location.hash === PRACTICE) practiceBake(); });
