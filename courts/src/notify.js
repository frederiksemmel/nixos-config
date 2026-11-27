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

async function post(path, body) {
  const response = await fetch(path, {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const answer = await response.json().catch(() => ({}));
  if (!response.ok || answer.ok === false) throw new Error(answer.message || `The server said ${response.status}`);
}

async function setUp(section) {
  const said = section.querySelector("[data-notify=said]");
  const button = section.querySelector("[data-notify=toggle]");
  let registration = null;
  let key = null;

  function show(state, text) {
    said.textContent = text;
    button.hidden = !["on", "off"].includes(state);
    button.textContent = state === "on" ? "Turn off" : "Turn on";
    button.className = state === "on" ? "pill quiet" : "pill";
    section.dataset.state = state;
  }

  async function check() {
    if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) {
      if (apple() && !standalone()) {
        return show("home-screen", "On iPhone, notifications need Courts on the Home Screen: tap Share, then Add to Home Screen, and turn them on from there.");
      }
      return show("unsupported", "This browser can't show notifications.");
    }
    if (Notification.permission === "denied") return show("blocked", "Notifications are blocked in this device's settings.");
    const answer = await (await fetch("/api/push", { credentials: "include" })).json();
    if (!answer.ok) throw new Error(answer.message);
    key = bytes(answer.key);
    registration = await navigator.serviceWorker.register("/sw.js");
    await navigator.serviceWorker.ready;
    let subscription = await registration.pushManager.getSubscription();
    if (subscription && !same(subscription.options && subscription.options.applicationServerKey, key)) {
      await subscription.unsubscribe();
      subscription = null;
    }
    if (subscription) {
      await post("/api/push/subscribe", subscription.toJSON());
      return show("on", "Notifications on this device are on.");
    }
    show("off", "Notifications on this device are off.");
  }

  button.addEventListener("click", async () => {
    button.disabled = true;
    try {
      const subscription = await registration.pushManager.getSubscription();
      if (subscription) {
        await post("/api/push/unsubscribe", { endpoint: subscription.endpoint });
        await subscription.unsubscribe();
        show("off", "Notifications on this device are off.");
      } else {
        if ((await Notification.requestPermission()) !== "granted") return show("blocked", "Notifications were not allowed.");
        const made = await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key });
        await post("/api/push/subscribe", made.toJSON());
        show("on", "Notifications on this device are on.");
      }
    } catch (error) {
      said.textContent = `That didn't work: ${error.message}`;
    } finally {
      button.disabled = false;
    }
  });

  try {
    await check();
  } catch (error) {
    show("error", `Notifications: ${error.message}`);
  }
}

for (const section of document.querySelectorAll("[data-notify=section]")) setUp(section);
