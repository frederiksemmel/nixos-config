const api = document.documentElement.dataset.api || "";
const HLS_JS = "https://cdn.jsdelivr.net/npm/hls.js@1.5.17/dist/hls.min.js";
const REPORT_EVERY = 10000;
const HIDE_AFTER = 3200;
const COUNTDOWN = 10;
const JUMP = 10;
const SWIPE_CLOSE = 110;

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
  airplay: '<svg viewBox="0 0 24 24"><path d="M6 17H4.5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h15a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H18"/><path class="fill" d="m12 14 5 6H7z"/></svg>',
};

const html = document.documentElement;
const DEVICE_KEY = "semmeltv.device";
const RECONNECT = 2000;
let player = null;
let screen = { kind: "tv", device: null, play: null, notice: null };
let tvPlaying = null;
let seenNotice = undefined;
let socket = null;
let pending = null;

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

function samePick(a, b) {
  if (!a || !b) return false;
  if (a.movie || b.movie) return a.movie === b.movie;
  return a.show === b.show && a.season === b.season && a.episode === b.episode;
}

function pickOfPlay(play) {
  return play.kind === "movie" ? { movie: play.movie } : { show: play.show, season: play.season, episode: play.episode };
}

function pickOfPlaying(playing) {
  if (!playing || (!playing.movie && !playing.show)) return null;
  return playing.movie ? { movie: playing.movie } : { show: playing.show, season: playing.season, episode: playing.episode };
}

function ours(view) {
  return Boolean(view.device && view.device.id === device.id);
}

function render() {
  const kind = isLaptop() ? "laptop" : "phone";
  if (html.dataset.device !== kind) html.dataset.device = kind;
  const mode = screen.kind === "tv" ? "tv" : "phone";
  if (html.dataset.mode !== mode) html.dataset.mode = mode;
  if (!html.hasAttribute("data-phone-ready")) html.dataset.phoneReady = "";
  if (player && !player.closed && !html.classList.contains("pp-open")) html.classList.add("pp-open");
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

function adopt(view) {
  screen = view;
  const notice = view.notice;
  if (seenNotice === undefined) {
    seenNotice = notice ? notice.id : null;
  } else if (notice && notice.id !== seenNotice) {
    seenNotice = notice.id;
    toast(notice.text, notice.tone === "error" ? "error" : "info");
  }
  render();
  reconcile();
}

function reconcile() {
  const view = screen;
  const play = view.play;
  const wanted = ours(view) && play && ["moving", "starting", "playing"].includes(play.state);
  if (!wanted) {
    if (player && !player.closed && !player.fresh()) {
      if (ours(view) && view.kind === "phone" && !play) player.idle();
      else player.close(false);
    }
    return;
  }
  const pick = pickOfPlay(play);
  if (player && !player.closed && player.shows(pick)) return;
  openPlayer(pick, { start: play.position });
}

function listen() {
  if (socket && socket.readyState <= 1) return;
  const base = api ? api.replace(/^http/, "ws") : location.origin.replace(/^http/, "ws");
  socket = new WebSocket(`${base}/api/events`);
  socket.addEventListener("open", () => {
    request("GET", "/api/screen").then(adopt).catch(() => {});
  });
  socket.addEventListener("message", (message) => {
    let event;
    try { event = JSON.parse(message.data); } catch { return; }
    if (event.type === "screen") adopt(event.screen);
    if (event.type === "playback") tvPlaying = event.now_playing;
  });
  socket.addEventListener("close", () => {
    socket = null;
    setTimeout(listen, RECONNECT);
  });
}

async function command(body) {
  try {
    await request("POST", "/api/screen", { ...body, device });
    const view = await request("GET", "/api/screen");
    pending = null;
    adopt(view);
  } catch (error) {
    pending = null;
    paintSwitch();
    toast(error.message, "error");
    if (player && player.waiting) player.close(false);
    request("GET", "/api/screen").then(adopt).catch(() => {});
  }
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
    const pick = pickOfPlaying(tvPlaying);
    if (pick) openPlayer(pick, { waiting: true });
    pending = "phone";
    paintSwitch();
    command({ action: "flip", to: "phone" });
  } else {
    const position = player && !player.closed && ours(screen) && player.video.currentTime > 0 ? player.video.currentTime : undefined;
    pending = "tv";
    paintSwitch();
    command({ action: "flip", to: "tv", position });
  }
}

