const api = document.documentElement.dataset.api || "";
const DRAFT = "semmeltv.feedback";
const LONGEST_SIDE = 1600;

let panel = null;
let shot = null;

function toast(text, tone, detail) {
  window.dispatchEvent(new CustomEvent("semmeltv:toast", { detail: { text, tone, detail } }));
}

function note(kind, message, detail) {
  window.semmeltvTrail?.note(kind, message, detail);
}

function remembered() {
  try {
    return localStorage.getItem(DRAFT) || "";
  } catch {
    return "";
  }
}

function remember(text) {
  try {
    if (text) localStorage.setItem(DRAFT, text);
    else localStorage.removeItem(DRAFT);
  } catch {}
}

function build() {
  const root = document.createElement("div");
  root.className = "feedback";
  root.hidden = true;
  root.innerHTML = `
    <div class="feedback-dim" data-close></div>
    <form class="feedback-panel" role="dialog" aria-modal="true" aria-labelledby="feedback-title">
      <div class="feedback-head">
        <h2 id="feedback-title">Feedback</h2>
        <button type="button" class="feedback-close" data-close aria-label="Close">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18"></path></svg>
        </button>
      </div>
      <textarea name="message" rows="5" maxlength="20000" placeholder="What happened, or what would you change? Type, or tap the microphone on the keyboard to dictate." aria-label="Your feedback"></textarea>
      <div class="feedback-shot" hidden>
        <img alt="Screenshot to send">
        <button type="button" class="feedback-unshot" aria-label="Remove the screenshot">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 7l10 10M17 7 7 17"></path></svg>
        </button>
      </div>
      <div class="feedback-actions">
        <label class="pill quiet feedback-attach">
          <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3.5" y="5.5" width="17" height="13" rx="2.5"></rect><circle cx="9" cy="10.5" r="1.6"></circle><path d="m20.5 15.5-5-4.5-8 7.5"></path></svg>
          <span>Add screenshot</span>
          <input type="file" accept="image/*">
        </label>
        <button type="submit" class="pill feedback-send">Send</button>
      </div>
      <p class="feedback-note">Your phone and the TV attach what they did in the last few minutes, so Frederik can see what went wrong.</p>
    </form>`;
  const form = root.querySelector("form");
  const field = root.querySelector("textarea");
  const file = root.querySelector("input[type=file]");
  field.value = remembered();
  field.addEventListener("input", () => remember(field.value.trim() ? field.value : ""));
  field.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      form.requestSubmit();
    }
  });
  root.addEventListener("click", (event) => {
    if (event.target.closest("[data-close]")) close();
  });
  root.addEventListener("keydown", (event) => {
    if (event.key === "Escape") close();
  });
  file.addEventListener("change", () => {
    const chosen = file.files && file.files[0];
    file.value = "";
    if (chosen) attach(chosen);
  });
  root.querySelector(".feedback-unshot").addEventListener("click", () => setShot(null));
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    submit(form, field);
  });
  return root;
}

function setShot(dataUrl) {
  shot = dataUrl;
  const holder = panel.querySelector(".feedback-shot");
  holder.hidden = !dataUrl;
  holder.querySelector("img").src = dataUrl || "";
  panel.querySelector(".feedback-attach span").textContent = dataUrl ? "Change screenshot" : "Add screenshot";
}

async function attach(chosen) {
  const url = URL.createObjectURL(chosen);
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const scale = Math.min(1, LONGEST_SIDE / Math.max(image.naturalWidth, image.naturalHeight));
    const canvas = document.createElement("canvas");
    canvas.width = Math.round(image.naturalWidth * scale);
    canvas.height = Math.round(image.naturalHeight * scale);
    canvas.getContext("2d").drawImage(image, 0, 0, canvas.width, canvas.height);
    setShot(canvas.toDataURL("image/jpeg", 0.8));
  } catch (error) {
    toast("That picture couldn't be opened", "error", String(error && error.message || error));
  } finally {
    URL.revokeObjectURL(url);
  }
}

async function submit(form, field) {
  const send = form.querySelector(".feedback-send");
  const message = field.value.trim();
  if (!message && !shot) {
    field.focus();
    toast("Write what happened first", "error");
    return;
  }
  send.disabled = true;
  send.textContent = "Sending…";
  const client = window.semmeltvTrail ? window.semmeltvTrail.snapshot() : { device: navigator.userAgent };
  try {
    const response = await fetch(`${api}/api/feedback`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      credentials: "include",
      body: JSON.stringify({ message, page: location.pathname + location.search, client, screenshot: shot }),
    });
    const answer = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(answer.error || `the TV answered ${response.status}`);
    note("feedback", `sent ${answer.id}`);
    field.value = "";
    remember("");
    setShot(null);
    close();
    toast("Feedback sent", "ok", "Thank you. It's saved on the TV.");
  } catch (error) {
    note("feedback", "not sent", error);
    toast("Feedback not sent", "error", `${error.message || error}. Your text is kept; try again.`);
  } finally {
    send.disabled = false;
    send.textContent = "Send";
  }
}

function open() {
  if (!panel) panel = build();
  if (!panel.isConnected) document.body.append(panel);
  panel.hidden = false;
  document.documentElement.dataset.feedback = "open";
  const field = panel.querySelector("textarea");
  field.focus({ preventScroll: true });
  field.setSelectionRange(field.value.length, field.value.length);
}

function close() {
  if (!panel || panel.hidden) return;
  panel.hidden = true;
  delete document.documentElement.dataset.feedback;
  document.getElementById("feedback-open")?.focus({ preventScroll: true });
}

document.addEventListener("click", async (event) => {
  if (event.target.closest?.("#feedback-open")) {
    event.preventDefault();
    if (panel && !panel.hidden) close();
    else open();
    return;
  }
  const copy = event.target.closest?.("[data-copy]");
  if (!copy) return;
  const source = document.getElementById(copy.dataset.copy);
  try {
    await navigator.clipboard.writeText(source ? source.textContent : "");
    toast("Copied", "ok", "Paste it to Claude.");
  } catch (error) {
    toast("Couldn't copy", "error", String(error && error.message || error));
  }
});

const DRAFTS = "semmeltv.draft.";

function keepDraft(field) {
  try {
    if (field.value.trim()) localStorage.setItem(DRAFTS + field.dataset.draft, field.value);
    else localStorage.removeItem(DRAFTS + field.dataset.draft);
  } catch {}
}

function restoreDrafts() {
  for (const field of document.querySelectorAll("textarea[data-draft]")) {
    if (field.value || field.dataset.restored) continue;
    field.dataset.restored = "1";
    let text = null;
    try {
      text = localStorage.getItem(DRAFTS + field.dataset.draft);
    } catch {}
    if (!text) continue;
    field.value = text;
    field.dispatchEvent(new Event("input", { bubbles: true }));
  }
}

document.addEventListener("input", (event) => {
  if (event.target.matches?.("textarea[data-draft]")) keepDraft(event.target);
});

document.addEventListener("submit", (event) => {
  const fields = [...(event.target.querySelectorAll?.("textarea[data-draft]") || [])];
  setTimeout(() => fields.forEach(keepDraft), 2000);
});

let restoring = 0;
new MutationObserver(() => {
  cancelAnimationFrame(restoring);
  restoring = requestAnimationFrame(restoreDrafts);
}).observe(document.documentElement, { childList: true, subtree: true });
window.addEventListener("load", restoreDrafts);
