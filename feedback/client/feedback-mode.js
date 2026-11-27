// Feedback mode: mark parts of a page and comment on them, then send them all
// at once. See feedback/README.md for the batch it sends.
//
// A page turns it on with attributes on <html>:
//   data-feedback-project="courts"        the project the comments belong to
//   data-feedback-send="/api/feedback"    where the batch is POSTed as JSON
//   data-feedback-list="/feedback"        optional: where sent comments are listed
//   data-feedback-launcher="off"          optional: no floating button; open it
//                                         from any element with data-feedback-open
// An app that sends differently sets window.feedbackMode.send = async (batch) => text.
// An app that wants the page's state with each comment sets
// window.feedbackMode.context = () => value; the value goes into the item.

const root = document.documentElement;
const project = root.dataset.feedbackProject || "site";
const DRAFT = `feedback-mode.${project}`;
const MOVE = 8;
const LONGEST_SIDE = 1600;

const kept = load();
const state = {
  on: false,
  scroll: false,
  items: kept.items,
  note: kept.note,
  photo: null,
  pending: null,
  pointers: new Map(),
  gesture: null,
};

const ui = {};

function load() {
  try {
    const kept = JSON.parse(localStorage.getItem(DRAFT) || "{}");
    return { items: Array.isArray(kept.items) ? kept.items : [], note: typeof kept.note === "string" ? kept.note : "" };
  } catch {
    return { items: [], note: "" };
  }
}

function save() {
  try {
    if (state.items.length || state.note) localStorage.setItem(DRAFT, JSON.stringify({ items: state.items, note: state.note }));
    else localStorage.removeItem(DRAFT);
  } catch {}
}

function ours(element) {
  return element && element.closest && element.closest("[data-fbm]");
}

function below(x, y) {
  return document.elementsFromPoint(x, y).find((element) => !ours(element) && element !== root && element !== document.body) || null;
}

const MEANINGFUL = new Set(["A", "BUTTON", "INPUT", "SELECT", "TEXTAREA", "LABEL", "LI", "P", "H1", "H2", "H3", "H4", "H5", "H6", "TD", "TH", "TR", "IMG", "SECTION", "ARTICLE", "NAV", "HEADER", "FOOTER", "FORM", "TABLE", "UL", "OL", "DL", "FIGURE"]);

function meaningful(element) {
  let current = element;
  while (current && current !== document.body) {
    if (MEANINGFUL.has(current.tagName) || current.getAttribute("role") || current.id) return current;
    const box = current.getBoundingClientRect();
    if (box.width * box.height > 4000 && getComputedStyle(current).display !== "inline") return current;
    current = current.parentElement;
  }
  return element;
}

function common(elements) {
  const list = elements.filter(Boolean);
  if (!list.length) return null;
  let candidate = list[0];
  while (candidate && !list.every((element) => candidate.contains(element))) candidate = candidate.parentElement;
  return candidate === document.body || candidate === root ? null : candidate;
}

function selector(element) {
  const parts = [];
  let current = element;
  while (current && current !== document.body && parts.length < 6) {
    if (current.id) {
      parts.unshift(`#${CSS.escape(current.id)}`);
      break;
    }
    let part = current.tagName.toLowerCase();
    const classes = [...current.classList].filter((name) => !name.startsWith("fbm")).slice(0, 2);
    if (classes.length) part += classes.map((name) => `.${CSS.escape(name)}`).join("");
    const parent = current.parentElement;
    if (parent) {
      const same = [...parent.children].filter((child) => child.tagName === current.tagName);
      if (same.length > 1) part += `:nth-of-type(${same.indexOf(current) + 1})`;
    }
    parts.unshift(part);
    current = current.parentElement;
  }
  return parts.join(" > ");
}

function clip(text, length) {
  const flat = (text || "").replace(/\s+/g, " ").trim();
  return flat.length > length ? `${flat.slice(0, length - 1)}…` : flat;
}

function describe(element) {
  const html = element.outerHTML || "";
  return {
    selector: selector(element),
    tag: element.tagName.toLowerCase(),
    text: clip(element.innerText || element.textContent, 300),
    label: element.getAttribute("aria-label") || element.getAttribute("title") || element.getAttribute("alt") || undefined,
    html: html.length > 600 ? `${html.slice(0, 599)}…` : html,
  };
}

