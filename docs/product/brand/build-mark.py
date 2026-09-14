#!/usr/bin/env python3
"""DohFlow mark — final construction.

Silhouette : the ORIGINAL shape the owner chose (top ∪ bottom masses ∪ bridge),
             so the distinctive bowl and taper are preserved exactly.
Bands      : vertical offsets of ONE curve, clipped to that silhouette — so the
             black traces the white exactly, and every band runs edge to edge.
Palette    : four FIXED colours. The mark is fully opaque, so it needs no
             theme-awareness and sits unchanged on any background.
"""
import re
src=open('A-1024.clean.svg').read(); P=re.findall(r'<path[^>]*/>',src)
d=lambda p: re.search(r'd="([^"]+)"',p).group(1)
TOP, BOT = d(P[1]), d(P[4])

GREEN, BLACK, WHITE, TERRA = "#006341", "#1c1815", "#FFFFFF", "#DB8F6B"
BRIDGE = "M247.1 645 L340 645 L340 712 L247.1 712 Z"

# One S-curve tracking the original wave's path through the mark, extended well
# past both edges so the CLIP decides where it terminates.
def wave(dy):
    return (f"M 170 {600+dy} C 300 {586+dy} 372 {528+dy} 470 {542+dy} "
            f"C 576 {557+dy} 628 {636+dy} 880 {498+dy}")
def rev(dy):
    return (f"L 880 {498+dy} C 628 {636+dy} 576 {557+dy} 470 {542+dy} "
            f"C 372 {528+dy} 300 {586+dy} 170 {600+dy} Z")

def build(black_h=44., white_h=36.):
    b1, b2 = black_h, black_h+white_h
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" role="img" aria-label="DohFlow">'
            f'<defs><clipPath id="c"><path d="{TOP}"/><path d="{BOT}"/><path d="{BRIDGE}"/></clipPath></defs>'
            f'<g clip-path="url(#c)">'
            f'<path fill="{GREEN}" d="{wave(0)} L 880 150 L 170 150 Z"/>'
            f'<path fill="{BLACK}" d="{wave(0)+" "+rev(b1)}"/>'
            f'<path fill="{WHITE}" d="{wave(b1)+" "+rev(b2)}"/>'
            f'<path fill="{TERRA}" d="{wave(b2)} L 880 880 L 170 880 Z"/>'
            f'</g></svg>')

for n,(bh,wh) in {'N1':(44.,36.),'N2':(54.,44.),'N3':(36.,30.)}.items():
    open(f'{n}.svg','w').write(build(bh,wh))
print("built N1, N2, N3")
