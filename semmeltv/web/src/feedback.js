const api = document.documentElement.dataset.api || "";

function toast(text, tone, detail) {
  window.dispatchEvent(new CustomEvent("semmeltv:toast", { detail: { text, tone, detail } }));
}

function note(kind, message, detail) {
  window.semmeltvTrail?.note(kind, message, detail);
}

function message(batch) {
  const lines = batch.items.map((item) => `${item.n}. ${item.comment}\n   (${item.what}, ${item.page})`);
  return [lines.join("\n"), batch.note].filter(Boolean).join("\n\n");
}

async function send(batch) {
  const client = window.semmeltvTrail ? window.semmeltvTrail.snapshot() : { device: navigator.userAgent };
  const { photo, ...marks } = batch;
  client.feedback_mode = marks;
  try {
    const response = await fetch(`${api}/api/feedback`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      credentials: "include",
      body: JSON.stringify({ message: message(batch), page: batch.page, client, screenshot: photo || null }),
    });
    const answer = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(answer.error || `the TV answered ${response.status}`);
    note("feedback", `sent ${answer.id}`);
    return "Feedback sent. Thank you: it's saved on the TV, and Claude takes it up in a few minutes.";
  } catch (error) {
    note("feedback", "not sent", error);
    throw error;
  }
}

window.feedbackMode = Object.assign(window.feedbackMode || {}, { send });

document.addEventListener("click", async (event) => {
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

function ready() {
  const html = document.documentElement;
  if (!html.hasAttribute("data-feedback-ready")) html.dataset.feedbackReady = "";
}

ready();
new MutationObserver(ready).observe(document.documentElement, { attributes: true, attributeFilter: ["data-feedback-ready"] });
