"""Typeset a string in Nunito and return SVG path data + metrics, as outlines."""
import re, functools
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

@functools.lru_cache(maxsize=8)
def _font(weight):
    f = instantiateVariableFont(TTFont('Nunito.ttf'), {'wght': float(weight)},
                                inplace=False, updateFontNames=False)
    kern = {}
    if 'GPOS' in f:
        for lu in f['GPOS'].table.LookupList.Lookup:
            if lu.LookupType != 2: continue
            for st in lu.SubTable:
                if getattr(st,'Format',None) != 1 or not hasattr(st,'PairSet'): continue
                for first, ps in zip(st.Coverage.glyphs, st.PairSet):
                    for rec in ps.PairValueRecord:
                        v = getattr(rec.Value1,'XAdvance',0) if rec.Value1 else 0
                        if v: kern[(first, rec.SecondGlyph)] = v
    return f, kern

def text_paths(s, weight=600, size=100.0, x=0.0, y=0.0, tracking=0.0):
    """Returns (svg_path_string, advance_width_in_px). y is the BASELINE."""
    f, kern = _font(weight)
    upem = f['head'].unitsPerEm
    cmap, gs, hmtx = f.getBestCmap(), f.getGlyphSet(), f['hmtx']
    sc = size / upem
    names = [cmap[ord(c)] for c in s if ord(c) in cmap]
    pen_x = 0.0
    out = []
    for i, gn in enumerate(names):
        if i: pen_x += kern.get((names[i-1], gn), 0) + tracking*upem/size*0
        p = SVGPathPen(gs)
        gs[gn].draw(TransformPen(p, (sc, 0, 0, -sc, x + pen_x*sc, y)))
        d = p.getCommands()
        if d: out.append(d)
        pen_x += hmtx[gn][0] + (tracking * upem / size if tracking else 0)
    return "".join(f'<path d="{d}"/>' for d in out), pen_x*sc

def cap_height(weight=600, size=100.0):
    f,_ = _font(weight)
    return f['OS/2'].sCapHeight / f['head'].unitsPerEm * size
