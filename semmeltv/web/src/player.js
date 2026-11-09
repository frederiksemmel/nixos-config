const api = document.documentElement.dataset.api || "";
const root = document.documentElement;
const CLOSE_DRAG = 90;
const SETTLE = 800;

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
    markSpeed();
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
  try {
    const response = await fetch(`${api}${button.dataset.post}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: button.dataset.body || "{}",
    });
    const answer = await response.json().catch(() => ({}));
    say(said, answer.message || (response.ok ? "" : `The TV answered ${response.status}`), response.ok && answer.ok !== false);
  } catch {
    say(said, "The TV is not answering", false);
  } finally {
    button.removeAttribute("aria-busy");
  }
  if (button.dataset.after === "tracks") setTimeout(refreshTracks, SETTLE);
}

function choose(button) {
  const list = button.closest(".choices");
  if (!list) return;
  for (const other of list.querySelectorAll(".choice")) other.setAttribute("aria-pressed", String(other === button));
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

function choice(label, pressed, path, body) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "choice";
  button.textContent = label;
  button.setAttribute("aria-pressed", String(pressed));
  button.dataset.post = path;
  button.dataset.body = JSON.stringify(body);
  button.dataset.after = "tracks";
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
        choice("Off", !(playing.visible && playing.track !== null), "/api/subtitles/track", { track: null }),
        ...playing.tracks.map((track, index) =>
          choice(subtitleNames[index], playing.visible && playing.track === track.id, "/api/subtitles/track", { track: track.id }),
        ),
      ]
    : [];
  const audioTracks = (playing && playing.audio_tracks) || [];
  const audioNames = trackNames(audioTracks);
  const audio = audioTracks.length > 1
    ? audioTracks.map((track, index) =>
        choice(audioNames[index], playing.audio === track.id, "/api/audio/track", { track: track.id }),
      )
    : [];
  fill("subtitles", subtitles);
  fill("audio", audio);
}

function markSpeed() {
  const state = document.querySelector(".now-state");
  const speed = state ? state.dataset.speed : null;
  for (const button of document.querySelectorAll(".choice[data-speed]")) {
    button.setAttribute("aria-pressed", String(button.dataset.speed === speed));
  }
}

new MutationObserver(() => {
  const state = document.querySelector(".now-state");
  const key = state ? state.dataset.key : null;
  if (key !== shownKey) {
    shownKey = key;
    if (isOpen()) refreshTracks();
  }
  markSpeed();
}).observe(document.body, { childList: true, subtree: true });

function dragTo(offset) {
  sheet().style.setProperty("--drag", `${Math.max(offset, 0)}px`);
}

document.addEventListener("pointerdown", (event) => {
  const top = event.target.closest && event.target.closest(".sheet-top");
  if (!top || !isOpen() || (event.pointerType === "mouse" && event.button !== 0)) return;
  drag = { pointer: event.pointerId, start: event.clientY, moved: 0 };
  sheet().classList.add("dragging");
});

document.addEventListener("pointermove", (event) => {
  if (!drag || event.pointerId !== drag.pointer) return;
  drag.moved = event.clientY - drag.start;
  dragTo(drag.moved);
});

function endDrag(event) {
  if (!drag || event.pointerId !== drag.pointer) return;
  const far = drag.moved > CLOSE_DRAG;
  dragged = Math.abs(drag.moved) > 8;
  setTimeout(() => {
    dragged = false;
  }, 0);
  drag = null;
  sheet().classList.remove("dragging");
  dragTo(0);
  if (far) closeSheet();
}

document.addEventListener("pointerup", endDrag);
document.addEventListener("pointercancel", endDrag);

if (history.state && history.state.sheet) setOpen(true);
markSpeed();
