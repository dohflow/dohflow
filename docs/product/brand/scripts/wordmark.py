#!/usr/bin/env python3
"""Typeset the DohFlow wordmark in Nunito and emit it as OUTLINES.

Outlines, not live text: a logo must not depend on the viewer having the font,
and ADR 0063 says the wordmark is typeset by hand and never generated.
"""
import sys
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.misc.transform import Identity

WEIGHT = float(sys.argv[1]) if len(sys.argv) > 1 else 800.0
TEXT   = "DohFlow"

f = TTFont('Nunito.ttf')
f = instantiateVariableFont(f, {'wght': WEIGHT}, inplace=False, updateFontNames=False)
upem = f['head'].unitsPerEm
cmap = f.getBestCmap()
gs   = f.getGlyphSet()
hmtx = f['hmtx']

# GPOS kerning pairs, where the font exposes them as a simple pair-position table.
kern = {}
if 'GPOS' in f:
    for lookup in f['GPOS'].table.LookupList.Lookup:
        if lookup.LookupType != 2:
            continue
        for st in lookup.SubTable:
            if getattr(st, 'Format', None) != 1 or not hasattr(st, 'PairSet'):
                continue
            for first, ps in zip(st.Coverage.glyphs, st.PairSet):
                for rec in ps.PairValueRecord:
                    v = getattr(rec.Value1, 'XAdvance', 0) if rec.Value1 else 0
                    if v:
                        kern[(first, rec.SecondGlyph)] = v

names = [cmap[ord(ch)] for ch in TEXT]
paths, x = [], 0.0
for i, gn in enumerate(names):
    if i:
        x += kern.get((names[i-1], gn), 0)
    pen = SVGPathPen(gs)
    # flip Y (font space is y-up, SVG is y-down) and place at the pen position
    tp = TransformPen(pen, (1, 0, 0, -1, x, 0))
    gs[gn].draw(tp)
    d = pen.getCommands()
    if d:
        paths.append(d)
    x += hmtx[gn][0]

adv = x
ymin = min(f['glyf'][g].yMin for g in names if f['glyf'][g].numberOfContours)
ymax = max(f['glyf'][g].yMax for g in names if f['glyf'][g].numberOfContours)
pad = upem * 0.06
vb = f"{-pad:.0f} {-ymax-pad:.0f} {adv+2*pad:.0f} {ymax-ymin+2*pad:.0f}"
body = "".join(f'<path d="{d}"/>' for d in paths)
svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}" role="img" aria-label="DohFlow" fill="#006341">'
       f'{body}</svg>')
open(f'wordmark-{int(WEIGHT)}.svg','w').write(svg)
print(f"weight {int(WEIGHT)}: advance {adv:.0f}, {len(paths)} glyph paths, "
      f"{len(kern)} kern pairs found, {len(svg)} bytes -> wordmark-{int(WEIGHT)}.svg")