function name(element) {
  const tag = element.tagName;
  const text = clip(element.getAttribute("aria-label") || element.innerText || element.getAttribute("alt") || "", 40);
  const kind =
    { A: "Link", BUTTON: "Button", INPUT: "Field", SELECT: "Menu", TEXTAREA: "Text field", IMG: "Picture", P: "Paragraph", LI: "List item", TD: "Cell", TH: "Heading cell", TR: "Row", TABLE: "Table", NAV: "Bar", HEADER: "Header", SECTION: "Section", ARTICLE: "Section", FORM: "Form", LABEL: "Label", UL: "List", OL: "List" }[tag] ||
    (/^H[1-6]$/.test(tag) ? "Heading" : "Area");
  return text ? `${kind} “${text}”` : kind;
}

function pageRect(box) {
  return { x: Math.round(box.left + scrollX), y: Math.round(box.top + scrollY), w: Math.round(box.width), h: Math.round(box.height) };
}

function svg(path) {
  return `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${path}"></path></svg>`;
}

function build() {
  const layer = document.createElement("div");
  layer.dataset.fbm = "layer";
  layer.className = "fbm-layer";

  const marks = document.createElement("div");
  marks.dataset.fbm = "marks";
  marks.className = "fbm-marks";

  const bar = document.createElement("div");
  bar.dataset.fbm = "bar";
  bar.className = "fbm-bar";
  bar.setAttribute("role", "toolbar");
  bar.setAttribute("aria-label", "Feedback mode");
  bar.innerHTML = `
    <div class="fbm-modes" role="radiogroup" aria-label="What touching the page does">
      <button type="button" class="fbm-mode" data-act="mark" role="radio">Mark</button>
      <button type="button" class="fbm-mode" data-act="scroll" role="radio">Scroll</button>
    </div>
    <button type="button" class="fbm-list-open" data-act="list"><span class="fbm-count">0</span><span class="fbm-count-word">comments</span></button>
    <button type="button" class="fbm-exit" data-act="exit" aria-label="Leave feedback mode">${svg("M6 6l12 12M18 6 6 18")}</button>`;

  const hint = document.createElement("p");
  hint.dataset.fbm = "hint";
  hint.className = "fbm-hint";
  hint.setAttribute("aria-live", "polite");

  const sheet = document.createElement("div");
  sheet.dataset.fbm = "sheet";
  sheet.className = "fbm-sheet";
  sheet.hidden = true;

  const launcher = document.createElement("button");
  launcher.dataset.fbm = "launcher";
  launcher.type = "button";
  launcher.className = "fbm-launcher";
  launcher.setAttribute("aria-label", "Feedback: mark something on this page and comment on it");
  launcher.innerHTML = `${svg("M7 16.5H5.5a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h13a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H12l-5 4Z")}<span>Feedback</span>`;
  launcher.hidden = root.dataset.feedbackLauncher === "off";

  Object.assign(ui, { layer, marks, bar, hint, sheet, launcher });
  document.body.append(marks, layer, hint, bar, sheet, launcher);

  launcher.addEventListener("click", () => start());
  bar.addEventListener("click", (event) => {
    const act = event.target.closest("[data-act]")?.dataset.act;
    if (act === "mark") setScroll(false);
    else if (act === "scroll") setScroll(true);
    else if (act === "list") showList();
    else if (act === "exit") stop();
  });
  layer.addEventListener("pointerdown", down);
  layer.addEventListener("pointermove", move);
  layer.addEventListener("pointerup", up);
  layer.addEventListener("pointercancel", cancel);
  layer.addEventListener("contextmenu", (event) => event.preventDefault());
  addEventListener("resize", drawMarks);
  document.addEventListener("keydown", (event) => {
    if (!state.on) return;
    if (event.key === "Escape") {
      if (!sheet.hidden) closeSheet();
      else stop();
    }
  });
}

function start() {
  if (!ui.layer) build();
  state.on = true;
  root.dataset.feedbackMode = "on";
  ui.launcher.hidden = true;
  setScroll(false);
  drawMarks();
  count();
}

function stop() {
  state.on = false;
  delete root.dataset.feedbackMode;
  closeSheet();
  clearPending();
  ui.launcher.hidden = root.dataset.feedbackLauncher === "off";
  drawMarks();
}

