function toast(text, tone, detail = "") {
  window.dispatchEvent(new CustomEvent("semmeltv:toast", { detail: { text, tone, detail } }));
}

function copy(message, url) {
  if (!navigator.clipboard) return toast("Copy this link", "info", url);
  navigator.clipboard.writeText(message).then(
    () => toast("Link copied", "ok"),
    () => toast("Copy this link", "info", url),
  );
}

document.addEventListener("click", (event) => {
  const button = event.target.closest && event.target.closest("[data-share]");
  if (!button) return;
  event.preventDefault();
  const url = new URL(button.dataset.share, location.origin).href;
  const message = `${button.dataset.shareText}\n${url}`;
  if (!navigator.share) return copy(message, url);
  navigator.share({ text: message }).catch((error) => {
    if (error && error.name !== "AbortError") copy(message, url);
  });
});

const shared = document.querySelector(".episode-row.shared");
if (shared) shared.scrollIntoView({ block: "center" });
