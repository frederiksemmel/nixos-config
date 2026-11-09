"""Builds the SemmelTV wordmarks and favicon from the traced Semmel.

Run ./generate.sh, which traces semmel-source.png and calls this.
"""

import base64
import re
import struct
import sys
from pathlib import Path

INTER_TV = [
    {
        "advance": 736,
        "left": 17.25,
        "right": 690.05,
        "d": "M690.1 1056V838.9H17.3V1056ZM200.7 1344H474.3V314.5Q474.3 259.8 496.6 238.5Q519 217.1 579.5 217.1"
        "Q604.3 217.1 637.5 217.1Q670.8 217.1 690.1 217.1V0Q663.6 0 618.4 0Q573.3 0 528.4 0Q360 0 280.4 66.7"
        "Q200.7 133.4 200.7 273.1Z",
    },
    {
        "advance": 1120,
        "left": 14.7,
        "right": 1105.3,
        "d": "M409.2 0 14.7 1056H303.1L486.6 520Q516.5 429.6 543.6 339.2Q570.7 248.8 597.1 158.4H531.8"
        "Q558.2 248.8 584.6 339.2Q611 429.6 639.9 520L820.3 1056H1105.3L710.8 0Z",
    },
]
INTER_T_HEIGHT = 1344
INTER_TRACKING = -20

INK = "#160C07"
CRUST = "#F8C473"
CRUMB = "#FDE6BD"
CHEEK = "#F59A9C"
MOUTH = "#E0474E"
SNOW = "#F4F4F6"


def num(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s == "-0" else s


def traced(work, name):
    svg = (work / f"{name}.svg").read_text()
    width, height = (float(v) for v in re.search(r'viewBox="0 0 ([\d.]+) ([\d.]+)"', svg).groups())
    transform = re.search(r'<g transform="([^"]+)"', svg).group(1)
    paths = [" ".join(d.split()) for d in re.findall(r'<path d="([^"]+)"', svg)]
    return width, height, transform, paths


def group(transform, paths, attrs):
    return f'<g transform="{transform}" {attrs}>' + "".join(f'<path d="{d}"/>' for d in paths) + "</g>"


def favicon(work):
    width, height, transform, silhouette = traced(work, "silhouette")
    *_, ink = traced(work, "ink")
    *_, cheeks = traced(work, "cheeks")
    *_, shine = traced(work, "shine")
    outline = 54
    pad = outline
    side = max(width, height) + 2 * pad
    x0 = (width - side) / 2
    y0 = (height - side) / 2
    face = ink[1:]
    mouth = min(face, key=lambda d: float(d.split()[1]))
    mouth_fill = mouth.split("z")[0] + "z"
    cheeks = sorted(cheeks, key=len)[-2:]
    body = (
        group(transform, silhouette, f'fill="{CRUST}" stroke="{INK}" stroke-width="{outline * 10}" stroke-linejoin="round"')
        + group(transform, cheeks, f'fill="{CHEEK}"')
        + group(transform, [mouth_fill], f'fill="{MOUTH}"')
        + group(transform, ink[:1], f'fill="{INK}"')
        + group(transform, face, f'fill="{INK}" stroke="{INK}" stroke-width="80" stroke-linejoin="round"')
        + group(transform, shine, 'fill="#fff"')
    )
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{num(x0)} {num(y0)} {num(side)} {num(side)}">{body}</svg>\n'


def png_size(data):
    return struct.unpack(">II", data[16:24])


def wordmark(semmel_png, ink):
    pixel_w, pixel_h = png_size(semmel_png)
    t_height = 100
    scale = t_height / INTER_T_HEIGHT
    semmel_h = 132
    semmel_w = semmel_h * pixel_w / pixel_h
    baseline = semmel_h - 14
    href = "data:image/png;base64," + base64.b64encode(semmel_png).decode()
    parts = [f'<image href="{href}" x="0" y="0" width="{num(semmel_w)}" height="{num(semmel_h)}"/>']
    x = semmel_w + 0.08 * semmel_w - INTER_TV[0]["left"] * scale
    for glyph in INTER_TV:
        parts.append(
            f'<path transform="matrix({num(scale * 1000)}e-3 0 0 {num(-scale * 1000)}e-3 {num(x)} {num(baseline)})" '
            f'd="{glyph["d"]}" fill="{ink}"/>'
        )
        last = x
        x += (glyph["advance"] + INTER_TRACKING) * scale
    right = last + INTER_TV[-1]["right"] * scale
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {num(right)} {num(semmel_h)}">{"".join(parts)}</svg>\n'


def main():
    work = Path(sys.argv[1])
    here = Path(__file__).parent
    semmel_png = (here / "semmel.png").read_bytes()
    (here / "favicon.svg").write_text(favicon(work))
    (here / "semmeltv-dark.svg").write_text(wordmark(semmel_png, SNOW))
    (here / "semmeltv-light.svg").write_text(wordmark(semmel_png, INK))


if __name__ == "__main__":
    main()
