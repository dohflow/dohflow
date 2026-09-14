#!/usr/bin/env python3
"""Path-data cleanup for SVG logo files.

Does: coordinate precision reduction, removal of degenerate (zero-length)
segments, and compact re-emission.

Does NOT do true node re-placement — moving bezier control points for better
optical curvature is vector-editor work, not something to fake programmatically.
"""
import re, sys

NUM = re.compile(r'-?\d*\.?\d+(?:[eE][-+]?\d+)?')
CMD = re.compile(r'([MmLlHhVvCcSsQqTtAaZz])')

def fmt(v, prec):
    s = f"{round(v, prec):.{prec}f}".rstrip('0').rstrip('.')
    return s if s not in ('-0', '') else '0'

ARGC = dict(M=2, L=2, H=1, V=1, C=6, S=4, Q=4, T=2, A=7, Z=0)

def clean_path(d, prec=1, eps=0.25):
    toks = [t for t in CMD.split(d) if t.strip()]
    out, i = [], 0
    cur = None          # current point
    dropped = 0
    while i < len(toks):
        c = toks[i]; i += 1
        n = ARGC[c.upper()]
        args = [float(x) for x in NUM.findall(toks[i])] if (n and i < len(toks) and not CMD.match(toks[i])) else []
        if args: i += 1
        if n == 0:
            out.append('Z'); continue
        for j in range(0, len(args), n):
            seg = args[j:j+n]
            if len(seg) < n: break
            if c.upper() == 'C' and cur is not None:
                ex, ey = seg[4], seg[5]
                # a cubic whose endpoint and both controls sit within eps of the
                # current point contributes nothing at any realistic render size
                if (abs(ex-cur[0]) < eps and abs(ey-cur[1]) < eps and
                    abs(seg[0]-cur[0]) < eps and abs(seg[1]-cur[1]) < eps and
                    abs(seg[2]-cur[0]) < eps and abs(seg[3]-cur[1]) < eps):
                    dropped += 1
                    continue
            if c.upper() in ('M','L','C','S','Q','T'):
                cur = (seg[-2], seg[-1])
            out.append(c + " ".join(fmt(v, prec) for v in seg))
    return "".join(out), dropped

def clean_svg(src, prec=1):
    total = 0
    def rep(m):
        nonlocal total
        d, dr = clean_path(m.group(1), prec)
        total += dr
        return f'd="{d}"'
    out = re.sub(r'd="([^"]+)"', rep, src)
    out = re.sub(r'>\s+<', '><', out).strip()
    return out, total

if __name__ == "__main__":
    src = open(sys.argv[1]).read()
    prec = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    out, dropped = clean_svg(src, prec)
    open(sys.argv[2], 'w').write(out)
    print(f"{sys.argv[1]} {len(src)}B -> {sys.argv[2]} {len(out)}B "
          f"({100*(1-len(out)/len(src)):.0f}% smaller), {dropped} degenerate segment(s) dropped")
