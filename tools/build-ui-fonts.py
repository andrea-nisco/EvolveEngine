#!/usr/bin/env python3
"""Builds the default UI fonts of crates/evolve_ui_text/fonts from their upstream releases.

All three families are under the SIL Open Font License 1.1 (the licence texts are stored next to the fonts). The files
are *subsets*, so that the player and the web download stay small:

  Inter-Regular.ttf, Inter-Bold.ttf   Inter 4.0 (https://github.com/rsms/inter), Latin, Greek, Cyrillic and symbols
  NotoSansArabic-Regular.ttf          Noto Sans Arabic (variable font, instanced at weight 400, width 100), Arabic blocks
  NotoSansJP-Regular.ttf              Noto Sans JP (variable font, instanced at weight 400), kana, punctuation and the
                                      6,355 characters of JIS X 0208 (levels 1 and 2)

Usage: python tools/build-ui-fonts.py <folder with the downloads> <output folder>
The downloads folder holds: Inter-Regular.ttf, Inter-Bold.ttf (from Inter-4.0.zip, extras/ttf),
NotoSansArabic[wdth,wght].ttf and NotoSansJP[wght].ttf (from github.com/google/fonts, ofl/).
Needs `pip install fonttools`.
"""
import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

LATIN = (
    list(range(0x0020, 0x0250))      # Basic Latin, Latin-1, Latin Extended-A and B
    + list(range(0x0370, 0x0400))    # Greek
    + list(range(0x0400, 0x0530))    # Cyrillic
    + list(range(0x1E00, 0x1F00))    # Latin Extended Additional
    + list(range(0x2000, 0x2070))    # general punctuation
    + list(range(0x20A0, 0x20C0))    # currency
    + list(range(0x2190, 0x2200))    # arrows
    + list(range(0x2200, 0x2300))    # mathematical operators, technical
    + list(range(0x25A0, 0x2600))    # geometric shapes, misc symbols
)
ARABIC = (
    list(range(0x0020, 0x0080))      # digits and punctuation used next to Arabic
    + list(range(0x0600, 0x0700))
    + list(range(0x0750, 0x0780))
    + list(range(0x08A0, 0x0900))
    + list(range(0x2000, 0x2070))
    + list(range(0xFB50, 0xFE00))
    + list(range(0xFE70, 0xFF00))
)


def jis_x_0208():
    """Every character of JIS X 0208 (kana, punctuation, 6,355 kanji) plus ASCII and the full-width forms."""
    chars = set(range(0x20, 0x7F))
    for lead in range(0xA1, 0xF5):
        for trail in range(0xA1, 0xFF):
            try:
                chars.add(ord(bytes([lead, trail]).decode("euc_jp")))
            except UnicodeDecodeError:
                pass
    chars.update(range(0x3000, 0x3100))   # CJK symbols, hiragana, katakana
    chars.update(range(0xFF00, 0xFFF0))   # full-width forms
    return sorted(chars)


def write_subset(font, unicodes, out):
    options = subset.Options()
    options.layout_features = ["*"]       # keep the shaping features (ligatures, kerning, Arabic joining...)
    options.glyph_names = False
    options.notdef_outline = True
    options.name_IDs = ["*"]
    options.hinting = False               # the renderer draws outlines, instructions are of no use
    options.desubroutinize = True
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=unicodes)
    subsetter.subset(font)
    font.save(out)
    print(f"{out}: {out.stat().st_size // 1024} KB")


def main(src, dst):
    src, dst = Path(src), Path(dst)
    dst.mkdir(parents=True, exist_ok=True)
    for name in ("Inter-Regular.ttf", "Inter-Bold.ttf"):
        write_subset(TTFont(src / name), LATIN, dst / name)
    arabic = instancer.instantiateVariableFont(TTFont(src / "NotoSansArabic[wdth,wght].ttf"), {"wght": 400, "wdth": 100})
    write_subset(arabic, ARABIC, dst / "NotoSansArabic-Regular.ttf")
    jp = instancer.instantiateVariableFont(TTFont(src / "NotoSansJP[wght].ttf"), {"wght": 400})
    write_subset(jp, jis_x_0208(), dst / "NotoSansJP-Regular.ttf")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
