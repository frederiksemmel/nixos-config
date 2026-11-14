(() => {
  const api = document.documentElement.dataset.api || "";
  const LOG = `${api}/api/log`;
  const FLUSH_AFTER = 1000;
  const MOST_QUEUED = 50;
  const started = performance.now();
  const queue = [];
  let timer = 0;

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
    const version = navigator.userAgent.match(/OS (\d+[_.]\d+)/) || navigator.userAgent.match(/Version\/([\d.]+)/);
    const standalone = navigator.standalone || matchMedia("(display-mode: standalone)").matches;
    return [version ? `iOS/Safari ${version[1].replace("_", ".")}` : navigator.userAgent.slice(0, 120), standalone ? "home-screen app" : "tab"].join(", ");
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
    if (queue.length >= MOST_QUEUED) return;
    queue.push({ level, message: text(message), detail: detail === undefined ? undefined : text(detail), page: location.pathname, at: performance.now() - started });
    if (!timer) timer = setTimeout(flush, FLUSH_AFTER);
  }

  const send = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const url = typeof input === "string" ? input : input.url;
    const method = (init && init.method) || "GET";
    if (url.endsWith("/api/log")) return send(input, init);
    try {
      const response = await send(input, init);
      if (!response.ok && /\/api\//.test(url)) {
        const answer = await response.clone().text().catch(() => "");
        report("warn", `${method} ${url.replace(location.origin, "")} answered ${response.status}`, answer.slice(0, 300));
      }
      return response;
    } catch (error) {
      if (!(error && error.name === "AbortError")) report("error", `${method} ${url.replace(location.origin, "")} failed`, error);
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
  });
  window.addEventListener("pagehide", () => flush(true));
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") flush(true);
  });

  window.semmeltvReport = report;
  report("info", "page opened", `${innerWidth}x${innerHeight} @${devicePixelRatio}x`);
})();