function setScroll(scroll) {
  state.scroll = scroll;
  root.dataset.feedbackScroll = scroll ? "on" : "off";
  for (const button of ui.bar.querySelectorAll(".fbm-mode")) {
    button.setAttribute("aria-checked", String((button.dataset.act === "scroll") === scroll));
  }
  hint(
    scroll
      ? "Scroll the page, then switch back to Mark."
      : matchMedia("(pointer: coarse)").matches
        ? "Tap something, drag across it, or draw a box with two fingers."
        : "Click something, drag across it, or Shift-drag a box.",
  );
}

function hint(text) {
  ui.hint.textContent = text;
}

function count() {
  const n = state.items.length;
  ui.bar.querySelector(".fbm-count").textContent = String(n);
  ui.bar.querySelector(".fbm-count-word").textContent = n === 1 ? "comment" : "comments";
  ui.bar.querySelector(".fbm-list-open").classList.toggle("fbm-has", n > 0);
}

// Gestures: a tap marks a point and what is under it, a drag the smallest
// thing it crossed, and two fingers (or Shift and drag) a box.

function down(event) {
  if (!state.sheetOpen) event.preventDefault();
  ui.layer.setPointerCapture?.(event.pointerId);
  state.pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  if (state.pointers.size === 2) {
    const [a, b] = [...state.pointers.values()];
    state.gesture = { kind: "box", a, b };
  } else if (state.pointers.size === 1) {
    state.gesture = {
      kind: event.shiftKey ? "box" : "tap",
      start: { x: event.clientX, y: event.clientY },
      a: { x: event.clientX, y: event.clientY },
      b: { x: event.clientX, y: event.clientY },
      path: [{ x: event.clientX, y: event.clientY }],
    };
  }
  preview();
}

function move(event) {
  if (!state.pointers.has(event.pointerId) || !state.gesture) return;
  event.preventDefault();
  const point = { x: event.clientX, y: event.clientY };
  state.pointers.set(event.pointerId, point);
  const gesture = state.gesture;
  if (gesture.kind === "box" && state.pointers.size === 2) {
    [gesture.a, gesture.b] = [...state.pointers.values()];
  } else if (gesture.kind === "box") {
    gesture.b = point;
  } else {
    gesture.path.push(point);
    if (gesture.kind === "tap" && Math.hypot(point.x - gesture.start.x, point.y - gesture.start.y) > MOVE) gesture.kind = "drag";
  }
  preview();
}

function up(event) {
  state.pointers.delete(event.pointerId);
  if (state.pointers.size > 0 || !state.gesture) return;
  const gesture = state.gesture;
  state.gesture = null;
  finish(gesture);
}

function cancel(event) {
  state.pointers.delete(event.pointerId);
  if (!state.pointers.size) {
    state.gesture = null;
    ui.marks.querySelector(".fbm-live")?.remove();
  }
}

function boxOf(gesture) {
  const left = Math.min(gesture.a.x, gesture.b.x);
  const top = Math.min(gesture.a.y, gesture.b.y);
  return { left, top, width: Math.abs(gesture.a.x - gesture.b.x), height: Math.abs(gesture.a.y - gesture.b.y) };
}

function crossed(path) {
  const step = Math.max(1, Math.floor(path.length / 24));
  const samples = path.filter((_, index) => index % step === 0).concat([path[path.length - 1]]);
  return common(samples.map((point) => below(point.x, point.y)));
}

function preview() {
  const gesture = state.gesture;
  let live = ui.marks.querySelector(".fbm-live");
  if (!gesture) return live?.remove();
  if (!live) {
    live = document.createElement("div");
    live.className = "fbm-live";
    ui.marks.append(live);
  }
  let box;
  if (gesture.kind === "box") {
    box = boxOf(gesture);
    live.dataset.kind = "box";
  } else {
    const element = gesture.kind === "drag" ? crossed(gesture.path) : meaningful(below(gesture.start.x, gesture.start.y));
    if (!element) return live.remove();
    box = element.getBoundingClientRect();
    live.dataset.kind = "element";
  }
  place(live, pageRect(box));
}

function place(node, rect) {
  node.style.transform = `translate(${rect.x}px, ${rect.y}px)`;
  node.style.width = `${rect.w}px`;
  node.style.height = `${rect.h}px`;
}

