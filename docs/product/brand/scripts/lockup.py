#!/usr/bin/env python3
"""Build horizontal and stacked lockups from the mark + Nunito wordmark."""
import re

MARK_SRC = 'mark-opt.svg'
WM_SRC   = 'wordmark-800-opt.svg'

def inner(p):
    s = open(p).read()
    return re.search(r'<svg[^>]*>(.*)</svg>', s, re.S).group(1).strip()

def viewbox(p):
    return [float(v) for v in re.search(r'viewBox="([^"]+)"', open(p).read()).group(1).split()]

# Tight bounds of the mark's artwork inside its 1024 canvas (measured, not assumed)
NUM = re.compile(r'-?\d*\.?\d+')
def bounds(p):
    xs, ys = [], []
    for d in re.findall(r'd="([^"]+)"', open(p).read()):
        n = [float(v) for v in NUM.findall(d)]
        xs += n[0::2]; ys += n[1::2]
    return min(xs), min(ys), max(xs), max(ys)

mx0, my0, mx1, my1 = bounds(MARK_SRC)
MW, MH = mx1-mx0, my1-my0
wvb = viewbox(WM_SRC)
WW, WH = wvb[2], wvb[3]

MARK, WM = inner(MARK_SRC), inner(WM_SRC)

def horizontal(mark_h=100.0, gap_ratio=0.34, wm_ratio=0.62, wm_fill='#006341'):
    """Mark on the left, wordmark optically centred on it."""
    ms  = mark_h / MH
    mw  = MW * ms
    wh  = mark_h * wm_ratio
    ws  = wh / WH
    ww  = WW * ws
    gap = mark_h * gap_ratio
    total_w = mw + gap + ww
    wy = (mark_h - wh) / 2
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total_w:.1f} {mark_h:.1f}" '
            f'role="img" aria-label="DohFlow">'
            f'<g transform="translate({-mx0*ms:.2f} {-my0*ms:.2f}) scale({ms:.5f})">{MARK}</g>'
            f'<g fill="{wm_fill}" transform="translate({mw+gap:.2f} {wy:.2f}) scale({ws:.5f}) '
            f'translate({-wvb[0]:.2f} {-wvb[1]:.2f})">{WM}</g></svg>')

def stacked(mark_h=100.0, gap_ratio=0.22, wm_ratio=0.40, wm_fill='#006341'):
    ms  = mark_h / MH
    mw  = MW * ms
    wh  = mark_h * wm_ratio
    ws  = wh / WH
    ww  = WW * ws
    gap = mark_h * gap_ratio
    total_w = max(mw, ww)
    total_h = mark_h + gap + wh
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total_w:.1f} {total_h:.1f}" '
            f'role="img" aria-label="DohFlow">'
            f'<g transform="translate({(total_w-mw)/2 - mx0*ms:.2f} {-my0*ms:.2f}) scale({ms:.5f})">{MARK}</g>'
            f'<g fill="{wm_fill}" transform="translate({(total_w-ww)/2:.2f} {mark_h+gap:.2f}) scale({ws:.5f}) '
            f'translate({-wvb[0]:.2f} {-wvb[1]:.2f})">{WM}</g></svg>')

open('lockup-horizontal.svg','w').write(horizontal())
open('lockup-stacked.svg','w').write(stacked())
open('lockup-horizontal-ink.svg','w').write(horizontal(wm_fill='#1c1815'))
open('lockup-stacked-ink.svg','w').write(stacked(wm_fill='#1c1815'))
print(f"mark artwork  {MW:.0f} x {MH:.0f}  (inside a 1024 canvas)")
print(f"wordmark      {WW:.0f} x {WH:.0f}")
print("wrote lockup-horizontal.svg, lockup-stacked.svg")
