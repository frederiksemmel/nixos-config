const api = document.documentElement.dataset.api || "";
const root = document.documentElement;
const UNCONFIRMED = 10000;

let returnFocus = null;
let shownKey = null;
let drag = null;
let dragged = false;
let closing = false;
let pendingLink = null;
let passLink = false;

const sheet = () => document.getElementById("sheet");
const isOpen = () => root.hasAttribute("data-sheet");

function setOpen(open) {
  const element = sheet();
  if (!element || open === isOpen()) return;
  root.toggleAttribute("data-sheet", open);
  element.inert = !open;
  if (open) {
    returnFocus = document.activeElement;
    element.querySelector(".sheet-scroll").scrollTop = 0;
    element.querySelector(".sheet-close").focus({ preventScroll: true });
    refreshTracks();
    markChoices();
  } else if (returnFocus && document.contains(returnFocus)) {
    returnFocus.focus({ preventScroll: true });
  }
}

function openSheet() {
  if (isOpen()) return;
  history.pushState({ ...(history.state || {}), sheet: true }, "");
  setOpen(true);
}

function closeSheet() {
  if (!isOpen() || closing) return;
  if (history.state && history.state.sheet) {
    closing = true;
    history.back();
  } else {
    setOpen(false);
  }
}

window.addEventListener("popstate", () => {
  closing = false;
  if (pendingLink) {
    const link = pendingLink;
    pendingLink = null;
    passLink = true;
    link.click();
    passLink = false;
  }
  setOpen(Boolean(history.state && history.state.sheet));
});

function answerFor(button) {
  for (let element = button.parentElement; element; element = element.parentElement) {
    const said = element.querySelector("[data-said]");
    if (said) return said;
  }
  return null;
}

function say(said, text, ok) {
  if (!said) return;
  said.textContent = text;
  said.classList.toggle("refused", !ok);
}

