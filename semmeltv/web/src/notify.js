const api = document.documentElement.dataset.api || "";

function bytes(base64) {
  const text = atob(base64.replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(text, (c) => c.charCodeAt(0));
}

function same(buffer, key) {
  if (!buffer) return false;
  const have = new Uint8Array(buffer);
  return have.length === key.length && have.every((byte, index) => byte === key[index]);
}

function apple() {
  return /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
}

function standalone() {
  return navigator.standalone === true || matchMedia("(display-mode: standalone)").matches;
}

async function send(method, body) {
  const response = await fetch(`${api}/api/push/subscriptions`, {
    method,
    credentials: "include",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const answer = await response.json().catch(() => ({}));
  if (!response.ok || answer.ok === false) throw new Error(answer.message || answer.error || `The TV said ${response.status}`);
  return answer.message || "";
}

function setUp(section) {
  section.dataset.ready = "true";
  const detail = section.querySelector("[data-notify=detail]");
  const button = section.querySelector("[data-notify=toggle]");
  const said = section.querySelector("[data-notify=said]");
  const what = detail.textContent;
  let registration = null;
  let key = null;

  function show(state, text) {
    section.dataset.state = state;
    detail.textContent = text;
    button.hidden = !["on", "off"].includes(state);
    button.textContent = state === "on" ? "Turn off" : "Turn on";
    button.className = state === "on" ? "pill quiet" : "pill";
  }

  async function check() {
    if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) {
      if (apple() && !standalone()) {
        return show(
          "home-screen",
          "On iPhone, notifications come to SemmelTV on the Home Screen: in Safari tap Share, then Add to Home Screen, open SemmelTV from there and turn them on here.",
        );
      }
      return show("unsupported", "This browser can't show notifications from SemmelTV.");
    }
    if (Notification.permission === "denied") {
      return show("blocked", "Notifications for SemmelTV are blocked in this device's settings.");
    }
    try {
      const answer = await fetch(`${api}/api/push`, { credentials: "include" });
      if (!answer.ok) throw new Error(`The TV said ${answer.status}`);
      key = bytes((await answer.json()).key);
      registration = await navigator.serviceWorker.register("/sw.js");
      await navigator.serviceWorker.ready;
      let subscription = await registration.pushManager.getSubscription();
      if (subscription && !same(subscription.options && subscription.options.applicationServerKey, key)) {
        await subscription.unsubscribe();
        subscription = null;
      }
      if (subscription) {
        await send("POST", subscription.toJSON());
        return show("on", `On: ${what}`);
      }
      show("off", what);
    } catch (error) {
      show("failed", `Notifications can't be set up right now: ${error.message}`);
    }
  }

  button.addEventListener("click", async () => {
    said.textContent = "";
    button.disabled = true;
    try {
      if (section.dataset.state === "on") {
        const subscription = await registration.pushManager.getSubscription();
        if (subscription) {
          await send("DELETE", { endpoint: subscription.endpoint });
          await subscription.unsubscribe();
        }
        show("off", what);
        said.textContent = "Notifications are off on this device";
      } else {
        const subscription = await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key });
        said.textContent = await send("POST", subscription.toJSON());
        show("on", `On: ${what}`);
      }
    } catch (error) {
      if (window.Notification && Notification.permission === "denied") {
        show("blocked", "Notifications for SemmelTV are blocked in this device's settings.");
      } else {
        said.textContent = error.message || "That didn't work";
      }
      if (window.semmeltvReport) window.semmeltvReport("warn", "notifications", error.message || String(error));
    } finally {
      button.disabled = false;
    }
  });

  check();
}

function look() {
  const section = document.querySelector("[data-notify=section]:not([data-ready])");
  if (section) setUp(section);
}

look();
new MutationObserver(look).observe(document.documentElement, { subtree: true, childList: true });
