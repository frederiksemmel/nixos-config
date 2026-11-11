const start = location.href;
const SHELL_MS = 8000;
const KEEP = 8;
const api = document.documentElement.dataset.api || "";
const pages = new Map();
const docUrl = (href) => href.split("#")[0];
let shownUrl = docUrl(location.href);

for (const name of ["pushState", "replaceState"]) {
  const real = history[name].bind(history);
  history[name] = (...args) => {
    const result = real(...args);
    shownUrl = docUrl(location.href);
    return result;
  };
}

function remember() {
  const main = document.querySelector("body > main");
  if (!main) return;
  pages.delete(shownUrl);
  pages.set(shownUrl, { main: main.cloneNode(true), scroll: scrollY });
  while (pages.size > KEEP) pages.delete(pages.keys().next().value);
}

function shell(main, offset) {
  document.querySelector("semmel-shell")?.remove();
  const element = document.createElement("semmel-shell");
  element.setAttribute("aria-hidden", "true");
  element.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
  }, true);
  main.style.transform = `translateY(${-offset}px)`;
  element.append(main);
  document.body.append(element);
  setTimeout(() => element.remove(), SHELL_MS);
}

function picture(src) {
  const image = document.createElement("img");
  image.alt = "";
  image.addEventListener("error", () => image.remove());
  image.src = src;
  return image;
}

function element(tag, className, ...children) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  node.append(...children);
  return node;
}

function bone(width) {
  const node = element("i", "bone");
  node.style.width = width;
  return node;
}

function skeleton(link) {
  const match = /^\/(movies|shows)\/([^/]+)$/.exec(new URL(link.href).pathname);
  if (!match) return null;
  const [, kind, key] = match;
  const title = (link.querySelector(".tile-name, .art-title")?.textContent || "").trim();
  const tint = link.querySelector(".art")?.getAttribute("style") || "";
  const hero = element("div", "hero", picture(`${api}/api/${kind}/${key}/backdrop`));
  hero.setAttribute("style", tint);
  const poster = element("span", "art poster detail-poster", element("span", "art-title", title), picture(`${api}/api/${kind}/${key}/poster`));
  poster.setAttribute("style", tint);
  const back = element("span", "back");
  back.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m14.5 5.5-6.5 6.5 6.5 6.5"></path></svg>Back';
  const article = element("article", "title detail nav-skeleton",
    back,
    hero,
    element("div", "title-body detail-body",
      element("div", "detail-head", poster, element("div", "detail-heading", element("h1", "", title), element("p", "detail-line", bone("70%")))),
      element("div", "bones", bone("100%"), bone("100%"), bone("85%"), bone("60%"))));
  return element("main", "", article);
}

window.addEventListener("click", (event) => {
  const back = event.target.closest?.("a.back");
  if (!back || back.closest("semmel-shell") || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  const cameFromApp = location.href !== start || document.referrer.startsWith(`${location.origin}/`);
  if (!cameFromApp) return;
  event.preventDefault();
  event.stopPropagation();
  history.back();
}, true);

window.addEventListener("click", (event) => {
  if (!event.defaultPrevented) return;
  const link = event.target.closest?.("a[data-topcoat-link]");
  if (!link || link.closest("semmel-shell") || event.target.closest("button, a") !== link) return;
  const url = new URL(link.href);
  const target = docUrl(url.href);
  if (url.origin !== location.origin || target === shownUrl) return;
  remember();
  const cached = pages.get(target);
  const main = cached ? cached.main.cloneNode(true) : skeleton(link);
  if (main) shell(main, 0);
});

window.addEventListener("popstate", () => {
  const state = history.state;
  const managed = state === null || (typeof state === "object" && "topcoat" in state);
  const target = docUrl(location.href);
  if (!managed || target === shownUrl) return;
  remember();
  shownUrl = target;
  const cached = pages.get(target);
  if (cached) shell(cached.main.cloneNode(true), state?.topcoat?.scroll?.y ?? cached.scroll);
});

const SLOP = 6;
let rowDrag = null;
let swallowClick = false;

function sidewaysScroller(target) {
  for (let node = target; node && node !== document.body; node = node.parentElement) {
    if (node.scrollWidth > node.clientWidth && getComputedStyle(node).overflowX === "auto") return node;
  }
  return null;
}

window.addEventListener("click", (event) => {
  if (!swallowClick) return;
  swallowClick = false;
  event.preventDefault();
  event.stopImmediatePropagation();
}, true);

document.addEventListener("dragstart", (event) => {
  if (event.target.closest?.("a, img")) event.preventDefault();
});

document.addEventListener("pointerdown", (event) => {
  if (event.pointerType !== "mouse" || event.button !== 0 || !event.target.closest) return;
  if (event.target.closest("input, textarea, select, .scrub, .sheet")) return;
  const row = sidewaysScroller(event.target);
  if (row) rowDrag = { row, pointer: event.pointerId, x: event.clientX, left: row.scrollLeft, moved: false };
});

document.addEventListener("pointermove", (event) => {
  if (!rowDrag || event.pointerId !== rowDrag.pointer) return;
  const dx = event.clientX - rowDrag.x;
  if (!rowDrag.moved) {
    if (Math.abs(dx) < SLOP) return;
    rowDrag.moved = true;
    rowDrag.row.setPointerCapture(event.pointerId);
    rowDrag.row.classList.add("dragging-row");
  }
  rowDrag.row.scrollLeft = rowDrag.left - dx;
});

for (const name of ["pointerup", "pointercancel"]) {
  document.addEventListener(name, (event) => {
    if (!rowDrag || event.pointerId !== rowDrag.pointer) return;
    const { row, moved } = rowDrag;
    rowDrag = null;
    if (!moved) return;
    row.classList.remove("dragging-row");
    swallowClick = true;
    setTimeout(() => {
      swallowClick = false;
    }, 0);
  });
}

for (const name of ["gesturestart", "gesturechange", "gestureend"]) {
  document.addEventListener(name, (event) => event.preventDefault(), { passive: false });
}
