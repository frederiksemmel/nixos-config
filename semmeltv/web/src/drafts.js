(() => {
  const DRAFTS = "semmeltv.draft.";
  const FIELDS = "textarea[data-draft]";

  function keep(field) {
    try {
      if (field.value.trim()) localStorage.setItem(DRAFTS + field.dataset.draft, field.value);
      else localStorage.removeItem(DRAFTS + field.dataset.draft);
    } catch {}
  }

  function kept(field) {
    try {
      return localStorage.getItem(DRAFTS + field.dataset.draft) || "";
    } catch {
      return "";
    }
  }

  function restore() {
    for (const field of document.querySelectorAll(FIELDS)) {
      if (field.dataset.restored) continue;
      field.dataset.restored = "1";
      const text = field.value || kept(field);
      if (!text) continue;
      field.value = text;
      field.dispatchEvent(new Event("input", { bubbles: true }));
    }
  }

  document.addEventListener("input", (event) => {
    if (event.target.matches?.(FIELDS)) keep(event.target);
  });

  function forget(form) {
    for (const field of form.querySelectorAll(FIELDS)) {
      try {
        localStorage.removeItem(DRAFTS + field.dataset.draft);
      } catch {}
    }
  }

  const sending = new Map();

  document.addEventListener("submit", (event) => {
    const form = event.target;
    if (form.querySelector?.(FIELDS)) sending.set(form, form.dataset.sent);
  });

  function confirmed(records) {
    for (const { target } of records) {
      if (!sending.has(target) || target.dataset.sent === sending.get(target)) continue;
      sending.delete(target);
      forget(target);
    }
  }

  function unsent(reason) {
    return reason instanceof TypeError || /^Procedure call failed/.test(reason?.message || "");
  }

  window.addEventListener("unhandledrejection", (event) => {
    if (!sending.size || !unsent(event.reason)) return;
    event.preventDefault();
    sending.clear();
    window.dispatchEvent(new CustomEvent("semmeltv:toast", {
      detail: { id: "unsent", text: "Not sent", detail: "SemmelTV didn't answer. Your text is still there; send it again.", tone: "error" },
    }));
  });

  document.addEventListener("DOMContentLoaded", () => {
    restore();
    new MutationObserver(restore).observe(document.documentElement, { childList: true, subtree: true });
    new MutationObserver(confirmed).observe(document.documentElement, { subtree: true, attributes: true, attributeFilter: ["data-sent"] });
  });
})();
