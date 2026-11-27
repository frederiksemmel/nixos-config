// The page that asks "which court suits you?": remembers the name someone
// gave, and lets the sender pass the link on again.

const NAME = "courts-name";
const field = document.querySelector(".ask-name input");

if (field) {
  try {
    field.value ||= localStorage.getItem(NAME) || "";
  } catch {}
  field.form.addEventListener("submit", () => {
    try {
      if (field.value.trim()) localStorage.setItem(NAME, field.value.trim());
    } catch {}
  });
}

document.addEventListener("click", async (event) => {
  const button = event.target.closest("[data-share]");
  if (!button) return;
  const url = button.dataset.share;
  if (navigator.share && matchMedia("(pointer: coarse)").matches) {
    navigator.share({ title: button.dataset.title || "Courts", url }).catch(() => {});
    return;
  }
  try {
    await navigator.clipboard.writeText(url);
    button.textContent = "Link copied";
  } catch {
    button.textContent = url;
  }
});
