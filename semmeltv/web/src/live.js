// Watches the page's live connection, which renders the bar, the remote and
// the switch. The web app renders a pulse over it every 5 s. When the pulse
// stops, or the page comes back from being away, the connection restarts and
// everything it renders comes fresh from the TV; until then the page is marked
// stale.
(() => {
  const STALE = 12000;
  const AWAY = 4000;
  const CHECK_EVERY = 2000;
  const html = document.documentElement;
  const Native = window.WebSocket;
  const sockets = new Set();

  const runtime = (protocols) => protocols === "topcoat-runtime" || (Array.isArray(protocols) && protocols.includes("topcoat-runtime"));

  window.WebSocket = class extends Native {
    constructor(url, protocols) {
      super(url, protocols);
      if (!runtime(protocols)) return;
      sockets.add(this);
      this.addEventListener("close", () => sockets.delete(this));
    }
  };

  let heard = Date.now();
  let pulse = null;
  let hiddenAt = document.visibilityState === "hidden" ? Date.now() : null;

  function stale(on) {
    if (on) html.dataset.live = "stale";
    else delete html.dataset.live;
  }

  function listen() {
    const mark = document.getElementById("pulse");
    const at = mark && mark.dataset.at;
    if (!at || at === pulse) return;
    pulse = at;
    heard = Date.now();
    stale(false);
  }

  function reconnect(why) {
    if (window.semmeltvReport) window.semmeltvReport("info", "live connection restarts", why);
    stale(true);
    heard = Date.now();
    for (const socket of [...sockets]) {
      sockets.delete(socket);
      socket.dispatchEvent(new CloseEvent("close", { code: 4000, reason: why }));
      try { socket.close(4000, "stale"); } catch {}
    }
    window.dispatchEvent(new CustomEvent("semmeltv:reconnect", { detail: why }));
  }

  function check() {
    if (document.visibilityState !== "visible") return;
    const quiet = Date.now() - heard;
    if (quiet > STALE) reconnect(`no pulse for ${Math.round(quiet / 1000)} s`);
  }

  new MutationObserver(listen).observe(html, { childList: true, subtree: true, attributes: true, attributeFilter: ["data-at"] });
  setInterval(check, CHECK_EVERY);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") {
      hiddenAt = Date.now();
      return;
    }
    const away = hiddenAt === null ? 0 : Date.now() - hiddenAt;
    hiddenAt = null;
    if (away > AWAY) reconnect(`back after ${Math.round(away / 1000)} s away`);
    else check();
  });
  window.addEventListener("pageshow", (event) => {
    if (event.persisted) reconnect("back from the back-forward cache");
  });
  window.addEventListener("online", () => reconnect("the network is back"));
})();