function finish(gesture) {
  ui.marks.querySelector(".fbm-live")?.remove();
  if (gesture.kind === "box") {
    const box = boxOf(gesture);
    if (box.width < 12 || box.height < 12) return;
    const inside = [...document.body.querySelectorAll("a, button, input, select, textarea, img, h1, h2, h3, h4, p, li, td, th, label")]
      .filter((element) => !ours(element))
      .filter((element) => {
        const r = element.getBoundingClientRect();
        return r.width && r.height && r.left >= box.left - 2 && r.top >= box.top - 2 && r.right <= box.left + box.width + 2 && r.bottom <= box.top + box.height + 2;
      })
      .slice(0, 12)
      .map((element) => ({ selector: selector(element), tag: element.tagName.toLowerCase(), text: clip(element.innerText || element.getAttribute("aria-label"), 80) }));
    const around = common([below(box.left + 2, box.top + 2), below(box.left + box.width - 2, box.top + box.height - 2)]);
    compose({
      kind: "box",
      rect: pageRect(box),
      target: around ? describe(around) : undefined,
      inside,
      what: `Box, ${Math.round(box.width)}×${Math.round(box.height)}${inside.length ? `, ${inside.length === 1 ? "1 thing" : `${inside.length} things`} in it` : ""}`,
    });
    return;
  }
  const element = gesture.kind === "drag" ? crossed(gesture.path) : meaningful(below(gesture.start.x, gesture.start.y));
  if (!element) return;
  compose({
    kind: gesture.kind === "drag" ? "drag" : "point",
    point: gesture.kind === "tap" ? { x: Math.round(gesture.start.x + scrollX), y: Math.round(gesture.start.y + scrollY) } : undefined,
    element,
  });
}

// The composer: what is selected, wider or narrower, and the comment.

function clearPending() {
  state.pending = null;
  ui.marks?.querySelector(".fbm-pending")?.remove();
}

function compose(pending) {
  state.pending = pending;
  state.pending.trail = pending.element ? [pending.element] : [];
  drawPending();
  openSheet("compose");
}

function current() {
  return state.pending.element || null;
}

function drawPending() {
  ui.marks.querySelector(".fbm-pending")?.remove();
  const pending = state.pending;
  if (!pending) return;
  const node = document.createElement("div");
  node.className = "fbm-pending";
  node.dataset.kind = pending.kind;
  const rect = pending.element ? pageRect(pending.element.getBoundingClientRect()) : pending.rect;
  place(node, rect);
  ui.marks.append(node);
  if (pending.point) {
    const dot = document.createElement("div");
    dot.className = "fbm-dot";
    dot.style.transform = `translate(${pending.point.x - rect.x}px, ${pending.point.y - rect.y}px)`;
    node.append(dot);
  }
}

function wider() {
  const element = current();
  if (!element?.parentElement || element.parentElement === document.body) return;
  state.pending.trail.push(element.parentElement);
  state.pending.element = element.parentElement;
  refreshCompose();
}

function narrower() {
  const trail = state.pending.trail;
  if (trail.length > 1) {
    trail.pop();
    state.pending.element = trail[trail.length - 1];
    refreshCompose();
  }
}

function refreshCompose() {
  drawPending();
  const what = ui.sheet.querySelector(".fbm-what");
  if (what) what.textContent = state.pending.element ? name(state.pending.element) : state.pending.what;
  const narrow = ui.sheet.querySelector("[data-act=narrower]");
  if (narrow) narrow.disabled = state.pending.trail.length <= 1;
}

function openSheet(kind, index) {
  state.sheetOpen = true;
  const sheet = ui.sheet;
  sheet.hidden = false;
  sheet.dataset.kind = kind;
  if (kind === "compose") {
    const editing = index !== undefined ? state.items[index] : null;
    const pending = state.pending;
    sheet.innerHTML = `
      <form class="fbm-card" data-fbm="compose">
        <div class="fbm-card-head">
          <p class="fbm-what"></p>
          ${pending && pending.element ? `<div class="fbm-scope" role="group" aria-label="Change what is selected">
            <button type="button" data-act="narrower" aria-label="Select less">${svg("M8 12h8")}</button>
            <button type="button" data-act="wider" aria-label="Select more">${svg("M12 8v8M8 12h8")}</button>
          </div>` : ""}
        </div>
        <textarea name="comment" rows="3" maxlength="4000" placeholder="What should change here?" aria-label="Your comment"></textarea>
        <div class="fbm-row">
          ${editing ? `<button type="button" class="fbm-quiet fbm-danger" data-act="remove">Remove</button>` : ""}
          <span class="fbm-grow"></span>
          <button type="button" class="fbm-quiet" data-act="cancel">Cancel</button>
          <button type="submit" class="fbm-primary">${editing ? "Save" : "Add comment"}</button>
        </div>
      </form>`;
    const form = sheet.querySelector("form");
    const field = form.elements.comment;
    if (editing) {
      field.value = editing.comment;
      sheet.querySelector(".fbm-what").textContent = editing.what;
    } else refreshCompose();
    form.addEventListener("click", (event) => {
      const act = event.target.closest("[data-act]")?.dataset.act;
      if (act === "wider") wider();
      else if (act === "narrower") narrower();
      else if (act === "cancel") {
        clearPending();
        closeSheet();
      } else if (act === "remove") {
        state.items.splice(index, 1);
        renumber();
        closeSheet();
      }
    });
    field.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        form.requestSubmit();
      }
    });
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      const text = field.value.trim();
      if (!text) return field.focus();
      if (editing) editing.comment = text;
      else add(text);
      clearPending();
      closeSheet();
      if (!editing) showListBriefly();
    });
    setTimeout(() => field.focus({ preventScroll: true }), 30);
  }
}

