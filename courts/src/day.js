// The day page: filters that apply as soon as they change, your location for
// distances, and the map (MapLibre GL on swisstopo's light base map).

const MAPLIBRE = "https://unpkg.com/maplibre-gl@5.24.1/dist";
const STYLE = "https://vectortiles.geo.admin.ch/styles/ch.swisstopo.lightbasemap.vt/style.json";
const WIDE = matchMedia("(min-width: 1024px)");

const details = document.querySelector("[data-filters]");
const form = details && details.querySelector("form");

function openOnWide() {
  if (details) details.open = WIDE.matches || details.open;
}

if (form) {
  openOnWide();
  form.addEventListener("change", () => form.requestSubmit());
  form.addEventListener("click", (event) => {
    if (event.target.closest("[data-locate]")) locate(event.target.closest("[data-locate]"));
    if (event.target.closest("[data-home]")) {
      form.elements.near.value = "";
      form.requestSubmit();
    }
  });
}

function locate(button) {
  if (!("geolocation" in navigator)) {
    button.textContent = "This browser can't tell where you are";
    return;
  }
  button.textContent = "Finding you…";
  navigator.geolocation.getCurrentPosition(
    (position) => {
      form.elements.near.value = `${position.coords.latitude.toFixed(4)},${position.coords.longitude.toFixed(4)}`;
      form.requestSubmit();
    },
    () => (button.textContent = "Location not allowed"),
    { enableHighAccuracy: false, timeout: 10000, maximumAge: 300000 },
  );
}

function load(tag, attributes) {
  return new Promise((resolve, reject) => {
    const node = Object.assign(document.createElement(tag), attributes);
    node.onload = resolve;
    node.onerror = () => reject(new Error(`could not load ${attributes.src || attributes.href}`));
    document.head.append(node);
  });
}

let map = null;
const markers = new Map();
const holder = document.getElementById("map");

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function popup(pin) {
  const box = element("div", "pop");
  const title = element("a", "pop-title", pin.name);
  title.href = `/clubs/${pin.id}`;
  box.append(title, element("p", "pop-meta", `${pin.km} · ${pin.about}`));
  if (pin.times.length) {
    const times = element("div", "pop-times");
    for (const time of pin.times) {
      const link = element("a", "time", time.label);
      link.href = time.book;
      link.target = "_blank";
      link.rel = "noopener";
      times.append(link);
    }
    box.append(times);
  } else box.append(element("p", "pop-meta", "Nothing free that fits."));
  const actions = element("div", "pop-actions");
  const list = element("a", "pop-link", "Courts and times");
  list.href = `#club-${pin.id}`;
  const book = element("a", "pill", `Book ${pin.day}`);
  book.href = pin.book;
  book.target = "_blank";
  book.rel = "noopener";
  actions.append(list, book);
  box.append(actions);
  return box;
}

async function start() {
  if (map || !holder || holder.offsetWidth === 0) return;
  const pins = JSON.parse(holder.dataset.pins || "[]");
  const [lat, lng] = (holder.dataset.origin || "47.3779,8.5403").split(",").map(Number);
  try {
    await Promise.all([load("link", { rel: "stylesheet", href: `${MAPLIBRE}/maplibre-gl.css` }), load("script", { src: `${MAPLIBRE}/maplibre-gl.js` })]);
  } catch (error) {
    holder.textContent = `The map didn't load: ${error.message}.`;
    return;
  }
  map = new maplibregl.Map({
    container: holder,
    style: STYLE,
    center: [lng, lat],
    zoom: 12,
    attributionControl: { compact: true },
    cooperativeGestures: false,
  });
  map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-right");
  map.addControl(new maplibregl.GeolocateControl({ trackUserLocation: false, showUserLocation: true }), "top-right");
  const origin = element("div", "origin");
  origin.title = holder.dataset.here ? "You" : "Home";
  new maplibregl.Marker({ element: origin }).setLngLat([lng, lat]).addTo(map);
  const bounds = new maplibregl.LngLatBounds([lng, lat], [lng, lat]);
  const places = new Map();
  for (const pin of pins) {
    const key = `${pin.lat.toFixed(4)},${pin.lng.toFixed(4)}`;
    if (!places.has(key)) places.set(key, []);
    places.get(key).push(pin);
  }
  for (const group of places.values()) {
    group.sort((a, b) => (b.free ? 1 : 0) - (a.free ? 1 : 0));
    const first = group[0];
    const free = group.some((pin) => pin.free);
    const button = element("button", free ? "pin" : "pin none", free ? first.times[0].label : "–");
    button.type = "button";
    const names = group.map((pin) => pin.name).join(", ");
    button.setAttribute("aria-label", free ? `${names}: free from ${first.times[0].label}` : `${names}: nothing free`);
    const content = element("div", "pops");
    for (const pin of group) content.append(popup(pin));
    const marker = new maplibregl.Marker({ element: button })
      .setLngLat([first.lng, first.lat])
      .setPopup(new maplibregl.Popup({ offset: 18, maxWidth: "300px", closeButton: true }).setDOMContent(content))
      .addTo(map);
    marker.getElement().style.zIndex = free ? "2" : "1";
    button.addEventListener("click", () => highlight(first.id));
    for (const pin of group) markers.set(String(pin.id), marker);
    if (free) bounds.extend([first.lng, first.lat]);
  }
  if (pins.some((pin) => pin.free)) map.fitBounds(bounds, { padding: 56, maxZoom: 14, duration: 0 });
}

function highlight(id) {
  for (const card of document.querySelectorAll(".club.picked")) card.classList.remove("picked");
  const card = document.getElementById(`club-${id}`);
  if (card && WIDE.matches) {
    card.classList.add("picked");
    card.scrollIntoView({ block: "nearest", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }
}

document.addEventListener("click", (event) => {
  const show = event.target.closest("[data-show-on-map]");
  if (!show) return;
  const id = show.dataset.showOnMap;
  const go = () => {
    const marker = markers.get(id);
    if (!marker) return;
    map.flyTo({ center: marker.getLngLat(), zoom: Math.max(map.getZoom(), 14) });
    if (!marker.getPopup().isOpen()) marker.togglePopup();
  };
  if (WIDE.matches) go();
  else {
    const url = new URL(location.href);
    url.searchParams.set("view", "map");
    url.hash = `pin-${id}`;
    location.href = url.toString();
  }
});

async function boot() {
  await start();
  const wanted = location.hash.match(/^#pin-(\d+)$/);
  if (map && wanted) {
    map.once("idle", () => {
      const marker = markers.get(wanted[1]);
      if (!marker) return;
      map.jumpTo({ center: marker.getLngLat(), zoom: 14 });
      marker.togglePopup();
    });
  }
}

boot();
WIDE.addEventListener("change", () => {
  openOnWide();
  start();
  map?.resize();
});

document.addEventListener("click", async (event) => {
  const share = event.target.closest("[data-share]");
  const copy = event.target.closest("[data-copy-link]");
  const url = share?.dataset.share || copy?.dataset.copyLink;
  if (!url) return;
  if (share && navigator.share) {
    navigator.share({ title: "Courts", url }).catch(() => {});
    return;
  }
  try {
    await navigator.clipboard.writeText(url);
    (share || copy).textContent = "Copied";
  } catch {
    (share || copy).textContent = "Copy it from the text above";
  }
});
