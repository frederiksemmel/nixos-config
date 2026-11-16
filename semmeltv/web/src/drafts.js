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

  document.addEventListener("submit", (event) => {
    const fields = [...(event.target.querySelectorAll?.(FIELDS) || [])];
    setTimeout(() => fields.forEach(keep), 2000);
  });

  document.addEventListener("DOMContentLoaded", () => {
    restore();
    new MutationObserver(restore).observe(document.documentElement, { childList: true, subtree: true });
  });
})();
