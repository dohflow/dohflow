#!/usr/bin/env python3
"""Lockup variant: the mark stands in for the D in 'DohFlow'.

The mark is a solid blob while D is an outlined letterform with a counter, so
matching cap height exactly makes it read heavier than the type. Scale is a
parameter for that reason.
"""
import re, sys
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

WEIGHT = 800.0
REST   = "ohFlow"
CAP    = 705.0          # Nunito cap height at this weight
D_ADV  = 774.0          # advance of the D it replaces
D_RSB  = 774.0 - 725.0  # right side bearing to reproduce after the mark

f = instantiateVariableFont(TTFont('Nunito.ttf'), {'wght': WEIGHT}, inplace=False, updateFontNames=False)
cmap, gs, hmtx, glyf = f.getBestCmap(), f.getGlyphSet(), f['hmtx'], f['glyf']

kern = {}
for lu in f['GPOS'].table.LookupList.Lookup:
    if lu.LookupType != 2: continue
    for st in lu.SubTable:
        if getattr(st,'Format',None) != 1 or not hasattr(st,'PairSet'): continue
        for first, ps in zip(st.Coverage.glyphs, st.PairSet):
            for rec in ps.PairValueRecord:
                v = getattr(rec.Value1,'XAdvance',0) if rec.Value1 else 0
                if v: kern[(first, rec.SecondGlyph)] = v

NUM = re.compile(r'-?\d*\.?\d+')
def mark_bounds(p):
    xs, ys = [], []
    for d in re.findall(r'd="([^"]+)"', open(p).read()):
        n = [float(v) for v in NUM.findall(d)]
        xs += n[0::2]; ys += n[1::2]
    return min(xs), min(ys), max(xs), max(ys)

MARK = re.search(r'<svg[^>]*>(.*)</svg>', open('mark-opt.svg').read(), re.S).group(1).strip()
mx0, my0, mx1, my1 = mark_bounds('mark-opt.svg')
MW, MH = mx1-mx0, my1-my0

def build(scale=1.0, overshoot=0.012, fill="#006341"):
    """scale: mark height as a fraction of cap height.
       overshoot: how far below the baseline the round form dips, like an 'o'."""
    mh = CAP * scale
    ms = mh / MH
    mw = MW * ms
    ov = CAP * overshoot
    # letters after the mark
    names = [cmap[ord(c)] for c in REST]
    x = mw + D_RSB
    paths = []
    for i, gn in enumerate(names):
        if i: x += kern.get((names[i-1], gn), 0)
        pen = SVGPathPen(gs)
        gs[gn].draw(TransformPen(pen, (1, 0, 0, -1, x, 0)))
        d = pen.getCommands()
        if d: paths.append(d)
        x += hmtx[gn][0]
    adv = x
    ymax = max(glyf[g].yMax for g in names if glyf[g].numberOfContours)
    ymin = min(glyf[g].yMin for g in names if glyf[g].numberOfContours)
    top  = max(ymax, mh - ov)
    bot  = min(ymin, -ov)
    pad  = 60.0
    vb   = f"{-pad:.0f} {-top-pad:.0f} {adv+2*pad:.0f} {top-bot+2*pad:.0f}"
    # mark: baseline-aligned with a slight overshoot, like a round letterform
    g_mark = (f'<g transform="translate({-mx0*ms:.2f} {ov - my1*ms:.2f}) scale({ms:.5f})">{MARK}</g>')
    g_text = f'<g fill="{fill}">' + "".join(f'<path d="{d}"/>' for d in paths) + '</g>'
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}" role="img" aria-label="DohFlow">'
            f'{g_mark}{g_text}</svg>')

if __name__ == "__main__":
    for tag, sc in [('100', 1.00), ('94', 0.94), ('88', 0.88)]:
        open(f'sub-{tag}.svg','w').write(build(sc))
    open('sub-94-ink.svg','w').write(build(0.94, fill="#1c1815"))
    print("mark artwork", f"{MW:.0f}x{MH:.0f}", "| cap height", CAP)
    print("built sub-100, sub-94, sub-88, sub-94-ink")
