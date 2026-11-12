const SHOWN_MS = { info: 4000, ok: 4000, error: 7000, pending: 30000 };
const MOST = 3;
const FLING = 0.45;
const DISTANCE = 70;
const SLOP = 6;
const SEEN_KEY = "semmeltv-seen-notices";

const root = document.documentElement;
const stack = document.createElement("div");
stack.className = "toasts";
stack.setAttribute("role", "status");
stack.setAttribute("aria-live", "polite");
const byId = new Map();
let seen = new Set();

try {
  seen = new Set(JSON.parse(sessionStorage.getItem(SEEN_KEY) || "[]"));
} catch {}

function remember(id) {
  seen.add(id);
  try {
    sessionStorage.setItem(SEEN_KEY, JSON.stringify([...seen].slice(-50)));
  } catch {}
}

const still = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

function attach() {
  if (!stack.isConnected && document.body) document.body.append(stack);
}

function remove(toast, direction) {
  if (toast.dataset.leaving) return;
  toast.dataset.leaving = "true";
  clearTimeout(toast.timer);
  if (toast.dataset.id) byId.delete(toast.dataset.id);
  const [x, y] = direction === "left" ? [-1, 0] : direction === "right" ? [1, 0] : [0, -1];
  toast.style.transition = still() ? "opacity 0.15s" : "transform 0.22s ease-in, opacity 0.22s ease-in";
  toast.style.transform = still() ? "" : `translate(${x * 110}%, ${y * 130}%)`;
  toast.style.opacity = "0";
  setTimeout(() => toast.remove(), 240);
}

function arm(toast) {
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => remove(toast), SHOWN_MS[toast.dataset.tone] || SHOWN_MS.info);
}

function fill(toast, text, detail, tone) {
  toast.dataset.tone = tone;
  const lines = [text, detail].filter(Boolean).map((line, index) => {
    const span = document.createElement("span");
    span.className = index === 0 ? "toast-text" : "toast-detail";
    span.textContent = line;
    return span;
  });
  toast.replaceChildren(...lines);
}

export function toast({ text, detail = "", tone = "info", id = "", dismiss = false }) {
  let element = id ? byId.get(id) : null;
  if (dismiss) return element && remove(element);
  if (!text) return;
  attach();
  if (element) {
    fill(element, text, detail, tone);
    arm(element);
    return;
  }
  element = document.createElement("div");
  element.className = "toast";
  if (id) {
    element.dataset.id = id;
    byId.set(id, element);
  }
  fill(element, text, detail, tone);
  stack.prepend(element);
  const live = [...stack.children].filter((each) => !each.dataset.leaving);
  live.slice(MOST).forEach((old) => remove(old));
  arm(element);
}

window.addEventListener("semmeltv:toast", (event) => toast(event.detail || {}));

let drag = null;

stack.addEventListener("pointerdown", (event) => {
  const element = event.target.closest(".toast");
  if (!element || element.dataset.leaving) return;
  clearTimeout(element.timer);
  element.setPointerCapture(event.pointerId);
  element.style.transition = "none";
  drag = { element, pointer: event.pointerId, x: event.clientX, y: event.clientY, dx: 0, dy: 0, moved: false, samples: [{ x: event.clientX, y: event.clientY, t: performance.now() }] };
});

stack.addEventListener("pointermove", (event) => {
  if (!drag || drag.pointer !== event.pointerId) return;
  drag.dx = event.clientX - drag.x;
  drag.dy = event.clientY - drag.y;
  if (!drag.moved && Math.hypot(drag.dx, drag.dy) < SLOP) return;
  drag.moved = true;
  const now = performance.now();
  drag.samples.push({ x: event.clientX, y: event.clientY, t: now });
  while (drag.samples.length > 2 && now - drag.samples[0].t > 100) drag.samples.shift();
  const sideways = Math.abs(drag.dx) > Math.abs(drag.dy);
  const dx = sideways ? drag.dx : 0;
  const dy = sideways ? 0 : drag.dy < 0 ? drag.dy : drag.dy / 4;
  drag.element.style.transform = `translate(${dx}px, ${dy}px)`;
  drag.element.style.opacity = String(Math.max(0.2, 1 - Math.hypot(dx, Math.min(dy, 0)) / 240));
});

function release(event) {
  if (!drag || drag.pointer !== event.pointerId) return;
  const { element, dx, dy, moved, samples } = drag;
  drag = null;
  if (!moved) return remove(element);
  const first = samples[0];
  const last = samples[samples.length - 1];
  const time = Math.max(last.t - first.t, 1);
  const vx = (last.x - first.x) / time;
  const vy = (last.y - first.y) / time;
  const sideways = Math.abs(dx) > Math.abs(dy);
  if (sideways && (Math.abs(dx) > DISTANCE || Math.abs(vx) > FLING)) return remove(element, dx < 0 ? "left" : "right");
  if (!sideways && (dy < -DISTANCE / 2 || vy < -FLING)) return remove(element, "up");
  element.style.transition = still() ? "none" : "transform 0.25s cubic-bezier(0.2, 0.9, 0.3, 1.2), opacity 0.2s";
  element.style.transform = "";
  element.style.opacity = "";
  arm(element);
}

stack.addEventListener("pointerup", release);
stack.addEventListener("pointercancel", release);

function notices() {
  for (const notice of document.querySelectorAll("[data-notice]")) {
    const id = notice.dataset.notice;
    if (seen.has(id)) continue;
    remember(id);
    toast({ text: notice.dataset.text, detail: notice.dataset.detail, tone: notice.dataset.tone });
  }
}

function said(element) {
  const text = element.textContent.trim();
  if (!text || element.dataset.toasted === text) return;
  element.dataset.toasted = text;
  toast({ text, tone: element.classList.contains("refused") ? "error" : "info" });
}

new MutationObserver((records) => {
  attach();
  notices();
  for (const record of records) {
    const target = record.target.nodeType === 1 ? record.target : record.target.parentElement;
    const element = target?.closest?.(".said, .tile-said, .controls-said, .setting-said");
    if (element) said(element);
  }
}).observe(root, { childList: true, subtree: true, characterData: true });

attach();
notices();
