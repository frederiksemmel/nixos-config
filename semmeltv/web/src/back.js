const start = location.href;

window.addEventListener("click", (event) => {
  const back = event.target.closest?.("a.back");
  if (!back || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  const cameFromApp = location.href !== start || document.referrer.startsWith(`${location.origin}/`);
  if (!cameFromApp) return;
  event.preventDefault();
  event.stopPropagation();
  history.back();
}, true);
