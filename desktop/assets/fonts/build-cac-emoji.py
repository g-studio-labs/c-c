#!/usr/bin/env python3
"""Genera CacEmoji.ttf a partir de Noto Color Emoji (OFL 1.1).

Uso: build-cac-emoji.py NotoColorEmoji.ttf CacEmoji.ttf [Inter-*.ttf …]
(requiere fonttools)

Dos cambios sobre Noto, y por eso el nombre propio:
- sólo los emojis de la lista `KEEP` (el original pesa 10 MB; éstos, unos 0,7);
- las caras de Inter que se pasen de más se quedan sin ☀ ⚠ ❤: Inter los trae
  en blanco y negro, y como cubre el carácter, GPUI ni mira la reserva y el
  ❤️ salía gris. (Sólo se quita su entrada en `cmap`; los glifos no se tocan.)
- la letra «m» apunta al glifo del espacio: GPUI descarta como tipografía las
  caras que no cubren la «m», y sin ella `Noto Color Emoji` no se carga ni
  como letra de reserva.
"""
import sys

from fontTools import subset
from fontTools.ttLib import TTFont

KEEP = (
    "U+1F600-1F64F,U+1F910-1F92F,U+1F970-1F97F,U+1F440-1F4A0,U+1F4A1-1F4AF,"
    "U+1F389-1F38B,U+1F3C6,U+1F680,U+1F525,U+1F31F,U+1F308,U+2764,U+2705,"
    "U+2728,U+2B50,U+2714,U+274C,U+2757,U+2753,U+26A1,U+2600,U+2601,U+260E,"
    "U+2615,U+263A,U+270C,U+270B,U+270A,U+270D,U+2639,U+26D4,U+26A0,U+23F0,"
    "U+231B,U+200D,U+FE0F,U+0020"
)
FAMILY = "Cac Emoji"

source, target, *inter = sys.argv[1:]
options = subset.Options()
options.layout_features = ["*"]
options.notdef_outline = True
font = TTFont(source)
subsetter = subset.Subsetter(options)
subsetter.populate(unicodes=subset.parse_unicodes(KEEP))
subsetter.subset(font)

space = font.getBestCmap()[0x20]
for table in font["cmap"].tables:
    if table.isUnicode() and table.format in (4, 12):
        table.cmap[ord("m")] = space

for record in font["name"].names:
    if record.nameID in (1, 4, 16):
        record.string = FAMILY
    elif record.nameID == 3:
        record.string = FAMILY + ";modified"
font.save(target)

for path in inter:
    face = TTFont(path)
    for table in face["cmap"].tables:
        for codepoint in (0x2600, 0x26A0, 0x2764):
            table.cmap.pop(codepoint, None)
    face.save(path)
