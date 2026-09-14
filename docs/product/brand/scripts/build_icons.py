#!/usr/bin/env python3
"""Favicon / app-icon set from mark.svg."""
import re, io, cairosvg
from PIL import Image

PAPER, EMERALD = "#faf8f6", "#006341"
MARK = open('mark.svg').read()
# tight artwork bounds inside the 1024 canvas (measured)
NUM = re.compile(r'-?\d*\.?\d+')
xs, ys = [], []
for d in re.findall(r'd="([^"]+)"', MARK):
    n=[float(v) for v in NUM.findall(d)]; xs+=n[0::2]; ys+=n[1::2]
MX0, MY0, MX1, MY1 = min(xs), min(ys), max(xs), max(ys)
MW, MH = MX1-MX0, MY1-MY0
INNER = re.search(r'<svg[^>]*>(.*)</svg>', MARK, re.S).group(1).strip()

def compose(size, pad_frac=0.0, bg=None, radius_frac=None):
    """Mark centred on an optional background, with padding as a fraction of size."""
    avail = size * (1 - 2*pad_frac)
    s = min(avail/MW, avail/MH)
    w, h = MW*s, MH*s
    tx, ty = (size-w)/2 - MX0*s, (size-h)/2 - MY0*s
    bgel = ""
    if bg:
        if radius_frac:
            r = size*radius_frac
            bgel = f'<rect width="{size}" height="{size}" rx="{r}" ry="{r}" fill="{bg}"/>'
        else:
            bgel = f'<rect width="{size}" height="{size}" fill="{bg}"/>'
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">'
            f'{bgel}<g transform="translate({tx:.2f} {ty:.2f}) scale({s:.5f})">{INNER}</g></svg>')

def png(svg, size, out):
    cairosvg.svg2png(bytestring=svg.encode(), write_to=out, output_width=size, output_height=size)
    return out

# --- the set -------------------------------------------------------------
# Transparent, no padding: the mark is opaque four-colour art, so it reads on
# any tab bar without a background plate.
open('icon.svg','w').write(compose(512, pad_frac=0.02))

# Background is PAPER, not emerald: the mark's dominant green vanishes into an
# emerald plate and the D silhouette is lost. Verified by looking at the render.
png(compose(180, pad_frac=0.11, bg=PAPER), 180, 'apple-touch-icon.png')
png(compose(192, pad_frac=0.04), 192, 'icon-192.png')
png(compose(512, pad_frac=0.04), 512, 'icon-512.png')
# maskable: 409/512 safe zone => the art must sit inside the middle 80%
png(compose(512, pad_frac=0.135, bg=PAPER), 512, 'icon-mask-512.png')

# favicon.ico — multi-size from one render
ico = Image.open(io.BytesIO(cairosvg.svg2png(bytestring=compose(256, pad_frac=0.02).encode(),
                                             output_width=256, output_height=256)))
ico.save('favicon.ico', sizes=[(16,16),(32,32),(48,48)])
print("icons built:", "icon.svg apple-touch-icon.png icon-192 icon-512 icon-mask-512 favicon.ico")
