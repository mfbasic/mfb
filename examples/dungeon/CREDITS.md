# Credits

## Tiles (`resources/dungeon/`)

Source: Kenney (kenney.nl) — the isometric dungeon tile set distributed as
"Isometric Miniature Dungeon" / "Isometric Dungeon Tiles"
(`kenney_isometricDungeon.zip`, internally labelled "Dungeon Pack" in its
own `License.txt`, dated 2019-02-15). Confirmed against the pack's Angle/
directory: identical 72 tile names, four compass-facing variants each (288
files total), matching this repository's original (pre-bug-714) shipped
set exactly.

License: CC0 1.0 Universal (Creative Commons Zero).
<https://creativecommons.org/publicdomain/zero/1.0/>

CC0 requires no attribution, but Kenney asks (not mandatory) that users
credit "Kenney" or www.kenney.nl when convenient — hence this file.

Files covered: every `*.png` under `resources/dungeon/` (currently the 28
tiles `examples/dungeon/src/render.mfb:loadTiles` actually loads: `barrel`,
`stone`, `stoneMissingTiles`, `stoneWall`, `stoneWallArchway`,
`stoneWallCorner`, `tableShort`, each in its four compass-facing (`_N`,
`_S`, `_E`, `_W`) variants). The other 260 tiles from the same pack that
this example never loaded were removed in bug-714 rather than credited,
since an unused file needs no license record.
