# Spike material for surface 5b (decoders/media), findings DEC-50..DEC-60

Ready-to-check-in trigger programs. I did **not** write these into `spikes/audit-3/`
because my task brief forbade edits under `spikes/`; copy them there verbatim.

Each `DEC-NN/` is a buildable MFB project in the `spikes/sN` shape (`project.json` +
`src/main.mfb`). The crafted input files are produced by the scripts in `gen/` rather
than committed, for two reasons: the PNG ones are large (389 KB, 400 KB) and the font
ones are derived from an Apple system font, which should not be redistributed. Each
project reads its input path from an environment variable with a default, so the data
file can be dropped next to the project instead if that is preferred.

`DEC-50/51/52/53/54` need `-app` (the `canvas` package requires app mode) and are run
headless with `MFB_MACAPP_HEADLESS=1`. `DEC-55` is a plain console program.

| Finding | Spike | Build + run | Observed |
|---|---|---|---|
| DEC-50 | `DEC-50/` | `python3 gen/mkpng.py 4000 4000 /tmp/dec50.png` ; `mfb build DEC-50 -app` ; `IMGPATH=/tmp/dec50.png MFB_MACAPP_HEADLESS=1 DEC-50/build/DEC-50.app/Contents/MacOS/DEC-50` | 69-byte file → **4.95 GB RSS**, 4.4 s, then `ErrBadImageFile` |
| DEC-51 | `DEC-51/` | `python3 gen/mkbomb.py 400000000 /tmp/dec51.png` ; same build/run with `IMGPATH=/tmp/dec51.png` | 389 KB file → **25.0 GB RSS**, 24 s, and reports **success** ("decoded 1x1") |
| DEC-52 | `DEC-52/` | `python3 gen/mkchunks.py 20000 8 /tmp/dec52.png` ; same with `IMGPATH=/tmp/dec52.png` | 400 KB file, 160 KB payload → **2.47 GB RSS**, 5.6 s, succeeds |
| DEC-53 | `DEC-53/` | `python3 gen/mkfont.py "/System/Library/Fonts/Supplemental/Andale Mono.ttf" /tmp/dec53.ttf 4` ; `mfb build DEC-53 -app` ; `FONTPATH=/tmp/dec53.ttf MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 DEC-53/build/DEC-53.app/Contents/MacOS/DEC-53` | one character "A" → **62.7 s, 7.57 GB RSS** (unmodified font: 0.21 s, 23 MB) |
| DEC-54 | `DEC-54/` | `python3 gen/cmap12.py "/System/Library/Fonts/Supplemental/Andale Mono.ttf" /tmp/dec54.ttf 0xFFFFFFFF` ; same with `FONTPATH=/tmp/dec54.ttf` | one character "A" → **583.7 s wall / 471 s CPU**, flat memory (a hang) |
| DEC-55 | `DEC-55/` | `mfb build DEC-55` ; `TUNE="{ C }200000" DEC-55/build/DEC-55.out` | 15-character tune → **38.1 GB RSS**, killed abnormally after 229 s |

`DEC-56`..`DEC-60` have no spike of their own: DEC-56 is the enabler for DEC-53/54 and
has no independent impact; DEC-57/58/59 are latent (wrong pixels / wrong glyph, no
resource impact); DEC-60 is program-controlled rather than file-controlled.

The same two projects serve all six — `DEC-50/51/52` are identical apart from the
finding they are named for, and so are `DEC-53/54`. If the lead prefers one project per
*mechanism* rather than per finding, collapse them to `png-decoder/` and `font-glyph/`
and pass the input path.
