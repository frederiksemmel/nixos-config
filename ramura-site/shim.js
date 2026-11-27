// Loaded before feedback-mode.js: each comment keeps the state the page was in,
// so the server can open the same projects again and take the screenshot.
window.feedbackMode = Object.assign(window.feedbackMode || {}, {
  context: () => ({
    width: document.documentElement.clientWidth,
    open: [...document.querySelectorAll("details[id]")].filter((d) => d.open).map((d) => d.id),
    hidden: [...document.querySelectorAll("details[id]")].filter((d) => d.hidden).map((d) => d.id),
    filter: document.querySelector(".chip.on")?.dataset.filter || null,
  }),
});
