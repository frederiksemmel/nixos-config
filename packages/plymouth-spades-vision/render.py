"""Render sv-panel's LogoSpinner as a PNG sequence: white on black, 30 fps.

Usage: render.py <blackjackeye>/sv-panel/src/lib/components/LogoSpinner.svelte frames 216
Needs a python with playwright and PLAYWRIGHT_BROWSERS_PATH set to playwright-driver.browsers.
"""

import re
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

svelte, out_dir, size = Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3])
FPS, PERIOD_MS = 30, 1800

src = svelte.read_text()
svg = re.search(r"<svg.*?</svg>", src, re.S).group(0).replace("class={className}", 'id="logo"')
style = re.search(r"<style>(.*?)</style>", src, re.S).group(1)

html = f"""<!doctype html><html><head><style>
html, body {{ margin: 0; background: #000; }}
#logo {{ display: block; width: {size}px; height: {size}px; color: #fff; }}
{style}
</style></head><body>{svg}</body></html>"""

out_dir.mkdir(parents=True, exist_ok=True)
with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_page(viewport={"width": size, "height": size}, reduced_motion="no-preference")
    page.set_content(html)
    count = page.evaluate("document.getAnimations().length")
    assert count == 1, f"expected 1 animation, found {count}"
    frames = PERIOD_MS * FPS // 1000
    for i in range(frames):
        page.evaluate(
            "t => { const a = document.getAnimations()[0]; a.pause(); a.currentTime = t; }",
            i * 1000 / FPS,
        )
        page.locator("#logo").screenshot(path=out_dir / f"throbber-{i:04d}.png")
    browser.close()
print(f"{frames} frames -> {out_dir}")
