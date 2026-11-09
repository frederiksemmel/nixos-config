const api = document.documentElement.dataset.api || "";
const HOLD = 4000;
const LANDED = 2;
const SLIP = 80;
const EDGE = 40;

let rail = null;
let seen = null;
let base = null;
let drag = null;
let hover = null;
let hold = null;

function clock(seconds) {
  const total = Math.floor(Math.max(seconds, 0));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const rest = String(total % 60).padStart(2, "0");
  return hours ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}` : `${minutes}:${rest}`;
}

function jumpLabel(seconds) {
  return `${seconds > 0 ? "+" : "−"}${Math.abs(seconds)}s`;
}

function read(element, now) {
  const data = element.dataset;
  return {
    position: Number(data.position) || 0,
    duration: Number(data.duration) || 0,
    paused: data.paused === "1",
    jump: data.jump ? Number(data.jump) : null,
    at: now,
  };
}

function playing(now) {
  if (!base) return 0;
  const moved = base.paused ? 0 : (now - base.at) / 1000;
  const position = base.position + moved;
  return base.duration > 0 ? Math.min(position, base.duration) : position;
}

function shown(now) {
  if (drag) return drag.fraction * base.duration;
  if (hold) {
    const landed = base.at > hold.sent && Math.abs(base.position - hold.to) < LANDED;
    if (landed || now > hold.until) hold = null;
    else return hold.to;
  }
  return playing(now);
}

function fraction(scrub, x) {
  const box = (scrub.querySelector(".scrub-rail") || scrub).getBoundingClientRect();
  return box.width > 0 ? Math.min(Math.max((x - box.left) / box.width, 0), 1) : 0;
}

function set(element, property, value) {
  if (element && element[property] !== value) element[property] = value;
}

function style(element, property, percent) {
  const value = `${percent.toFixed(3)}%`;
  if (element && element.style[property] !== value) element.style[property] = value;
}

function place(element, percent) {
  style(element, "left", percent);
}

function float(bubble, percent) {
  const width = rail.clientWidth;
  if (!bubble || width <= 0) return;
  const margin = Math.max(bubble.offsetWidth / 2 - EDGE, 0);
  const at = Math.min(Math.max((percent / 100) * width, margin), width - margin);
  style(bubble, "left", (at / width) * 100);
}

function render(now = performance.now()) {
  const scrub = document.querySelector(".scrub");
  const found = scrub && scrub.querySelector(".scrub-rail");
  if (!found) return;
  rail = found;
  const data = found.dataset;
  const state = `${data.position} ${data.duration} ${data.paused} ${data.jump}`;
  if (state !== seen) {
    seen = state;
    base = read(found, now);
  }
  const duration = base.duration;
  const position = shown(now);
  const percent = duration > 0 ? Math.min((position / duration) * 100, 100) : 0;
  const [elapsed, left] = scrub.querySelectorAll(".scrub-time");
  set(elapsed, "textContent", clock(position));
  set(left, "textContent", `-${clock(duration - position)}`);
  style(rail.querySelector(".scrub-fill"), "width", percent);
  place(rail.querySelector(".scrub-thumb"), percent);

  const bubble = rail.querySelector(".scrub-bubble");
  const jumping = !drag && !hold && base.jump !== null;
  if (drag) {
    set(bubble, "textContent", clock(position));
    float(bubble, percent);
  } else if (hover !== null && duration > 0) {
    set(bubble, "textContent", clock(hover * duration));
    float(bubble, hover * 100);
  } else if (jumping) {
    set(bubble, "textContent", `${clock(position)}  ${jumpLabel(base.jump)}`);
    float(bubble, percent);
  }
  scrub.classList.toggle("dragging", drag !== null);
  scrub.classList.toggle("hovering", !drag && hover !== null);
  scrub.classList.toggle("jumping", jumping);

  for (const button of document.querySelectorAll(".playing [data-skip]")) {
    const sign = button.dataset.skip === "back" ? -1 : 1;
    const amount = base.jump !== null && Math.sign(base.jump) === sign ? jumpLabel(base.jump) : "";
    set(button.querySelector(".skip-amount"), "textContent", amount);
    button.classList.toggle("skipped", amount !== "");
  }

  scrub.setAttribute("aria-valuemax", String(Math.round(duration)));
  scrub.setAttribute("aria-valuenow", String(Math.round(position)));
  scrub.setAttribute("aria-valuetext", `${clock(position)} of ${clock(duration)}`);
}

function post(path, body) {
  return fetch(`${api}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

function seek(to) {
  const now = performance.now();
  hold = { to, sent: now, until: now + HOLD };
  post("/api/playback/seek", { to }).then(
    (response) => {
      if (!response.ok) hold = null;
    },
    () => {
      hold = null;
    },
  );
}

document.addEventListener("pointerdown", (event) => {
  const scrub = event.target.closest && event.target.closest(".scrub");
  if (!scrub || !base || base.duration <= 0) return;
  if (event.pointerType === "mouse" && event.button !== 0) return;
  event.preventDefault();
  scrub.setPointerCapture(event.pointerId);
  drag = { pointer: event.pointerId, fraction: fraction(scrub, event.clientX) };
  render();
});

document.addEventListener("pointermove", (event) => {
  const scrub = document.querySelector(".scrub");
  if (!scrub) return;
  if (drag && event.pointerId === drag.pointer) {
    drag.fraction = fraction(scrub, event.clientX);
    render();
  } else if (!drag && event.pointerType === "mouse") {
    const over = event.target.closest && event.target.closest(".scrub");
    hover = over && base && base.duration > 0 ? fraction(scrub, event.clientX) : null;
    render();
  }
});

document.addEventListener("pointerout", (event) => {
  const into = event.relatedTarget;
  if (hover !== null && !(into && into.closest && into.closest(".scrub"))) {
    hover = null;
    render();
  }
});

document.addEventListener("pointerup", (event) => {
  if (!drag || event.pointerId !== drag.pointer) return;
  const scrub = document.querySelector(".scrub");
  const box = scrub && scrub.getBoundingClientRect();
  const away = box && (event.clientY < box.top - SLIP || event.clientY > box.bottom + SLIP);
  const to = drag.fraction * base.duration;
  drag = null;
  if (!away) seek(to);
  render();
});

for (const name of ["pointercancel", "lostpointercapture"]) {
  document.addEventListener(name, (event) => {
    if (drag && event.pointerId === drag.pointer) {
      drag = null;
      render();
    }
  });
}

document.addEventListener("keydown", (event) => {
  if (!event.target.closest || !event.target.closest(".scrub")) return;
  const direction = { ArrowLeft: "back", ArrowRight: "forward" }[event.key];
  if (!direction) return;
  event.preventDefault();
  post("/api/playback/skip", { direction }).catch(() => {});
});

new MutationObserver(() => render()).observe(document.documentElement, {
  childList: true,
  subtree: true,
  attributes: true,
  attributeFilter: ["data-position", "data-duration", "data-paused", "data-jump"],
});

function frame(now) {
  render(now);
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
