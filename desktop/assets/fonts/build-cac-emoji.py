#!/usr/bin/env python3
"""Genera CacEmoji.ttf a partir de Noto Color Emoji (OFL 1.1).

Uso: build-cac-emoji.py NotoColorEmoji.ttf CacEmoji.ttf [Inter-*.ttf …]
(requiere fonttools)

Tres cambios sobre Noto, y por eso el nombre propio de la familia:
- sólo los emojis de uso diario (el original pesa 10 MB). Las variantes de tono
  de piel y las secuencias con ZWJ sólo se conservan si están **todos** sus
  componentes, y los modificadores 🏻–🏿, el ZWJ, el selector FE0F y el 20E3
  van siempre: una secuencia a medias no se pinta.
- la letra «m» apunta al glifo del espacio: GPUI descarta como tipografía las
  caras que no cubren la «m», y sin ella `Noto Color Emoji` no se carga ni
  como letra de reserva.
- las caras de Inter que se pasen de más se quedan sin ☀ ⚠ ❤: Inter los trae
  en blanco y negro, y como cubre el carácter, GPUI ni mira la reserva y el
  ❤️ salía gris. (Sólo se quita su entrada en `cmap`; los glifos no se tocan.)

Lo que GPUI da por hecho y aquí se conserva (el script lo comprueba al final):
- El nombre PostScript sigue siendo `NotoColorEmoji`: GPUI sólo pinta en color
  las caras que se llaman así (`check_is_known_emoji_font`). Sólo se cambia el
  nombre de la familia.
- El glifo 3 es el espacio. GPUI se salta ese glifo en esa cara
  (`glyph_id == 3`) porque el shaper pone el espacio de la cara donde van el
  FE0F y el ZWJ; si el espacio cae en otro número, se intenta pintar como
  emoji, no tiene imagen, y el `?` de `paint` deja sin pintar el resto de la
  línea. Por eso se conservan U+0000 y U+000D (los glifos 1 y 2) y no se
  renumera nada.
- Todo glifo alcanzable por `cmap` o por una ligadura tiene imagen, salvo ese
  espacio.

macOS y Windows: allí `cosmic_text` no usa la lista de reserva de Linux, y
esta cara va de reserva propia; no se ha medido ni se sabe que haga falta. No
empeora nada respecto a antes.
"""
import sys

from fontTools import subset
from fontTools.ttLib import TTFont

FAMILY = "Cac Emoji"
POSTSCRIPT = "NotoColorEmoji"
TONES = range(0x1F3FB, 0x1F400)

# Rangos enteros: caras, objetos, comida, animales, transporte, símbolos.
RANGES = [
    (0x1F300, 0x1F54F), (0x1F568, 0x1F5FF), (0x1F600, 0x1F64F), (0x1F680, 0x1F6FF),
    (0x1F7E0, 0x1F7EB), (0x1F900, 0x1F9FF), (0x1FA70, 0x1FA7C), (0x1FAE0, 0x1FAF8),
    (0x2600, 0x27BF), (0x23E9, 0x23F3), (0x25AA, 0x25FE),
]
LOOSE = [
    0x00A9, 0x00AE, 0x203C, 0x2049, 0x2122, 0x2139, 0x231A, 0x231B, 0x24C2,
    0x2934, 0x2935, 0x2B05, 0x2B06, 0x2B07, 0x2B1B, 0x2B1C, 0x2B50, 0x2B55,
    0x3030, 0x303D, 0x3297, 0x3299, 0x0023, 0x002A, *range(0x30, 0x3A),
]
# Los que sí llevan tono de piel (cada uno trae cinco variantes, y las
# secuencias con ellos otras tantas): sólo los de todos los días.
WITH_TONES = [
    0x261D, 0x270A, 0x270B, 0x270C, 0x270D, 0x1F44A, 0x1F44B, 0x1F44C,
    0x1F44D, 0x1F44E, 0x1F44F, 0x1F450, 0x1F4AA, 0x1F595, 0x1F64B, 0x1F64C, 0x1F64F, 0x1F645,
    0x1F646, 0x1F647, 0x1F481, 0x1F91D, 0x1F91E, 0x1F91F, 0x1F918, 0x1F919,
    0x1F926, 0x1F937,
    0x1FAF0, 0x1FAF6, 0x1F90C, 0x1F90F,
]
STRUCTURE = [0x0000, 0x000D, 0x0020, 0x200D, 0xFE0F, 0x20E3, *TONES]

source, target, *inter = sys.argv[1:]
font = TTFont(source)
cmap = font.getBestCmap()
reverse = {glyph: cp for cp, glyph in cmap.items()}

keep = set(STRUCTURE) | set(LOOSE) | set(WITH_TONES) | {0x1F468, 0x1F469}
for low, high in RANGES:
    keep |= set(range(low, high + 1))
keep &= set(cmap)
# Las bases con tono que no están en la lista salen enteras, y con ellas
# sus cinco variantes y todas las secuencias que las usan: pesan más que
# el resto junto. Se quitan (siguen saliendo del sistema, si lo tiene).
tone_bases = set()
ligatures = []
for lookup in font["GSUB"].table.LookupList.Lookup:
    for table in lookup.SubTable:
        table = getattr(table, "ExtSubTable", table)
        for first, group in getattr(table, "ligatures", {}).items():
            for ligature in group:
                parts = [reverse.get(name) for name in [first, *ligature.Component]]
                if None in parts:
                    continue
                ligatures.append((parts, ligature.LigGlyph))
                if len(parts) == 2 and parts[1] in TONES:
                    tone_bases.add(parts[0])
keep -= tone_bases - set(WITH_TONES) - {0x1F468, 0x1F469}

# Las ligaduras se eligen una a una, sin dejar que el subsetter cierre por su
# cuenta (con los modificadores presentes arrastraría las de todos los tonos).
glyphs = {cmap[cp] for cp in keep}
for parts, glyph in ligatures:
    if not set(parts) <= keep:
        continue
    tone = [cp in TONES for cp in parts]
    if any(tone) and not (len(parts) == 2 and parts[0] in set(WITH_TONES) | {0x1F468, 0x1F469}):
        continue
    glyphs.add(glyph)

options = subset.Options()
options.layout_features = ["*"]
options.layout_closure = False
options.name_IDs = ["*"]
subsetter = subset.Subsetter(options)
subsetter.populate(glyphs=sorted(glyphs, key=font.getGlyphID))
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

check = TTFont(target)
assert check.getGlyphOrder()[3] == space == "space", "el espacio no es el glifo 3"
assert check.getBestCmap()[ord("m")] == "space"
assert any(r.nameID == 6 and str(r) == POSTSCRIPT for r in check["name"].names)
assert any(r.nameID == 13 for r in check["name"].names), "falta la licencia"

for path in inter:
    face = TTFont(path)
    for table in face["cmap"].tables:
        for codepoint in (0x2600, 0x26A0, 0x2764):
            table.cmap.pop(codepoint, None)
    face.save(path)
