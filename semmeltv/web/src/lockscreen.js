// Lock-screen controls for the TV. A phone shows play, pause and seek on its
// lock screen only for a page that plays sound, so once a tap on the remote
// or a play button for the TV engages it, the page plays a loop of silence
// while the TV plays, and the Media Session carries the TV's title and
// artwork and sends the lock screen's keys to the TV. It lets go when the
// phone plays the video itself, when the TV has played nothing for a while,
// and when something else on the phone takes the sound.
(() => {
  const api = document.documentElement.dataset.api || "";
  const html = document.documentElement;
  const RATE = 8000;
  const SECONDS = 30;
  const IDLE_FOR = 30000;
  const DRIFT = 3;
  const JUMP = 10;

  let audio = null;
  let engaged = false;
  let ownPause = false;
  let metadata = null;
  let described = null;
  let placed = null;
  let idleTimer = 0;
  let nextFor = null;
  let next = null;

  const report = (level, message, detail) => window.semmeltvReport?.(level, message, detail);

  function silence() {
    const samples = RATE * SECONDS;
    const bytes = new Uint8Array(44 + samples);
    const view = new DataView(bytes.buffer);
    const text = (at, word) => [...word].forEach((letter, index) => { bytes[at + index] = letter.charCodeAt(0); });
    text(0, "RIFF");
    view.setUint32(4, 36 + samples, true);
    text(8, "WAVE");
    text(12, "fmt ");
    view.setUint32(16, 16, true);
    view.setUint16(20, 1, true);
    view.setUint16(22, 1, true);
    view.setUint32(24, RATE, true);
    view.setUint32(28, RATE, true);
    view.setUint16(32, 1, true);
    view.setUint16(34, 8, true);
    text(36, "data");
    view.setUint32(40, samples, true);
    bytes.fill(128, 44);
    return URL.createObjectURL(new Blob([bytes], { type: "audio/wav" }));
  }

  function eligible() {
    return "mediaSession" in navigator
      && html.hasAttribute("data-phone-ready")
      && html.dataset.device === "phone"
      && html.dataset.mode === "tv"
      && !html.classList.contains("pp-open");
  }

  function tv() {
    const state = document.querySelector(".now-state");
    if (!state || !state.dataset.key) return null;
    const rail = document.querySelector(".scrub-rail");
    const art = document.querySelector(".bar-art img");
    const [movie, show, season, episode] = state.dataset.key.split("|");
    return {
      key: state.dataset.key,
      movie,
      show,
      season: Number(season),
      episode: Number(episode),
      title: document.querySelector(".bar-text strong")?.textContent || "",
      line: document.querySelector(".bar-text span")?.textContent || "",
      art: art ? new URL(art.getAttribute("src"), location.href).href : null,
      loading: state.dataset.loading === "1",
      paused: state.classList.contains("paused") && state.dataset.loading !== "1",
      position: Number(rail?.dataset.position) || 0,
      duration: Number(rail?.dataset.duration) || 0,
      speed: Number(state.dataset.speed) || 1,
    };
  }

  function command(path, body = {}) {
    fetch(`${api}${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) })
      .then((response) => {
        if (!response.ok) report("warn", "a lock-screen key failed", `${path} answered ${response.status}`);
      })
      .catch((error) => report("warn", "a lock-screen key failed", `${path}: ${error.message}`));
  }

  function sound() {
    if (!audio.paused) return;
    audio.play().catch((error) => {
      if (error.name === "AbortError") return;
      report("warn", "lock-screen controls could not start", `${error.name}: ${error.message}`);
      letGo("the phone refused to play");
    });
  }

  function quiet() {
    if (!audio || audio.paused) return;
    ownPause = true;
    audio.pause();
  }

  function onPause() {
    if (ownPause) {
      ownPause = false;
      return;
    }
    if (engaged) letGo("something else on the phone took the sound");
  }

  const handlers = {
    play: () => {
      sound();
      command("/api/playback/resume");
    },
    pause: () => {
      quiet();
      command("/api/playback/pause");
    },
    seekbackward: (details) => command("/api/playback/seek", { by: -(details.seekOffset || JUMP) }),
    seekforward: (details) => command("/api/playback/seek", { by: details.seekOffset || JUMP }),
    seekto: (details) => command("/api/playback/seek", { to: details.seekTime }),
  };

  function handle(action, handler) {
    try {
      navigator.mediaSession.setActionHandler(action, handler);
    } catch {}
  }

  function describe(now) {
    if (now.loading) return;
    const look = `${now.title}|${now.line}|${now.art}`;
    if (look === described && navigator.mediaSession.metadata === metadata) return;
    described = look;
    const episode = Boolean(now.show);
    metadata = new MediaMetadata({
      title: episode ? now.line : now.title,
      artist: episode ? now.title : "SemmelTV",
      album: "SemmelTV",
      artwork: now.art ? [{ src: now.art, sizes: "342x513", type: "image/jpeg" }] : [],
    });
    navigator.mediaSession.metadata = metadata;
  }

  function place(now) {
    if (!navigator.mediaSession.setPositionState || !(now.duration > 0)) return;
    const at = Date.now();
    if (placed && placed.key === now.key && placed.paused === now.paused && placed.speed === now.speed && placed.duration === now.duration) {
      const expected = placed.position + (now.paused ? 0 : ((at - placed.at) / 1000) * now.speed);
      if (Math.abs(expected - now.position) < DRIFT) return;
    }
    const position = Math.max(0, Math.min(now.position, now.duration));
    placed = { key: now.key, paused: now.paused, speed: now.speed, duration: now.duration, position, at };
    try {
      navigator.mediaSession.setPositionState({ duration: now.duration, position, playbackRate: now.speed });
    } catch {}
  }

  async function findNext(now) {
    if (nextFor === now.key) return;
    nextFor = now.key;
    next = null;
    handle("nexttrack", null);
    if (!now.show) return;
    let show;
    try {
      const response = await fetch(`${api}/api/shows/${encodeURIComponent(now.show)}`);
      if (!response.ok) return;
      show = await response.json();
    } catch {
      return;
    }
    const episodes = (show.seasons || []).flatMap((season) => season.episodes || []);
    const index = episodes.findIndex((each) => each.season === now.season && each.episode === now.episode);
    const following = index >= 0 ? episodes[index + 1] : null;
    if (!following || nextFor !== now.key || !engaged) return;
    next = { show: now.show, season: following.season, episode: following.episode };
    handle("nexttrack", () => command("/api/play", next));
  }

  function sync() {
    if (!engaged) return;
    if (!eligible()) {
      letGo("the phone plays it now");
      return;
    }
    const now = tv();
    if (!now) {
      if (!idleTimer) idleTimer = setTimeout(() => letGo("the TV plays nothing"), IDLE_FOR);
      return;
    }
    clearTimeout(idleTimer);
    idleTimer = 0;
    describe(now);
    if (now.paused) quiet();
    else sound();
    place(now);
    findNext(now);
  }

  function engage() {
    if (!eligible()) return;
    if (!audio) {
      audio = new Audio(silence());
      audio.loop = true;
      audio.addEventListener("pause", onPause);
    }
    audio.play().catch((error) => {
      if (error.name !== "AbortError") report("warn", "lock-screen controls could not start", `${error.name}: ${error.message}`);
    });
    if (!engaged) {
      engaged = true;
      for (const [action, handler] of Object.entries(handlers)) handle(action, handler);
      report("info", "lock-screen controls on");
    }
    sync();
  }

  function letGo(why) {
    if (!engaged) return;
    engaged = false;
    clearTimeout(idleTimer);
    idleTimer = 0;
    quiet();
    if (navigator.mediaSession.metadata === metadata) {
      navigator.mediaSession.metadata = null;
      for (const action of [...Object.keys(handlers), "nexttrack"]) handle(action, null);
    }
    metadata = null;
    described = null;
    placed = null;
    nextFor = null;
    next = null;
    report("info", "lock-screen controls off", why);
  }

  document.addEventListener("click", (event) => {
    if (event.target.closest?.("[data-play-movie], [data-play-show]") || (event.target.closest?.(".player") && tv())) engage();
  }, true);

  window.addEventListener("pagehide", () => letGo("the page closed"));

  new MutationObserver(sync).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["class"] });
  new MutationObserver(sync).observe(html, { attributes: true, attributeFilter: ["class", "data-mode"] });
})();
