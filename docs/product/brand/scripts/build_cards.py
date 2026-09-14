#!/usr/bin/env python3
"""OG card, GitHub social preview, README headers, DMG background, PH thumbnail."""
import re, cairosvg, text

PAPER, INK, MUTED, EMERALD, TERRA = "#faf8f6", "#1c1815", "#6b6259", "#006341", "#DB8F6B"
DARK_BG, DARK_INK, DARK_MUTED = "#0e1411", "#eef2ef", "#a9b3ac"

def inner(p): return re.search(r'<svg[^>]*>(.*)</svg>', open(p).read(), re.S).group(1).strip()
def vbox(p):  return [float(v) for v in re.search(r'viewBox="([^"]+)"', open(p).read()).group(1).split()]

def place(path, x, y, height, fill=None):
    """Place an SVG file's content with its top-left at (x,y), scaled to `height`."""
    vb = vbox(path); s = height / vb[3]
    f = f' fill="{fill}"' if fill else ''
    return (f'<g{f} transform="translate({x:.2f} {y:.2f}) scale({s:.5f}) '
            f'translate({-vb[0]:.2f} {-vb[1]:.2f})">{inner(path)}</g>')

def lockup_w(path, height):
    vb = vbox(path); return vb[2] * (height / vb[3])

def card(w, h, bg, body):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">'
            f'<rect width="{w}" height="{h}" fill="{bg}"/>{body}</svg>')

def png(svg, out, w, h):
    cairosvg.svg2png(bytestring=svg.encode(), write_to=out, output_width=w, output_height=h)

# ---------------------------------------------------------------- OG 1200x630
def og(dark=False):
    bg, ink, muted = (DARK_BG, DARK_INK, DARK_MUTED) if dark else (PAPER, INK, MUTED)
    lk = 'lockup-substitute-paper.svg' if dark else 'lockup-substitute.svg'
    X, LH = 96, 104
    b  = place(lk, X, 190, LH)
    t, _ = text.text_paths("See your money before it happens.", 600, 44, X, 190+LH+86)
    b += f'<g fill="{ink}">{t}</g>'
    b += f'<rect x="{X}" y="{190+LH+120}" width="104" height="7" rx="3.5" fill="{TERRA}"/>'
    s, _ = text.text_paths("Local-first  ·  Encrypted  ·  Open source  ·  macOS", 500, 26, X, 190+LH+186)
    b += f'<g fill="{muted}">{s}</g>'
    return card(1200, 630, bg, b)

# ------------------------------------------------- GitHub social 1280x640
def social():
    X, LH = 110, 112
    b  = place('lockup-substitute.svg', X, 214, LH)
    t, _ = text.text_paths("Local-first personal finance for macOS", 600, 42, X, 214+LH+84)
    b += f'<g fill="{INK}">{t}</g>'
    s, _ = text.text_paths("Free and open source  ·  Your data never leaves your Mac", 500, 26, X, 214+LH+140)
    b += f'<g fill="{MUTED}">{s}</g>'
    return card(1280, 640, PAPER, b)

# ---------------------------------------------- README header 1280x320
def readme(dark=False):
    bg, ink, muted = (DARK_BG, DARK_INK, DARK_MUTED) if dark else (PAPER, INK, MUTED)
    lk = 'lockup-substitute-paper.svg' if dark else 'lockup-substitute.svg'
    LH = 72
    lw = lockup_w(lk, LH)
    b  = place(lk, (1280-lw)/2, 96, LH)
    t, tw = text.text_paths("See your money before it happens.", 600, 30, 0, 0)
    b += f'<g fill="{muted}" transform="translate({(1280-tw)/2:.1f} {228})">{t}</g>'
    return card(1280, 320, bg, b)

# ---------------------------------------------- DMG background
# Designed in Tauri's OWN coordinate space. tauri.conf.json currently has no
# bundle.macOS.dmg block, so these are Tauri v2's defaults, which the config
# change alongside this asset pins explicitly:
#   windowSize 660x400, appPosition (180,170), applicationFolderPosition (480,170)
# Positions are in 1x units; the @2x asset is the same layout at double scale.
DMG_W, DMG_H = 660, 400
APP_X, APP_Y = 180, 170
FLD_X, FLD_Y = 480, 170
ICON_HALF    = 64          # DMG icons render ~128 wide

def dmg():
    b = place('lockup-substitute.svg', 36, 30, 34)
    # arrow strictly between the two icon wells so it never sits under an icon
    x0 = APP_X + ICON_HALF + 22
    x1 = FLD_X - ICON_HALF - 22
    y  = APP_Y
    b += (f'<g stroke="{TERRA}" stroke-width="5" stroke-linecap="round" stroke-linejoin="round" fill="none">'
          f'<path d="M {x0} {y} L {x1} {y}"/>'
          f'<path d="M {x1-19} {y-17} L {x1} {y} L {x1-19} {y+17}"/></g>')
    t, tw = text.text_paths("Drag DohFlow into your Applications folder", 600, 17, 0, 0)
    b += f'<g fill="{MUTED}" transform="translate({(DMG_W-tw)/2:.1f} {334})">{t}</g>'
    return card(DMG_W, DMG_H, PAPER, b)

# ---------------------------------------------- Product Hunt thumb 240
def ph_thumb():
    return card(240, 240, PAPER, place('mark.svg', 0, 0, 240))

if __name__ == "__main__":
    png(og(),            'og.png',                    1200, 630)
    png(og(dark=True),   'og-dark.png',               1200, 630)
    png(social(),        'github-social-preview.png', 1280, 640)
    png(readme(),        'readme-header-light.png',   1280, 320)
    png(readme(True),    'readme-header-dark.png',    1280, 320)
    png(dmg(),           'dmg-background.png',         660,  400)
    png(dmg(),           'dmg-background@2x.png',     1320,  800)
    png(ph_thumb(),      'ph-thumbnail.png',           240, 240)
    print("cards built")
