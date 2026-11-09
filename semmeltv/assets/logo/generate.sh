#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

if ! command -v magick potrace resvg python3 >/dev/null; then
  exec nix shell --inputs-from ../../.. nixpkgs#imagemagick nixpkgs#potrace nixpkgs#resvg nixpkgs#python3 --command "$0"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

last=$(magick identify -format '%[fx:w-1],%[fx:h-1]' semmel-source.png)
magick semmel-source.png -alpha off -fuzz 12% -fill none \
  -draw "color 0,0 floodfill" -draw "color ${last%,*},0 floodfill" \
  -draw "color 0,${last#*,} floodfill" -draw "color ${last} floodfill" \
  -alpha extract -morphology Erode Disk:2 -blur 0x0.8 "$work/mask.png"
magick semmel-source.png -alpha off "$work/mask.png" -compose CopyOpacity -composite -trim +repage "$work/cut.png"
magick "$work/cut.png" -resize 480x semmel.png

flat="$work/flat.png"
magick "$work/cut.png" -background white -alpha remove "$flat"
magick "$work/cut.png" -alpha extract -threshold 50% -negate "$work/silhouette.pbm"
magick "$flat" -colorspace gray -threshold 22% "$work/ink.pbm"
magick "$work/cut.png" -alpha extract -morphology Erode Disk:40 "$work/inner.png"
magick "$flat" -fuzz 8% -fill white -opaque '#F7A0A0' -fill black +opaque white "$work/inner.png" -compose Multiply -composite -negate "$work/cheeks.pbm"
magick "$flat" "$work/inner.png" -compose Multiply -composite -colorspace gray -threshold 97% -negate "$work/shine.pbm"
for layer in silhouette ink cheeks shine; do
  potrace -b svg -t 40 -a 1.1 -O 0.4 -o "$work/$layer.svg" "$work/$layer.pbm"
done

python3 generate.py "$work"

resvg -w 1200 semmeltv-dark.svg semmeltv-dark.png
resvg -w 32 -h 32 favicon.svg favicon-32.png
for size in 180 192 512; do
  magick -size ${size}x${size} xc:'#FFF1D6' \( semmel.png -resize $((size * 3 / 4))x \) -gravity center -composite -strip "icon-$size.png"
done
mv icon-180.png apple-touch-icon.png
