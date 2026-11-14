(() => {
  const FAILED_FOR = 1200;
  const UNCONFIRMED = 10000;
  const running = new Map();
  const failing = new Map();

  function toast(text) {
    window.dispatchEvent(new CustomEvent("semmeltv:toast", { detail: { text, tone: "error" } }));
  }

  function marked(mark) {
    if (!mark) return [];
    if (typeof mark === "string") return [...document.querySelectorAll(mark)];
    return mark.isConnected ? [mark] : [];
  }

  function paint() {
    const wanted = new Map();
    for (const [key, run] of running) for (const element of marked(run.mark)) wanted.set(element, ["pending", key]);
    for (const [key, fail] of failing) for (const element of marked(fail.mark)) if (!wanted.has(element)) wanted.set(element, ["failed", key]);
    for (const element of document.querySelectorAll("[data-act-key]")) {
      if (wanted.has(element)) continue;
      element.removeAttribute("data-act");
      element.removeAttribute("data-act-key");
      element.removeAttribute("aria-busy");
    }
    for (const [element, [state, key]] of wanted) {
      if (element.dataset.act !== state) element.dataset.act = state;
      if (element.dataset.actKey !== key) element.dataset.actKey = key;
      if (state === "pending") element.setAttribute("aria-busy", "true");
      else element.removeAttribute("aria-busy");
    }
  }

  function until(predicate, timeout) {
    return new Promise((resolve) => {
      if (predicate()) return resolve(true);
      const watcher = new MutationObserver(() => {
        if (!predicate()) return;
        stop();
        resolve(true);
      });
      const timer = setTimeout(() => {
        stop();
        resolve(predicate());
      }, timeout);
      function stop() {
        watcher.disconnect();
        clearTimeout(timer);
      }
      watcher.observe(document.documentElement, { subtree: true, childList: true, attributes: true, characterData: true });
    });
  }

  function act({ key, mark, optimistic, send, confirmed, revert, settled, timeout = UNCONFIRMED, failure = "The TV didn't answer" }) {
    const run = { mark };
    running.set(key, run);
    failing.delete(key);
    const current = () => running.get(key) === run;
    if (optimistic) optimistic();
    paint();
    const finish = (ok, message) => {
      if (!current()) return;
      running.delete(key);
      if (settled) settled(ok);
      if (!ok) {
        if (revert) revert();
        const fail = { mark };
        failing.set(key, fail);
        setTimeout(() => {
          if (failing.get(key) === fail) failing.delete(key);
          paint();
        }, FAILED_FOR);
        toast(message || failure);
        if (window.semmeltvReport) window.semmeltvReport("warn", `act ${key} failed`, message || failure);
      }
      paint();
    };
    (async () => {
      try {
        if (send) await send();
        if (!current()) return;
        const agreed = confirmed ? await until(() => !current() || confirmed(), timeout) : true;
        finish(agreed, agreed ? "" : failure);
      } catch (error) {
        finish(false, (error && error.message) || failure);
      }
    })();
    return { cancel: () => current() && (running.delete(key), settled && settled(true), paint()) };
  }

  act.pending = (key) => running.has(key);
  window.semmeltvAct = act;
  new MutationObserver((records) => {
    if (records.some((record) => record.type === "childList" && record.addedNodes.length)) paint();
  }).observe(document.documentElement, { subtree: true, childList: true });
})();