function playHere(pick) {
  if (!canWatchHere()) return;
  openPlayer(pick, { waiting: true });
  command({ action: "play", ...pick });
}

class Player {
  constructor() {
    this.capabilities = capabilities();
    this.root = element("div", "pp", { role: "dialog", "aria-label": "Player" });
    this.video = element("video", "pp-video", { playsinline: "", "webkit-playsinline": "", "x-webkit-airplay": "allow", preload: "auto" });
    this.video.disablePictureInPicture = false;
    this.layer = element("div", "pp-layer");
    this.build();
    document.body.append(this.root);
    html.classList.add("pp-open");
    this.session = null;
    this.hls = null;
    this.played = 0;
    this.lastTime = null;
    this.started = false;
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
    this.closeButton = button("pp-icon pp-close", "Close", icons.close);
    this.titleText = element("div", "pp-title");
    this.airplay = button("pp-icon pp-airplay", "AirPlay", icons.airplay);
    this.pip = button("pp-icon pp-pip", "Picture in Picture", icons.pip);
    this.airplay.hidden = true;
    this.pip.hidden = !(document.pictureInPictureEnabled || this.video.webkitSupportsPresentationMode);
    this.tvButton = button("pp-icon pp-tv", "Continue on TV", icons.tv);
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
    this.root.append(this.video, this.spinner, this.ripple, this.layer, this.skip, this.upNext, this.sheet, this.message);
  }

