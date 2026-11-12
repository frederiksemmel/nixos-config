const api = document.documentElement.dataset.api || "";
const MODE_KEY = "semmeltv.mode";
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
let mode = "tv";
let player = null;
let noteTimer = 0;

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

function note(text, hold = 4500) {
  const target = document.getElementById("mode-note");
  if (!target) return;
  target.textContent = text;
  clearTimeout(noteTimer);
  if (text && hold) noteTimer = setTimeout(() => { target.textContent = ""; }, hold);
}

function capabilities() {
  const probe = document.createElement("video");
  const native = probe.canPlayType("application/vnd.apple.mpegurl") !== "";
  const MSE = window.ManagedMediaSource || window.MediaSource;
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

function applyMode(next) {
  mode = next;
  html.dataset.mode = next;
  const toggle = document.getElementById("mode");
  if (toggle) {
    toggle.dataset.mode = next;
    toggle.setAttribute("aria-pressed", String(next === "phone"));
    toggle.setAttribute("aria-label", next === "phone" ? "Watch on the TV" : "Watch on this phone");
  }
}

async function switchMode() {
  if (mode === "phone") {
    applyMode("tv");
    store(MODE_KEY, "tv");
    note("Play goes to the TV", 2500);
    return;
  }
  if (!capabilities().playable) {
    note("This browser can't play the videos");
    return;
  }
  try {
    await request("GET", "/api/stream/access");
  } catch (error) {
    note(error.status === 403 ? "Watching on this phone needs Tailscale on" : error.message);
    return;
  }
  applyMode("phone");
  store(MODE_KEY, "phone");
  note("Play stays on this phone", 2500);
  offerHandoff();
}

async function offerHandoff() {
  let playing = null;
  try {
    playing = (await request("GET", "/api/playback")).now_playing;
  } catch {
    return;
  }
  if (!playing || (!playing.movie && !playing.show)) return;
  document.querySelector(".pp-handoff")?.remove();
  const card = element("div", "pp-handoff", { role: "dialog", "aria-label": "Continue on this phone" });
  const text = element("p", "", { text: `The TV is playing ${playing.title}${playing.caption ? ` · ${playing.caption}` : ""}` });
  const move = element("button", "pp-pill pp-strong", { type: "button", text: "Watch here" });
  const stay = element("button", "pp-pill", { type: "button", text: "Leave it" });
  card.append(text, element("div", "pp-row"));
  card.lastChild.append(move, stay);
  document.body.append(card);
  stay.addEventListener("click", () => card.remove());
  move.addEventListener("click", () => {
    card.remove();
    continueOnPhone();
  });
  setTimeout(() => card.remove(), 15000);
}

async function continueOnPhone() {
  document.querySelector("#sheet .sheet-close")?.click();
  try {
    const now = (await request("GET", "/api/playback")).now_playing;
    if (!now || (!now.movie && !now.show)) {
      note("The TV isn't playing anything");
      return;
    }
    await request("POST", "/api/playback/stop");
    const pick = now.movie ? { movie: now.movie } : { show: now.show, season: now.season, episode: now.episode };
    openPlayer(pick, { start: now.position });
  } catch (error) {
    note(error.message);
  }
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
    const top = element("div", "pp-top");
    top.append(this.closeButton, this.titleText, this.airplay, this.pip);

    this.backButton = button("pp-icon pp-jump", `Back ${JUMP} seconds`, `${icons.back}<span>${JUMP}</span>`);
    this.playButton = button("pp-icon pp-play", "Play", icons.play);
    this.aheadButton = button("pp-icon pp-jump", `Ahead ${JUMP} seconds`, `${icons.ahead}<span>${JUMP}</span>`);
    const middle = element("div", "pp-middle");
    middle.append(this.backButton, this.playButton, this.aheadButton);

    this.elapsed = element("span", "pp-time", { text: "0:00" });
    this.remaining = element("span", "pp-time", { text: "" });
    this.scrub = element("input", "pp-scrub", { type: "range", min: "0", max: "1", step: "any", value: "0", "aria-label": "Position" });
    this.bubble = element("span", "pp-bubble");
    const line = element("div", "pp-line");
    const track = element("div", "pp-track");
    track.append(this.scrub, this.bubble);
    line.append(this.elapsed, track, this.remaining);

    this.tracksButton = element("button", "pp-pill", { type: "button", html: `${icons.tracks}<span>Audio &amp; subtitles</span>` });
    this.nextButton = element("button", "pp-pill", { type: "button", html: `${icons.next}<span>Next episode</span>` });
    this.tvButton = element("button", "pp-pill", { type: "button", html: `${icons.tv}<span>Continue on TV</span>` });
    this.fullButton = button("pp-icon pp-full", "Full screen", icons.full);
    this.nextButton.hidden = true;
    const actions = element("div", "pp-actions");
    actions.append(this.tracksButton, this.nextButton, this.tvButton, this.fullButton);

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
    video.addEventListener("error", () => this.failed(video.error && video.error.message));

    this.reportTimer = setInterval(() => {
      if (this.started && !video.paused) this.report("playing");
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
    this.remaining.textContent = `-${clock(duration - at)}`;
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

  async open(pick, options = {}) {
    this.pick = pick;
    this.root.classList.add("pp-loading");
    this.message.textContent = "";
    this.root.classList.remove("pp-failed");
    const previous = this.session;
    const body = { ...pick, client: this.capabilities.client };
    if (options.start !== undefined) body.start = options.start;
    if (options.audio !== undefined) body.audio = options.audio;
    if (previous) body.replaces = previous.id;
    let session;
    try {
      session = await request("POST", "/api/stream/open", body);
    } catch (error) {
      this.failed(error.message, error.status);
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
      hls.on(Hls.Events.ERROR, (_, data) => { if (data.fatal) this.failed(data.details); });
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
    await this.report(this.video.ended ? "ended" : "stopped");
    this.open({ show: session.key, season: session.next.season, episode: session.next.episode }, { start: 0 });
  }

  ended() {
    const session = this.session;
    if (!session) return;
    this.report("ended");
    this.started = false;
    if (session.next && this.countdownDismissed !== session.id) {
      this.playNext();
    } else {
      this.show(true);
    }
  }

  async report(state, keepalive = false) {
    const session = this.session;
    if (!session) return;
    const body = { position: this.video.currentTime || 0, played: this.played, state };
    try {
      await request("POST", `/api/stream/${session.id}/progress`, body, { keepalive });
    } catch {}
  }

  async continueOnTv() {
    const session = this.session;
    if (!session) return;
    this.video.pause();
    await this.report("stopped");
    const pick = this.pickOf(session);
    this.close(false);
    try {
      await request("POST", "/api/play", pick);
      note("Playing on the TV", 3000);
    } catch (error) {
      note(error.message);
    }
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

  toast(text) {
    const node = element("div", "pp-toast", { text });
    this.root.append(node);
    setTimeout(() => node.remove(), 6000);
  }

  failed(text, status) {
    this.root.classList.remove("pp-loading");
    this.root.classList.add("pp-failed");
    this.message.innerHTML = "";
    const words = status === 403 ? "Watching on this phone needs Tailscale on" : (text || "This video didn't play");
    this.message.append(element("p", "", { text: words }));
    const row = element("div", "pp-row");
    if (this.pick) {
      const tv = element("button", "pp-pill pp-strong", { type: "button", text: "Play on TV instead" });
      tv.addEventListener("click", async () => {
        const pick = this.pick;
        this.close(false);
        try { await request("POST", "/api/play", pick); note("Playing on the TV", 3000); } catch (error) { note(error.message); }
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
  if (player) player.close();
  player = new Player();
  player.open(pick, options);
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
  const toggle = event.target.closest("#mode");
  if (toggle) {
    event.preventDefault();
    event.stopImmediatePropagation();
    switchMode();
    return;
  }
  if (mode !== "phone") return;
  if (event.target.closest('[data-handoff="phone"]')) {
    event.preventDefault();
    event.stopImmediatePropagation();
    continueOnPhone();
    return;
  }
  const target = event.target.closest("[data-play-movie], [data-play-show]");
  if (!target) return;
  const pick = pickFrom(target);
  if (!pick) return;
  event.preventDefault();
  event.stopImmediatePropagation();
  openPlayer(pick);
}, true);

applyMode(stored(MODE_KEY) === "phone" ? "phone" : "tv");
new MutationObserver(() => {
  const toggle = document.getElementById("mode");
  if (toggle && toggle.dataset.mode !== mode) applyMode(mode);
}).observe(document.body, { childList: true, subtree: true });
