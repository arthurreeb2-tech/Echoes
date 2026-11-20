#!/usr/bin/env python3
"""Génère les SVG de la marque Echoes (icône d'app, glyphe, favicon) depuis une seule géométrie.

    python3 brand/generer.py        # écrit brand/*.svg

Les valeurs (couleurs, rayons) viennent de brand/tokens.json : changer la marque, c'est changer ce fichier.
"""
import json, math, pathlib

ICI = pathlib.Path(__file__).resolve().parent
T = json.loads((ICI / "tokens.json").read_text())
C = T["color"]["brand"]

def squircle(cx, cy, half, n=4.2, pas=160):
    """Super-ellipse |x|^n + |y|^n = 1 : le carré « continu » des icônes macOS."""
    pts = []
    for i in range(pas):
        t = 2 * math.pi * i / pas
        c, s = math.cos(t), math.sin(t)
        x = cx + half * math.copysign(abs(c) ** (2 / n), c)
        y = cy + half * math.copysign(abs(s) ** (2 / n), s)
        pts.append(f"{x:.1f},{y:.1f}")
    return "M" + " L".join(pts) + " Z"

def arc(cx, cy, r, debut, fin):
    """Arc de cercle de `debut` à `fin` degrés (0 = à droite, sens horaire à l'écran)."""
    a0, a1 = math.radians(debut), math.radians(fin)
    x0, y0 = cx + r * math.cos(a0), cy + r * math.sin(a0)
    x1, y1 = cx + r * math.cos(a1), cy + r * math.sin(a1)
    grand = 1 if (fin - debut) % 360 > 180 else 0
    return f"M{x0:.1f},{y0:.1f} A{r},{r} 0 {grand} 1 {x1:.1f},{y1:.1f}"

def point(cx, cy, r, angle):
    a = math.radians(angle)
    return cx + r * math.cos(a), cy + r * math.sin(a)

# Les ondes : chacune s'ouvre sur la même direction (en haut à droite, -45°), là où part la tâche.
# Plus l'onde s'éloigne, plus elle est fine et pâle : un écho qui s'éteint. Au bout de la dernière,
# un satellite : la machine qui reçoit le travail.
ONDES = [  # rayon, épaisseur, demi-ouverture (degrés), opacité
    (150, 46, 34, 1.00),
    (238, 36, 30, 0.78),
    (322, 26, 26, 0.52),
]
DIRECTION = -45
SATELLITE = (322 + 40, DIRECTION, 30)  # rayon orbite, angle, taille

def glyphe(cx, cy, echelle, trait="url(#onde)", plein="url(#coeur)", lueur=True, satellite=True):
    out = []
    for r, ep, ouv, op in ONDES:
        r2, ep2 = r * echelle, ep * echelle
        out.append(f'<path d="{arc(cx, cy, r2, DIRECTION + ouv, DIRECTION - ouv + 360)}" fill="none" '
                   f'stroke="{trait}" stroke-width="{ep2:.1f}" stroke-linecap="round" opacity="{op}"/>')
    if satellite:
        ro, ang, taille = SATELLITE
        x, y = point(cx, cy, ro * echelle, ang)
        out.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{taille * echelle:.1f}" fill="{plein}"/>')
    if lueur:
        out.append(f'<circle cx="{cx}" cy="{cy}" r="{120 * echelle:.1f}" fill="url(#halo)"/>')
    out.append(f'<circle cx="{cx}" cy="{cy}" r="{62 * echelle:.1f}" fill="{plein}"/>')
    return "\n  ".join(out)