function add(comment) {
  const pending = state.pending;
  const element = pending.element;
  const rect = element ? pageRect(element.getBoundingClientRect()) : pending.rect;
  state.items.push({
    n: state.items.length + 1,
    kind: pending.kind,
    comment,
    what: element ? name(element) : pending.what,
    page: location.pathname + location.search,
    point: pending.point,
    rect,
    target: element ? describe(element) : pending.target,
    inside: pending.inside,
    scroll: { x: Math.round(scrollX), y: Math.round(scrollY) },
    context: window.feedbackMode?.context?.(),
  });
  renumber();
}

function renumber() {
  state.items.forEach((item, index) => (item.n = index + 1));
  save();
  count();
  drawMarks();
}

function showListBriefly() {
  hint(`${state.items.length === 1 ? "1 comment" : `${state.items.length} comments`} ready. Mark more, or open the list to send them.`);
}

function closeSheet() {
  state.sheetOpen = false;
  if (ui.sheet) {
    ui.sheet.hidden = true;
    ui.sheet.innerHTML = "";
  }
}

function drawMarks() {
  if (!ui.marks) return;
  for (const pin of ui.marks.querySelectorAll(".fbm-pin")) pin.remove();
  if (!state.on) return;
  const here = location.pathname + location.search;
  state.items.forEach((item, index) => {
    if (item.page !== here) return;
    const pin = document.createElement("button");
    pin.type = "button";
    pin.dataset.fbm = "pin";
    pin.className = "fbm-pin";
    pin.textContent = String(item.n);
    pin.setAttribute("aria-label", `Comment ${item.n}: ${item.comment}`);
    const at = item.point || { x: item.rect.x + item.rect.w, y: item.rect.y };
    pin.style.transform = `translate(${at.x - 12}px, ${at.y - 12}px)`;
    pin.addEventListener("click", () => openSheet("compose", index));
    ui.marks.append(pin);
  });
}

// The list: every comment, a note for all of them, and Send.