async function post(button) {
  const said = answerFor(button);
  if (button.dataset.pending) say(said, button.dataset.pending, true);
  button.setAttribute("aria-busy", "true");
  let ok = false;
  try {
    const response = await fetch(`${api}${button.dataset.post}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: button.dataset.body || "{}",
    });
    const answer = await response.json().catch(() => ({}));
    ok = response.ok && answer.ok !== false;
    say(said, answer.message || (response.ok ? "" : `The TV answered ${response.status}`), ok);
  } catch {
    say(said, "The TV is not answering", false);
  } finally {
    button.removeAttribute("aria-busy");
  }
  if (!ok) button.classList.remove("pending");
}

function choose(button) {
  const list = button.closest(".choices");
  if (!list) return;
  for (const other of list.querySelectorAll(".choice")) other.classList.toggle("pending", other === button);
  setTimeout(() => button.classList.remove("pending"), UNCONFIRMED);
  markChoices();
}

document.addEventListener("click", (event) => {
  if (dragged) {
    dragged = false;
    return;
  }
  const target = event.target.closest && event.target.closest("[data-sheet], [data-post]");
  if (!target) return;
  if (target.dataset.sheet === "open") openSheet();
  else if (target.dataset.sheet === "close") closeSheet();
  else if (target.dataset.post && target.getAttribute("aria-busy") !== "true") {
    if (target.classList.contains("choice")) choose(target);
    post(target);
  }
});

window.addEventListener("click", (event) => {
  const link = event.target.closest && event.target.closest("#sheet a[href]");
  if (!link || passLink || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  if (history.state && history.state.sheet && !closing) {
    event.preventDefault();
    pendingLink = link;
    closing = true;
    history.back();
  } else {
    setOpen(false);
  }
}, true);

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && isOpen()) closeSheet();
});

function trackName(track) {
  let name = track.label;
  if (track.hearing_impaired) name += " SDH";
  if (track.forced) name += " (forced)";
  return name;
}

function trackNames(tracks) {
  const names = tracks.map(trackName);
  return tracks.map((track, index) => {
    const name = names[index];
    const same = names.filter((other) => other === name).length;
    if (same === 1) return name;
    const title = track.title && track.title !== track.label ? track.title : null;
    if (title && tracks.filter((other) => trackName(other) === name && other.title === track.title).length === 1) {
      return `${name} · ${title}`;
    }
    return `${name} ${names.slice(0, index + 1).filter((other) => other === name).length}`;
  });
}

function choice(label, track, path) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "choice";
  button.textContent = label;
  button.dataset.value = track === null ? "off" : String(track);
  button.dataset.post = path;
  button.dataset.body = JSON.stringify({ track });
  return button;
}

function fill(kind, buttons) {
  const group = document.querySelector(`[data-tracks="${kind}"]`);
  if (!group) return;
  group.hidden = buttons.length === 0;
  group.querySelector(".choices").replaceChildren(...buttons);
}

async function refreshTracks() {
  let playing = null;
  try {
    const response = await fetch(`${api}/api/subtitles`);
    if (response.ok) playing = (await response.json()).playing;
  } catch {
    return;
  }
  const subtitleNames = playing ? trackNames(playing.tracks) : [];
  const subtitles = playing && playing.tracks.length > 0
    ? [
        choice("Off", null, "/api/subtitles/track"),
        ...playing.tracks.map((track, index) => choice(subtitleNames[index], track.id, "/api/subtitles/track")),
      ]
    : [];
  const audioTracks = (playing && playing.audio_tracks) || [];
  const audioNames = trackNames(audioTracks);
  const audio = audioTracks.length > 1
    ? audioTracks.map((track, index) => choice(audioNames[index], track.id, "/api/audio/track"))
    : [];
  fill("subtitles", subtitles);
  fill("audio", audio);
  markChoices();
}

function markChoices() {
  const state = document.querySelector(".now-state");
  for (const list of document.querySelectorAll(".choices[data-state]")) {
    const value = state ? state.dataset[list.dataset.state] : null;
    for (const button of list.querySelectorAll(".choice")) {
      const chosen = button.dataset.value === value;
      button.setAttribute("aria-pressed", String(chosen));
      if (chosen) button.classList.remove("pending");
    }
  }
}

new MutationObserver(() => {
  const state = document.querySelector(".now-state");
  const key = state ? state.dataset.key : null;
  if (key !== shownKey) {
    shownKey = key;
    if (isOpen()) refreshTracks();
  }
  markChoices();
}).observe(document.body, { childList: true, subtree: true });

const SETTLE_MS = 650;
const FLICK = 0.5;
const START_DRAG = 8;
let settleTimer = 0;

const dim = () => document.querySelector(".sheet-dim");

function sheetHeight() {
  return sheet().getBoundingClientRect().height - 60;
}

function place(offset, height) {
  const y = Math.min(Math.max(offset, 0), height + 40);
  sheet().style.transform = `translateY(${y}px)`;
  dim().style.opacity = String(Math.max(0, 1 - y / height));
}

function beginDrag(kind, y, pointer) {
  clearTimeout(settleTimer);
  const height = sheetHeight();
  drag = { kind, pointer, start: y, base: kind === "open" ? height + 40 : 0, height, moved: 0, samples: [{ y, t: performance.now() }], live: false };
}

function moveDrag(y) {
  drag.moved = y - drag.start;
  const now = performance.now();
  drag.samples.push({ y, t: now });
  while (drag.samples.length > 2 && now - drag.samples[0].t > 100) drag.samples.shift();
  if (!drag.live) {
    if (Math.abs(drag.moved) < START_DRAG) return;
    if (drag.kind === "open" && drag.moved > 0) return;
    drag.live = true;
    root.toggleAttribute("data-sheet-drag", true);
    sheet().classList.add("dragging");
    dim().classList.add("dragging");
  }
  place(drag.base + drag.moved, drag.height);
}

function velocity() {
  const first = drag.samples[0];
  const last = drag.samples[drag.samples.length - 1];
  const time = last.t - first.t;
  return time > 0 ? (last.y - first.y) / time : 0;
}

function settle(open) {
  const element = sheet();
  element.classList.remove("dragging");
  dim().classList.remove("dragging");
  element.style.transform = open ? "translateY(0px)" : "translateY(calc(100% + 40px))";
  dim().style.opacity = open ? "1" : "0";
  if (open) openSheet();
  else closeSheet();
  settleTimer = setTimeout(() => {
    element.style.transform = "";
    dim().style.opacity = "";
    root.removeAttribute("data-sheet-drag");
  }, SETTLE_MS);
}

function endDrag() {
  if (!drag) return;
  const finished = drag;
  const speed = velocity();
  drag = null;
  if (!finished.live) return;
  dragged = true;
  setTimeout(() => {
    dragged = false;
  }, 0);
  const travelled = Math.abs(finished.moved) / finished.height;
  if (finished.kind === "open") settle(speed < -FLICK || (travelled > 0.25 && speed <= FLICK));
  else settle(!(speed > FLICK || (travelled > 0.25 && speed >= -FLICK)));
}

function startsDrag(target) {
  if (target.closest(".bar-open")) return isOpen() ? null : "open";
  if (target.closest(".sheet-top")) return isOpen() ? "close" : null;
  return null;
}

document.addEventListener("pointerdown", (event) => {
  if (drag || (event.pointerType === "mouse" && event.button !== 0) || !event.target.closest) return;
  const kind = startsDrag(event.target);
  if (!kind) return;
  beginDrag(kind, event.clientY, event.pointerId);
});

document.addEventListener("pointermove", (event) => {
  if (drag && drag.pointer === event.pointerId) moveDrag(event.clientY);
});

for (const name of ["pointerup", "pointercancel"]) {
  document.addEventListener(name, (event) => {
    if (drag && drag.pointer === event.pointerId) endDrag();
  });
}

let pull = null;

document.addEventListener("touchstart", (event) => {
  const scroller = event.target.closest && event.target.closest(".sheet-scroll");
  if (!scroller || drag || !isOpen() || event.touches.length !== 1 || event.target.closest(".sheet-top, .scrub, input, [data-no-drag]")) {
    pull = null;
    return;
  }
  pull = { scroller, start: event.touches[0].clientY, id: event.touches[0].identifier };
}, { passive: true });

document.addEventListener("touchmove", (event) => {
  if (!pull) return;
  const touch = [...event.changedTouches].find((each) => each.identifier === pull.id);
  if (!touch) return;
  if (!drag) {
    const down = touch.clientY - pull.start;
    if (pull.scroller.scrollTop > 0 || down < 0) {
      pull = null;
      return;
    }
    event.preventDefault();
    if (down < START_DRAG) return;
    beginDrag("close", pull.start, "touch");
  }
  if (drag && drag.pointer === "touch") {
    event.preventDefault();
    moveDrag(touch.clientY);
  }
}, { passive: false });

for (const name of ["touchend", "touchcancel"]) {
  document.addEventListener(name, () => {
    pull = null;
    if (drag && drag.pointer === "touch") endDrag();
  });
}

if (history.state && history.state.sheet) setOpen(true);
markChoices();