DEFS = f"""<defs>
    <linearGradient id="fond" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{C['ink-700']}"/>
      <stop offset="1" stop-color="{C['ink-950']}"/>
    </linearGradient>
    <radialGradient id="chaleur" cx="0.5" cy="0.5" r="0.55">
      <stop offset="0" stop-color="{C['copper-500']}" stop-opacity="0.22"/>
      <stop offset="1" stop-color="{C['copper-500']}" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="onde" x1="0.15" y1="0.9" x2="0.85" y2="0.1">
      <stop offset="0" stop-color="{C['copper-600']}"/>
      <stop offset="0.55" stop-color="{C['copper-400']}"/>
      <stop offset="1" stop-color="{C['amber-300']}"/>
    </linearGradient>
    <radialGradient id="coeur" cx="0.38" cy="0.34" r="0.75">
      <stop offset="0" stop-color="{C['amber-100']}"/>
      <stop offset="0.45" stop-color="{C['amber-300']}"/>
      <stop offset="1" stop-color="{C['copper-500']}"/>
    </radialGradient>
    <radialGradient id="halo">
      <stop offset="0.4" stop-color="{C['amber-300']}" stop-opacity="0.45"/>
      <stop offset="1" stop-color="{C['amber-300']}" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="reflet" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.14"/>
      <stop offset="0.5" stop-color="#ffffff" stop-opacity="0"/>
    </linearGradient>
    <filter id="ombre" x="-10%" y="-10%" width="120%" height="125%">
      <feDropShadow dx="0" dy="12" stdDeviation="14" flood-color="#000" flood-opacity="0.35"/>
    </filter>
  </defs>"""

def icone_app():
    # Grille Apple : 1024 de côté, corps de 824 centré, ombre portée dans la marge.
    forme = squircle(512, 512, 412)
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <title>Echoes</title>
  {DEFS}
  <clipPath id="corps"><path d="{forme}"/></clipPath>
  <path d="{forme}" fill="url(#fond)" filter="url(#ombre)"/>
  <g clip-path="url(#corps)">
  <rect width="1024" height="1024" fill="url(#chaleur)"/>
  {glyphe(512, 512, 0.95)}
  <path d="{forme}" fill="url(#reflet)"/>
  </g>
  <path d="{forme}" fill="none" stroke="#ffffff" stroke-opacity="0.10" stroke-width="3"/>
</svg>
"""

def glyphe_seul(couleur=None):
    """Glyphe sans fond, 512 de côté. Sans couleur : dégradé cuivre. Avec : une teinte (currentColor…)."""
    corps = glyphe(256, 256, 0.62, lueur=couleur is None,
                   **({} if couleur is None else {"trait": couleur, "plein": couleur}))
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">
  <title>Echoes</title>
  {DEFS if couleur is None else ""}
  {corps}
</svg>
"""

def favicon():
    """Petites tailles : moins d'ondes, plus épaisses, pour rester lisible à 16 px."""
    forme = squircle(32, 32, 28.5, n=4.2, pas=72)
    o = DIRECTION
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">
  <title>Echoes</title>
  <defs>
    <linearGradient id="f" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{C['ink-700']}"/><stop offset="1" stop-color="{C['ink-950']}"/></linearGradient>
    <linearGradient id="o" x1="0.1" y1="0.9" x2="0.9" y2="0.1"><stop offset="0" stop-color="{C['copper-500']}"/><stop offset="1" stop-color="{C['amber-300']}"/></linearGradient>
  </defs>
  <path d="{forme}" fill="url(#f)"/>
  <path d="{arc(32, 32, 13, o + 36, o - 36 + 360)}" fill="none" stroke="url(#o)" stroke-width="5" stroke-linecap="round"/>
  <path d="{arc(32, 32, 21.5, o + 30, o - 30 + 360)}" fill="none" stroke="url(#o)" stroke-width="4" stroke-linecap="round" opacity="0.65"/>
  <circle cx="32" cy="32" r="6" fill="{C['amber-300']}"/>
</svg>
"""

if __name__ == "__main__":
    (ICI / "echoes-icone.svg").write_text(icone_app())
    (ICI / "echoes-glyphe.svg").write_text(glyphe_seul())
    (ICI / "echoes-glyphe-mono.svg").write_text(glyphe_seul("currentColor"))
    (ICI / "favicon.svg").write_text(favicon())
    print("SVG écrits dans", ICI)