function showList() {
  state.sheetOpen = true;
  const sheet = ui.sheet;
  sheet.hidden = false;
  sheet.dataset.kind = "list";
  const items = state.items;
  const listLink = root.dataset.feedbackList;
  sheet.innerHTML = `
    <div class="fbm-card" data-fbm="list">
      <div class="fbm-card-head">
        <p class="fbm-title">${items.length ? `${items.length === 1 ? "1 comment" : `${items.length} comments`} to send` : "No comments yet"}</p>
        ${listLink ? `<a class="fbm-link" href="${listLink}">Sent earlier</a>` : ""}
      </div>
      <ol class="fbm-items">
        ${items
          .map(
            (item, index) => `<li><button type="button" data-edit="${index}">
              <span class="fbm-n">${item.n}</span>
              <span class="fbm-item-text"><span class="fbm-item-what"></span><span class="fbm-item-comment"></span></span></button></li>`,
          )
          .join("")}
      </ol>
      <textarea name="note" rows="2" maxlength="4000" placeholder="Anything else, about the page as a whole?" aria-label="A note for all comments"></textarea>
      <div class="fbm-photo" ${state.photo ? "" : "hidden"}><img alt="Photo to send"><button type="button" class="fbm-quiet" data-act="unphoto">Remove photo</button></div>
      <div class="fbm-row">
        <label class="fbm-quiet fbm-attach">Add photo<input type="file" accept="image/*"></label>
        <span class="fbm-grow"></span>
        <button type="button" class="fbm-quiet" data-act="close">Back</button>
        <button type="button" class="fbm-primary" data-act="send" ${items.length || state.note ? "" : "disabled"}>Send</button>
      </div>
      <p class="fbm-said" aria-live="polite"></p>
    </div>`;
  sheet.querySelectorAll(".fbm-items li").forEach((li, index) => {
    li.querySelector(".fbm-item-what").textContent = `${items[index].what}${items[index].page !== location.pathname + location.search ? ` · ${items[index].page}` : ""}`;
    li.querySelector(".fbm-item-comment").textContent = items[index].comment;
  });
  const note = sheet.querySelector("textarea");
  const send = sheet.querySelector("[data-act=send]");
  note.value = state.note;
  note.addEventListener("input", () => {
    state.note = note.value;
    save();
    send.disabled = !(state.items.length || state.note.trim());
  });
  if (state.photo) sheet.querySelector(".fbm-photo img").src = state.photo;
  sheet.querySelector("input[type=file]").addEventListener("change", async (event) => {
    const file = event.target.files && event.target.files[0];
    event.target.value = "";
    if (file) {
      state.photo = await shrink(file).catch(() => null);
      showList();
    }
  });
  sheet.querySelector("[data-fbm=list]").addEventListener("click", (event) => {
    const edit = event.target.closest("[data-edit]");
    if (edit) return openSheet("compose", Number(edit.dataset.edit));
    const act = event.target.closest("[data-act]")?.dataset.act;
    if (act === "close") closeSheet();
    else if (act === "unphoto") {
      state.photo = null;
      showList();
    } else if (act === "send") submit(send, sheet.querySelector(".fbm-said"));
  });
}

async function shrink(file) {
  const url = URL.createObjectURL(file);
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const scale = Math.min(1, LONGEST_SIDE / Math.max(image.naturalWidth, image.naturalHeight));
    const canvas = document.createElement("canvas");
    canvas.width = Math.round(image.naturalWidth * scale);
    canvas.height = Math.round(image.naturalHeight * scale);
    canvas.getContext("2d").drawImage(image, 0, 0, canvas.width, canvas.height);
    return canvas.toDataURL("image/jpeg", 0.8);
  } finally {
    URL.revokeObjectURL(url);
  }
}

function batch() {
  return {
    project,
    at: new Date().toISOString(),
    page: location.pathname + location.search,
    title: document.title,
    note: state.note.trim(),
    photo: state.photo || undefined,
    viewport: { width: innerWidth, height: innerHeight, scale: devicePixelRatio, coarse: matchMedia("(pointer: coarse)").matches },
    agent: navigator.userAgent,
    items: state.items,
  };
}

async function post(sent) {
  const response = await fetch(root.dataset.feedbackSend, {
    method: "POST",
    credentials: "include",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(sent),
  });
  const answer = await response.json().catch(() => ({}));
  if (!response.ok || answer.ok === false) throw new Error(answer.message || answer.error || `the server answered ${response.status}`);
  return answer.message || "";
}

async function submit(button, said) {
  button.disabled = true;
  button.textContent = "Sending…";
  try {
    const sender = window.feedbackMode && window.feedbackMode.send;
    const message = await (sender ? sender(batch()) : post(batch()));
    const n = state.items.length;
    state.items = [];
    state.note = "";
    state.photo = null;
    save();
    count();
    closeSheet();
    stop();
    toast(message || `Sent ${n === 1 ? "1 comment" : `${n} comments`}. Claude takes them up together in a few minutes.`);
  } catch (error) {
    said.textContent = `Not sent: ${error.message || error}. Your comments are kept; try again.`;
    button.disabled = false;
    button.textContent = "Send";
  }
}

function toast(text) {
  const node = document.createElement("p");
  node.dataset.fbm = "toast";
  node.className = "fbm-toast";
  node.setAttribute("role", "status");
  node.textContent = text;
  document.body.append(node);
  setTimeout(() => node.remove(), 5000);
}

document.addEventListener("click", (event) => {
  if (event.target.closest?.("[data-feedback-open]")) {
    event.preventDefault();
    if (state.on) stop();
    else start();
  }
});

window.feedbackMode = Object.assign(window.feedbackMode || {}, { start, stop });

function boot() {
  if (!ui.layer) build();
  count();
}

if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", boot);
else boot();