  bind() {
    const video = this.video;
    this.closeButton.addEventListener("click", () => this.close());
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
      else if (Number.isFinite(target)) video.currentTime = target;
    });
    if (window.WebKitPlaybackTargetAvailabilityEvent) {
      video.addEventListener("webkitplaybacktargetavailabilitychanged", (event) => {
        this.airplay.hidden = event.availability !== "available";
      });
    }

    let scrubbing = false;
    this.scrub.addEventListener("input", () => {
      scrubbing = true;
      const at = Number(this.scrub.value);
      this.bubble.textContent = clock(at);
      this.bubble.style.left = `${(at / (Number(this.scrub.max) || 1)) * 100}%`;
      this.bubble.classList.add("pp-shown");
      this.elapsed.textContent = clock(at);
      this.show(true);
    });
    this.scrub.addEventListener("change", () => {
      scrubbing = false;
      this.bubble.classList.remove("pp-shown");
      video.currentTime = Number(this.scrub.value);
      this.show();
    });

    video.addEventListener("timeupdate", () => {
      const now = video.currentTime;
      if (this.lastTime !== null && !video.seeking && !video.paused) {
        const step = now - this.lastTime;
        if (step > 0 && step < 2) this.played += step;
      }
      this.lastTime = now;
      if (!scrubbing) this.paint();
      this.marksAt(now);
      this.positionState();
    });
    video.addEventListener("seeking", () => { this.lastTime = null; });
    video.addEventListener("waiting", () => this.root.classList.add("pp-loading"));
    video.addEventListener("canplay", () => this.root.classList.remove("pp-loading"));
    video.addEventListener("playing", () => {
      this.root.classList.remove("pp-loading");
      this.playButton.innerHTML = icons.pause;
      this.playButton.setAttribute("aria-label", "Pause");
      if (!this.started) {
        this.started = true;
        this.report("start");
      }
      this.show();
      this.mediaState("playing");
    });
    video.addEventListener("pause", () => {
      this.playButton.innerHTML = icons.play;
      this.playButton.setAttribute("aria-label", "Play");
      this.show(true);
      if (this.started && !video.ended) this.report("paused");
      this.mediaState("paused");
    });
    video.addEventListener("ended", () => this.ended());
    video.addEventListener("error", () => { if (!this.recover()) this.failed(video.error && video.error.message); });

    this.reportTimer = setInterval(() => {
      if (this.started && !this.idling) this.report(video.paused ? "paused" : "playing");
    }, REPORT_EVERY);
    this.onHide = () => {
      if (document.visibilityState === "hidden" && this.started) this.report(video.paused ? "paused" : "playing", true);
    };
    document.addEventListener("visibilitychange", this.onHide);
    this.onLeave = () => { if (this.started) this.report("stopped", true); };
    window.addEventListener("pagehide", this.onLeave);

    let taps = [];
    let touchStart = null;
    this.layer.addEventListener("touchstart", (event) => {
      if (event.target.closest("button, input")) return;
      const touch = event.touches[0];
      touchStart = { x: touch.clientX, y: touch.clientY, time: Date.now() };
    }, { passive: true });
    this.layer.addEventListener("touchmove", (event) => {
      if (!touchStart) return;
      const dy = event.touches[0].clientY - touchStart.y;
      if (dy > 0) this.root.style.transform = `translateY(${dy * 0.6}px)`;
    }, { passive: true });
    this.layer.addEventListener("touchend", (event) => {
      if (!touchStart) return;
      const touch = event.changedTouches[0];
      const dy = touch.clientY - touchStart.y;
      const dx = touch.clientX - touchStart.x;
      touchStart = null;
      this.root.style.transform = "";
      if (dy > SWIPE_CLOSE && Math.abs(dx) < dy) {
        event.preventDefault();
        this.close();
      }
    });
    this.layer.addEventListener("click", (event) => {
      if (event.target.closest("button, input")) return;
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
    this.root.addEventListener("keydown", (event) => {
      if (event.key === " ") { event.preventDefault(); this.toggle(); }
      if (event.key === "ArrowLeft") this.jump(-JUMP);
      if (event.key === "ArrowRight") this.jump(JUMP);
      if (event.key === "Escape") this.close();
    });
  }

  show(stay = false) {
    this.root.classList.add("pp-controls");
    clearTimeout(this.hideTimer);
    if (stay || this.video.paused) return;
    this.hideTimer = setTimeout(() => {
      if (this.sheet.hidden) this.root.classList.remove("pp-controls");
    }, HIDE_AFTER);
  }

  hide() {
    clearTimeout(this.hideTimer);
    if (!this.video.paused && this.sheet.hidden) this.root.classList.remove("pp-controls");
  }

  paint() {
    const duration = this.duration();
    const at = this.video.currentTime || 0;
    this.scrub.max = String(duration || 1);
    this.scrub.value = String(at);
    this.scrub.style.setProperty("--done", `${duration ? (at / duration) * 100 : 0}%`);
    this.elapsed.textContent = clock(at);
    this.remaining.textContent = duration ? `-${clock(duration - at)}` : "–:––";
  }

  duration() {
    const own = this.video.duration;
    return Number.isFinite(own) && own > 0 ? own : (this.session ? this.session.duration : 0);
  }

  toggle() {
    if (this.video.paused) this.video.play().catch(() => {});
    else this.video.pause();
  }

  jump(seconds, side = 0) {
    const video = this.video;
    video.currentTime = Math.max(0, Math.min(this.duration() - 1, video.currentTime + seconds));
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
    this.video.pause();
    this.root.classList.remove("pp-loading");
    this.root.classList.add("pp-failed");
    this.message.innerHTML = "";
    this.message.append(element("p", "", { text: `Stopped at ${clock(this.video.currentTime)}` }));
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

  async open(pick, options = {}) {
    this.pick = pick;
    this.waiting = false;
    this.idling = false;
    this.root.classList.add("pp-loading");
    this.message.textContent = "";
    this.root.classList.remove("pp-failed");
    const previous = this.session;
    const body = { ...pick, client: this.capabilities.client, device };
    if (options.start !== undefined) body.start = options.start;
    if (options.audio !== undefined) body.audio = options.audio;
    if (previous) body.replaces = previous.id;
    let session;
    try {
      session = await request("POST", "/api/stream/open", body);
    } catch (error) {
      command({ action: "failed", reason: error.message });
      this.failed(error.message);
      return;
    }
    if (this.closed) {
      request("DELETE", `/api/stream/${session.id}`).catch(() => {});
      return;
    }
    this.session = session;
    this.played = 0;
    this.started = false;
    this.lastTime = null;
    this.marks = { segments: [], credits: session.credits };
    this.cancelCountdown();
    this.titleText.innerHTML = "";
    this.titleText.append(element("strong", "", { text: session.title }));
    if (session.kind === "show") this.titleText.append(element("span", "", { text: session.caption }));
    this.nextButton.hidden = !session.next;
    this.media(session);
    this.load(session);
    request("GET", `/api/stream/${session.id}/marks`).then((marks) => {
      if (this.session === session && marks) this.marks = marks;
    }).catch(() => {});
    if (session.heavy) this.toast(`This file streams at ${(session.bitrate / 1e6).toFixed(1)} Mbit/s, about ${(session.bitrate * 3600 / 8e9).toFixed(1)} GB an hour on mobile data`);
    if (session.converted_audio) this.toast("The sound is converted to stereo for this browser");
  }

  load(session) {
    const video = this.video;
    const source = `${api}${session.playlist}`;
    const begin = () => {
      if (session.start > 1 && Math.abs(video.currentTime - session.start) > 2) video.currentTime = session.start;
      this.applyDefaultSubtitles(session);
      video.play().catch(() => this.show(true));
    };
    if (this.hls) { this.hls.destroy(); this.hls = null; }
    if (this.capabilities.native) {
      video.src = source;
      video.addEventListener("loadedmetadata", begin, { once: true });
      video.load();
      return;
    }
    loadHls().then((Hls) => {
      if (this.closed || this.session !== session) return;
      const hls = new Hls({ startPosition: session.start, maxBufferLength: 30, backBufferLength: 30 });
      this.hls = hls;
      hls.on(Hls.Events.ERROR, (_, data) => { if (data.fatal && !this.recover()) this.failed(data.details); });
      hls.on(Hls.Events.MANIFEST_PARSED, () => video.play().catch(() => this.show(true)));
      hls.on(Hls.Events.SUBTITLE_TRACKS_UPDATED, () => this.applyDefaultSubtitles(session));
      hls.loadSource(source);
      hls.attachMedia(video);
    }).catch(() => this.failed("The player library didn't load"));
  }

  textTracks() {
    return Array.from(this.video.textTracks || []).filter((track) => track.kind === "subtitles" || track.kind === "captions" || track.kind === "forced");
  }

  applyDefaultSubtitles(session) {
    const wanted = session.subtitles.find((subtitle) => subtitle.default && subtitle.supported);
    if (!wanted) return;
    this.chooseSubtitle(wanted.id);
  }

  chooseSubtitle(id) {
    const session = this.session;
    const wanted = id === null ? null : session.subtitles.find((subtitle) => subtitle.id === id);
    this.subtitle = wanted ? wanted.id : null;
    if (this.hls) {
      const index = wanted ? this.hls.subtitleTracks.findIndex((track) => track.name === wanted.name || track.url?.endsWith(`/${wanted.id}.m3u8`)) : -1;
      this.hls.subtitleTrack = index;
      this.hls.subtitleDisplay = index >= 0;
      return;
    }
    const tracks = this.textTracks();
    const supported = session.subtitles.filter((subtitle) => subtitle.supported);
    for (const track of tracks) {
      const position = tracks.indexOf(track);
      const matches = wanted && (track.label === wanted.name || (supported[position] && supported[position].id === wanted.id && !tracks.some((other) => other.label === wanted.name)));
      track.mode = matches ? "showing" : "disabled";
    }
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
        if (track.index !== session.audio_index) this.open(this.pickOf(session), { audio: track.index, start: this.video.currentTime });
      });
      audio.append(choice);
    }
    const subtitles = element("section", "");
    subtitles.append(element("h3", "", { text: "Subtitles" }));
    const off = element("button", "pp-choice", { type: "button", text: "Off", "aria-pressed": String(this.subtitle == null) });
    off.addEventListener("click", () => { this.chooseSubtitle(null); this.sheet.hidden = true; });
    subtitles.append(off);
    for (const subtitle of session.subtitles) {
      const choice = element("button", "pp-choice", { type: "button", text: subtitle.name, "aria-pressed": String(this.subtitle === subtitle.id) });
      choice.disabled = !subtitle.supported;
      choice.addEventListener("click", () => { this.chooseSubtitle(subtitle.id); this.sheet.hidden = true; });
      subtitles.append(choice);
    }
    if (session.subtitle_delay) subtitles.append(element("p", "pp-quiet", { text: `Shifted ${session.subtitle_delay > 0 ? "later" : "earlier"} by ${Math.abs(session.subtitle_delay).toFixed(1)} s, as on the TV` }));
    columns.append(audio, subtitles);
    const done = element("button", "pp-pill pp-strong pp-done", { type: "button", text: "Done" });
    done.addEventListener("click", () => { this.sheet.hidden = true; this.show(); });
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
    this.wait(next);
    await command({ action: "play", ...next });
    this.reportFor(session, finished);
  }

  ended() {
    const session = this.session;
    if (!session) return;
    this.started = false;
    if (session.next && this.countdownDismissed !== session.id) {
      this.playNext();
    } else {
      this.report("ended");
      this.show(true);
    }
  }

  async report(state, keepalive = false) {
    if (this.session) await this.reportFor(this.session, state, keepalive);
  }

  async reportFor(session, state, keepalive = false) {
    const body = { position: this.video.currentTime || 0, played: this.played, state };
    try {
      await request("POST", `/api/stream/${session.id}/progress`, body, { keepalive });
    } catch {}
  }

  continueOnTv() {
    this.video.pause();
    command({ action: "flip", to: "tv", position: this.video.currentTime || undefined });
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
      pause: () => this.video.pause(),
      seekbackward: (details) => this.jump(-(details.seekOffset || JUMP)),
      seekforward: (details) => this.jump(details.seekOffset || JUMP),
      seekto: (details) => { this.video.currentTime = details.seekTime; },
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
      navigator.mediaSession.setPositionState({ duration, position: Math.min(this.video.currentTime, duration), playbackRate: this.video.playbackRate || 1 });
    } catch {}
  }

  recover() {
    const session = this.session;
    if (!session || this.closed || Date.now() - (this.recovered || 0) < 30000) return false;
    this.recovered = Date.now();
    const at = this.video.currentTime > 1 ? this.video.currentTime : session.start;
    this.open(this.pickOf(session), { start: at, audio: session.audio_index ?? undefined });
    return true;
  }

  toast(text) {
    const node = element("div", "pp-toast", { text });
    this.root.append(node);
    setTimeout(() => node.remove(), 6000);
  }

  failed(text) {
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
    const session = this.session;
    if (session && reportStop && this.started) this.report(this.video.ended ? "ended" : "stopped", true);
    if (session) request("DELETE", `/api/stream/${session.id}`, undefined, { keepalive: true }).catch(() => {});
    clearInterval(this.reportTimer);
    this.cancelCountdown();
    document.removeEventListener("visibilitychange", this.onHide);
    window.removeEventListener("pagehide", this.onLeave);
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    this.video.pause();
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

function openPlayer(pick, options = {}) {
  if (!player || player.closed) player = new Player();
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
  if (screen.kind === "tv") return;
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
  request("GET", "/api/screen").then(adopt).catch(() => {});
});

new MutationObserver(render).observe(html, { attributes: true });
new MutationObserver(paintSwitch).observe(document.body, { childList: true, subtree: true });
render();
listen();
request("GET", "/api/screen").then(adopt).catch(() => {});
