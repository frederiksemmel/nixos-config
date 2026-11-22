// While SemmelTV is closed (planned work, a restart, an outage), the front
// answers with a 503 marked X-SemmelTV-Maintenance and keeps the status at
// /api/maintenance. This lays the front's closed screen over the page, as
// an iframe of /api/maintenance/screen, which keeps its own countdown and
// tells this page once SemmelTV is open again. `semmeltv:maintenance` tells
// the rest of the page, the player included, with `{ closed }`.
(() => {
  const html = document.documentElement;
  const fetchNow = window.fetch.bind(window);
  const EVERY_MS = 10000;
  let frame = null;

  function tell(closed) {
    window.dispatchEvent(new CustomEvent("semmeltv:maintenance", { detail: { closed } }));
  }

  function close() {
    if (frame) return;
    frame = document.createElement("iframe");
    frame.className = "maintenance";
    frame.title = "SemmelTV";
    frame.src = "/api/maintenance/screen";
    html.dataset.maintenance = "";
    (document.body || html).append(frame);
    tell(true);
  }

  function open() {
    if (!frame) return;
    frame.remove();
    frame = null;
    delete html.dataset.maintenance;
    tell(false);
  }

  window.addEventListener("message", (event) => {
    if (frame && event.origin === location.origin && event.source === frame.contentWindow && event.data?.semmeltv === "open") open();
  });

  window.fetch = async (input, init) => {
    const response = await fetchNow(input, init);
    if (response.status === 503 && response.headers.has("X-SemmelTV-Maintenance")) close();
    return response;
  };

  async function check() {
    if (frame || document.visibilityState !== "visible") return;
    try {
      const response = await fetchNow("/api/maintenance", { cache: "no-store" });
      if (response.ok && (await response.json()).state === "closed") close();
    } catch {}
  }

  setInterval(check, EVERY_MS);
  document.addEventListener("visibilitychange", check);
})();
