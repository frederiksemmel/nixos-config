(() => {
  const api = document.documentElement.dataset.api || "";
  const LOG = `${api}/api/log`;
  const FLUSH_AFTER = 1000;
  const MOST_QUEUED = 50;
  const started = performance.now();
  const queue = [];
  let timer = 0;
  const TRAIL = "semmeltv.trail";
  const MOST_TRAIL = 300;
  const opened = Date.now();
  let trail = [];
  try {
    trail = JSON.parse(sessionStorage.getItem(TRAIL) || "[]").slice(-MOST_TRAIL);
  } catch {}

  function deviceId() {
    try {
      return (localStorage.getItem("semmeltv.device") || "new").slice(0, 8);
    } catch {
      return "private";
    }
  }

  function deviceName() {
    const agent = navigator.userAgent;
    if (/iPhone/.test(agent)) return "iPhone";
    if (/iPad/.test(agent) || (/Macintosh/.test(agent) && navigator.maxTouchPoints > 1)) return "iPad";
    if (/Android/.test(agent)) return "Android";
    if (/Macintosh/.test(agent)) return "Mac";
    if (/Windows/.test(agent)) return "Windows";
    return "browser";
  }

  function agent() {
    const ua = navigator.userAgent;
    const standalone = navigator.standalone || matchMedia("(display-mode: standalone)").matches;
    const ios = /iPhone|iPad|iPod/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1);
    if (!ios) return [ua.slice(0, 120), standalone ? "home-screen app" : "tab"].join(", ");
    const safari = ua.match(/Version\/([\d.]+)/);
    const frozen = ua.match(/OS (\d+)[_.](\d+)/);
    const version = safari ? `iOS ${safari[1]}` : `iOS unknown (the user agent says ${frozen ? `${frozen[1]}.${frozen[2]}` : "nothing"}, frozen)`;
    return `${version}, ${standalone ? "home-screen app" : "Safari tab"}`;
  }

  function text(value) {
    if (value instanceof Error) return `${value.name}: ${value.message}${value.stack ? ` ${value.stack.split("\n").slice(0, 3).join(" < ")}` : ""}`;
    if (typeof value === "string") return value;
    try {
      return JSON.stringify(value);
    } catch {
      return String(value);
    }
  }

  function cut(value, most) {
    return value.length > most ? `${value.slice(0, most)}…` : value;
  }

  function note(kind, message, detail) {
    const entry = { t: Date.now(), kind, message: cut(text(message), 300), page: location.pathname };
    if (detail !== undefined && detail !== null && detail !== "") entry.detail = cut(text(detail), 600);
    trail.push(entry);
    if (trail.length > MOST_TRAIL) trail.splice(0, trail.length - MOST_TRAIL);
  }

  function keepTrail() {
    try {
      sessionStorage.setItem(TRAIL, JSON.stringify(trail));
    } catch {}
  }

  function snapshot() {
    const connection = navigator.connection;
    const html = document.documentElement;
    return {
      device: `${deviceName()} ${deviceId()}`,
      agent: agent(),
      user_agent: navigator.userAgent,
      url: location.pathname + location.search,
      viewport: `${innerWidth}x${innerHeight} @${devicePixelRatio}x`,
      scroll_y: Math.round(scrollY),
      visibility: document.visibilityState,
      online: navigator.onLine,
      connection: connection ? `${connection.effectiveType || "?"}, ${connection.downlink ?? "?"} Mbit/s, rtt ${connection.rtt ?? "?"} ms` : undefined,
      live: html.dataset.live || "fresh",
      device_mode: html.dataset.device,
      page_age_s: Math.round((Date.now() - opened) / 1000),
      language: navigator.language,
      time_zone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      at: new Date().toISOString(),
      trail: trail.slice(),
    };
  }

  function flush(beacon = false) {
    clearTimeout(timer);
    timer = 0;
    if (!queue.length) return;
    const body = JSON.stringify({ device: `${deviceName()} ${deviceId()}`, agent: agent(), entries: queue.splice(0) });
    if (beacon && navigator.sendBeacon) {
      navigator.sendBeacon(LOG, new Blob([body], { type: "application/json" }));
      return;
    }
    send(LOG, { method: "POST", headers: { "content-type": "application/json" }, body, keepalive: true }).catch(() => {});
  }

  function report(level, message, detail) {
    note(level, message, detail);
    if (queue.length >= MOST_QUEUED) return;
    queue.push({ level, message: text(message), detail: detail === undefined ? undefined : text(detail), page: location.pathname, at: performance.now() - started });
    if (!timer) timer = setTimeout(flush, FLUSH_AFTER);
  }

  const send = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const url = typeof input === "string" ? input : input.url;
    const method = (init && init.method) || "GET";
    if (url.endsWith("/api/log")) return send(input, init);
    const path = url.replace(location.origin, "");
    const began = performance.now();
    try {
      const response = await send(input, init);
      if (!url.endsWith("/api/feedback")) note("fetch", `${method} ${path} ${response.status}`, `${Math.round(performance.now() - began)} ms`);
      if (!response.ok && /\/api\//.test(url)) {
        const answer = await response.clone().text().catch(() => "");
        report("warn", `${method} ${url.replace(location.origin, "")} answered ${response.status}`, answer.slice(0, 300));
      }
      return response;
    } catch (error) {
      if (error && error.name === "AbortError") note("fetch", `${method} ${path} aborted`, `${Math.round(performance.now() - began)} ms`);
      else report("error", `${method} ${path} failed`, error);
      throw error;
    }
  };

  for (const level of ["error", "warn"]) {
    const original = console[level].bind(console);
    console[level] = (...values) => {
      report(level, values.map(text).join(" "));
      original(...values);
    };
  }

  window.addEventListener("error", (event) => {
    if (event.error || event.message) report("error", event.message || "script error", event.error || `${event.filename}:${event.lineno}`);
  });
  window.addEventListener("unhandledrejection", (event) => report("error", "unhandled rejection", event.reason));
  window.addEventListener("semmeltv:toast", (event) => {
    const detail = event.detail || {};
    if (detail.tone === "error" && detail.text) report("error", `toast: ${detail.text}`);
    else if (detail.text) note("toast", detail.text, detail.tone);
  });

  for (const name of ["pushState", "replaceState"]) {
    const real = history[name].bind(history);
    history[name] = (...args) => {
      const before = location.pathname + location.search;
      const result = real(...args);
      const after = location.pathname + location.search;
      if (after !== before) note("route", `${name === "pushState" ? "to" : "replaced with"} ${after}`);
      return result;
    };
  }
  window.addEventListener("popstate", () => note("route", `back or forward to ${location.pathname + location.search}`));

  function label(element) {
    const named = element.getAttribute("aria-label") || element.getAttribute("title") || element.innerText || element.textContent || "";
    const said = named.replace(/\s+/g, " ").trim().slice(0, 60);
    const href = element.getAttribute("href");
    return [element.tagName.toLowerCase(), said && `"${said}"`, href && href.startsWith("/") && href].filter(Boolean).join(" ");
  }

  window.addEventListener("click", (event) => {
    const target = event.target.closest?.("button, a, [role=button], input, select, summary");
    if (target) note("tap", label(target));
  }, { capture: true, passive: true });

  for (const type of ["play", "pause", "waiting", "stalled", "ended", "error", "loadedmetadata", "emptied", "seeking"]) {
    document.addEventListener(type, (event) => {
      const media = event.target;
      if (!(media instanceof HTMLMediaElement)) return;
      const where = Number.isFinite(media.currentTime) ? `at ${media.currentTime.toFixed(1)} s` : "";
      const problem = type === "error" && media.error ? `code ${media.error.code} ${media.error.message || ""}` : "";
      note("media", `${media.tagName.toLowerCase()} ${type}`, [where, problem, `ready ${media.readyState}, network ${media.networkState}`].filter(Boolean).join(", "));
    }, true);
  }

  window.addEventListener("online", () => note("network", "online"));
  window.addEventListener("offline", () => note("network", "offline"));

  const NativeSocket = window.WebSocket;
  window.WebSocket = class extends NativeSocket {
    constructor(url, protocols) {
      super(url, protocols);
      const where = String(url).replace(/^wss?:\/\/[^/]+/, "");
      const began = Date.now();
      note("socket", `connecting ${where}`);
      this.addEventListener("open", () => note("socket", `open ${where}`, `after ${Date.now() - began} ms`));
      this.addEventListener("close", (event) => note("socket", `closed ${where}`, `code ${event.code}${event.reason ? ` ${event.reason}` : ""}, after ${Math.round((Date.now() - began) / 1000)} s`));
    }
  };
  const LEFT = "semmeltv.left";
  let touch = null;
  window.addEventListener("touchstart", (event) => {
    const point = event.touches[0];
    touch = { x: Math.round(point.clientX), y: Math.round(point.clientY), dy: 0, at: Date.now() };
  }, { passive: true, capture: true });
  window.addEventListener("touchmove", (event) => {
    if (touch) touch.dy = Math.round(event.touches[0].clientY - touch.y);
  }, { passive: true, capture: true });

  function lastTouch() {
    if (!touch) return "no touch";
    return `last touch ${((Date.now() - touch.at) / 1000).toFixed(1)} s before, from ${touch.x},${touch.y} moving ${touch.dy} px down`;
  }

  window.addEventListener("pagehide", (event) => {
    try {
      sessionStorage.setItem(LEFT, JSON.stringify({ at: Date.now(), persisted: event.persisted, path: location.pathname, touch: lastTouch(), scroll: Math.round(scrollY) }));
    } catch {}
    report("info", "page hides", `${event.persisted ? "kept in the back-forward cache" : "unloading"}, ${lastTouch()}, scrollY ${Math.round(scrollY)}`);
    flush(true);
    keepTrail();
  });
  window.addEventListener("pageshow", (event) => {
    if (event.persisted) report("info", "page shows again from the back-forward cache");
  });
  document.addEventListener("visibilitychange", () => {
    report("info", `page ${document.visibilityState}`);
    if (document.visibilityState === "hidden") {
      flush(true);
      keepTrail();
    }
  });

  function arrival() {
    const navigation = performance.getEntriesByType && performance.getEntriesByType("navigation")[0];
    const kind = navigation ? navigation.type : "unknown";
    let left = null;
    try {
      left = JSON.parse(sessionStorage.getItem(LEFT) || "null");
      sessionStorage.removeItem(LEFT);
    } catch {}
    if (left) return `${kind}; the page before (${left.path}) hid ${((Date.now() - left.at) / 1000).toFixed(1)} s ago, ${left.touch}`;
    return kind === "reload" ? "reload, and the page before never fired pagehide, so it was killed" : kind;
  }

  window.semmeltvReport = report;
  window.semmeltvTrail = { note, snapshot };
  report("info", "page opened", `${innerWidth}x${innerHeight} @${devicePixelRatio}x, ${arrival()}`);
})();
