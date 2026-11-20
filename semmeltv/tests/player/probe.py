"""Prints which video and audio types each Playwright browser plays through
Media Source Extensions, the way hls.js feeds them.

    nix-shell --run "python3 -I probe.py firefox"
"""

import asyncio
import sys

from playwright.async_api import async_playwright

TYPES = [
    'video/mp4; codecs="hvc1.2.4.L120.90"',
    'video/mp4; codecs="avc1.640028"',
    'audio/mp4; codecs="mp4a.40.2"',
]

CHECK = """(types) => types.map((type) =>
  `${type}: MSE ${window.MediaSource ? MediaSource.isTypeSupported(type) : "missing"}`)"""


async def main(names):
    async with async_playwright() as playwright:
        for name in names:
            browser = await asyncio.wait_for(getattr(playwright, name).launch(), 30)
            page = await browser.new_page()
            print(name, browser.version)
            for line in await page.evaluate(CHECK, TYPES):
                print("  " + line)
            await browser.close()


asyncio.run(main(sys.argv[1:] or ["firefox"]))
