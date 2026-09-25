#!/usr/bin/env python3
"""Generate src/tables.mfb from the oracle's own compiled tables.

Every number in the output is printed by a C program linked against the oracle
engine (oracle/src), so the port's terrain, dungeon-feature, autogenerator and
dungeon-profile tables are the oracle's exact values, never hand-transcribed.
Enum member names come from oracle/src/brogue/Rogue.h; their values are also
printed by the C program, so a name can never drift from its number.

Usage: gen/gen_tables.py            (rewrites src/tables.mfb)
       gen/gen_tables.py --check    (exit 1 if src/tables.mfb is stale)
"""

import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
ORACLE = os.path.join(ROOT, "oracle", "src")
OUT = os.path.join(ROOT, "src", "tables.mfb")

# Enums whose member names become Integer constants in tables.mfb.
ENUMS = [
    "tileType",
    "dungeonFeatureTypes",
    "dungeonLayers",
    "dungeonProfileTypes",
    "directions",
    "tileFlags",
    "terrainFlagCatalog",
    "terrainMechanicalFlagCatalog",
    "DFFlags",
]

# C names that are MFBASIC keywords, and the name each is emitted as.
RENAMES = {"NOTHING": "NOTHING_TILE"}

# Plain #define constants the generator needs.
DEFINES = [
    "DCOLS", "DROWS", "ROOM_TYPE_COUNT",
    "HORIZONTAL_CORRIDOR_MIN_LENGTH", "HORIZONTAL_CORRIDOR_MAX_LENGTH",
    "VERTICAL_CORRIDOR_MIN_LENGTH", "VERTICAL_CORRIDOR_MAX_LENGTH",
    "CAVE_MIN_WIDTH", "CAVE_MIN_HEIGHT", "PDS_FORBIDDEN", "PDS_OBSTRUCTION",
]

# gameConstants fields the generator reads, taken from the Brogue variant.
GAME_CONSTANTS = [
    "deepestLevel", "amuletLevel", "minimumLavaLevel", "minimumBrimstoneLevel",
    "numberAutogenerators",
]


def enum_members(header, name):
    m = re.search(r"\benum\s+" + name + r"\s*\{(.*?)\};", header, re.S)
    if not m:
        sys.exit(f"gen_tables: enum {name} not found in Rogue.h")
    body = re.sub(r"//[^\n]*|/\*.*?\*/", "", m.group(1), flags=re.S)
    names = []
    for part in body.split(","):
        part = part.strip()
        if part:
            names.append(part.split("=")[0].strip())
    return names


def c_program(constants):
    lines = [
        '#include <stdio.h>',
        '#include "Rogue.h"',
        '#include "GlobalsBase.h"',
        '#include "Globals.h"',
        '#include "GlobalsBrogue.h"',
        'int main(void) {',
        '    initializeGameVariantBrogue();',
    ]
    for c in constants:
        lines.append(f'    printf("const {c} %lld\\n", (long long) ({c}));')
    for g in GAME_CONSTANTS:
        lines.append(f'    printf("const gameConst.{g} %lld\\n", (long long) gameConst->{g});')
    lines += [
        '    for (int i = 0; i < NUMBER_TILETYPES; i++) {',
        '        const floorTileType *t = &tileCatalog[i];',
        '        printf("tile %d %d %d %d %d %d %d %lu %lu\\n", t->drawPriority, t->chanceToIgnite,',
        '               t->fireType, t->discoverType, t->promoteType, t->promoteChance, t->glowLight,',
        '               t->flags, t->mechFlags);',
        '    }',
        '    for (int i = 0; i < NUMBER_DUNGEON_FEATURES; i++) {',
        '        const dungeonFeature *f = &dungeonFeatureCatalog[i];',
        '        printf("df %d %d %d %d %lu %d %d %d %d %d %d\\n", f->tile, f->layer, f->startProbability,',
        '               f->probabilityDecrement, f->flags, f->description[0] ? 1 : 0, f->lightFlare,',
        '               f->flashColor ? 1 : 0, f->effectRadius, f->propagationTerrain, f->subsequentDF);',
        '    }',
        '    for (int i = 0; i < gameConst->numberAutogenerators; i++) {',
        '        const autoGenerator *a = &autoGeneratorCatalog[i];',
        '        printf("ag %d %d %d %d %d %d %d %d %d %d %d %d\\n", a->terrain, a->layer, a->DFType, a->machine,',
        '               a->requiredDungeonFoundationType, a->requiredLiquidFoundationType, a->minDepth,',
        '               a->maxDepth, a->frequency, a->minNumberIntercept, a->minNumberSlope, a->maxNumber);',
        '    }',
        '    for (int i = 0; i < NUMBER_DUNGEON_PROFILES; i++) {',
        '        const dungeonProfile *p = &dungeonProfileCatalog[i];',
        '        printf("dp");',
        '        for (int k = 0; k < ROOM_TYPE_COUNT; k++) printf(" %d", p->roomFrequencies[k]);',
        '        printf(" %d\\n", p->corridorChance);',
        '    }',
        '    return 0;',
        '}',
    ]
    return "\n".join(lines) + "\n"


def run_oracle(constants):
    with tempfile.TemporaryDirectory() as work:
        src = os.path.join(work, "tables_dump.c")
        exe = os.path.join(work, "tables_dump")
        with open(src, "w") as f:
            f.write(c_program(constants))
        engine = [os.path.join(ORACLE, "brogue", n) for n in sorted(os.listdir(os.path.join(ORACLE, "brogue"))) if n.endswith(".c")]
        engine += [os.path.join(ORACLE, "variants", n) for n in sorted(os.listdir(os.path.join(ORACLE, "variants"))) if n.endswith(".c")]
        engine += [os.path.join(ORACLE, "platform", "platformdependent.c"),
                   os.path.join(ORACLE, "platform", "null-platform.c"),
                   os.path.join(ROOT, "check", "oracle_host.c")]
        subprocess.run(
            ["cc", "-std=c99", "-w", "-DBROGUE_EXTRA_VERSION=\"\"", "-DDATADIR=.",
             "-I" + os.path.join(ORACLE, "brogue"), "-I" + os.path.join(ORACLE, "platform"),
             "-I" + os.path.join(ORACLE, "variants"), src, *engine, "-o", exe],
            check=True)
        return subprocess.run([exe], check=True, capture_output=True, text=True).stdout


def emit(members, rows, consts):
    ints = lambda line: [int(v) for v in line.split()[1:]]
    tiles = [ints(r) for r in rows if r.startswith("tile ")]
    dfs = [ints(r) for r in rows if r.startswith("df ")]
    ags = [ints(r) for r in rows if r.startswith("ag ")]
    dps = [ints(r) for r in rows if r.startswith("dp ")]
    tile_names = members["tileType"]
    df_names = {consts[n]: n for n in members["dungeonFeatureTypes"]}
    dp_names = members["dungeonProfileTypes"]

    o = []
    w = o.append
    w("' tables — Brogue CE's terrain generation tables, as MFBASIC constants.")
    w("'")
    w("' GENERATED by gen/gen_tables.py from the oracle's compiled tables. Do not")
    w("' edit; change the generator and rerun it. Names are the oracle's own C names")
    w("' so the port reads line for line against oracle/src, except where a C name is")
    w("' an MFBASIC keyword: " + ", ".join(f"{k} is {v}" for k, v in RENAMES.items()) + ".")
    w("")
    w("IMPORT collections")
    w("")
    w("' #define constants.")
    for n in DEFINES:
        w(f"PUBLIC LET {n} AS Integer = {consts[n]}")
    w("")
    w("' gameConstants fields of the Brogue variant (variants/GlobalsBrogue.c).")
    for g in GAME_CONSTANTS:
        w(f"PUBLIC LET {g} AS Integer = {consts['gameConst.' + g]}")
    for e in ENUMS:
        w("")
        w(f"' enum {e}.")
        for n in members[e]:
            w(f"PUBLIC LET {RENAMES.get(n, n)} AS Integer = {consts[n]}")
    w("")
    w("' floorTileType, less its display and text fields.")
    w("PUBLIC TYPE FloorTileType")
    for f in ["drawPriority", "chanceToIgnite", "fireType", "discoverType", "promoteType",
              "promoteChance", "glowLight", "flags", "mechFlags"]:
        w(f"  {f} AS Integer")
    w("END TYPE")
    w("")
    w("' dungeonFeature, less its message text and colour; hasDescription and")
    w("' hasFlashColor record whether the C fields are set.")
    w("PUBLIC TYPE DungeonFeature")
    for f in ["tile", "layer", "startProbability", "probabilityDecrement", "flags"]:
        w(f"  {f} AS Integer")
    w("  hasDescription AS Boolean")
    w("  lightFlare AS Integer")
    w("  hasFlashColor AS Boolean")
    for f in ["effectRadius", "propagationTerrain", "subsequentDF"]:
        w(f"  {f} AS Integer")
    w("END TYPE")
    w("")
    w("PUBLIC TYPE AutoGenerator")
    ag_fields = ["terrain", "layer", "DFType", "machine", "requiredDungeonFoundationType",
                 "requiredLiquidFoundationType", "minDepth", "maxDepth", "frequency",
                 "minNumberIntercept", "minNumberSlope", "maxNumber"]
    for f in ag_fields:
        w(f"  {f} AS Integer")
    w("END TYPE")
    w("")
    w("PUBLIC TYPE DungeonProfile")
    w("  roomFrequencies AS List OF Integer")
    w("  corridorChance AS Integer")
    w("END TYPE")
    w("")

    tile_fields = ["drawPriority", "chanceToIgnite", "fireType", "discoverType", "promoteType",
                   "promoteChance", "glowLight", "flags", "mechFlags"]
    w("FUNC buildTileCatalog() AS List OF FloorTileType")
    w("  MUT t AS List OF FloorTileType = []")
    for i, t in enumerate(tiles):
        body = ", ".join(f"{f} := {v}" for f, v in zip(tile_fields, t))
        w(f"  t = collections::append(t, FloorTileType[{body}]) ' {tile_names[i]}")
    w("  RETURN t")
    w("END FUNC")
    w("")
    w("PUBLIC LET tileCatalog AS List OF FloorTileType = buildTileCatalog()")
    w("")
    w("FUNC buildDungeonFeatureCatalog() AS List OF DungeonFeature")
    w("  MUT t AS List OF DungeonFeature = []")
    for i, d in enumerate(dfs):
        vals = [str(d[0]), str(d[1]), str(d[2]), str(d[3]), str(d[4]),
                "TRUE" if d[5] else "FALSE", str(d[6]), "TRUE" if d[7] else "FALSE",
                str(d[8]), str(d[9]), str(d[10])]
        names = ["tile", "layer", "startProbability", "probabilityDecrement", "flags",
                 "hasDescription", "lightFlare", "hasFlashColor", "effectRadius",
                 "propagationTerrain", "subsequentDF"]
        body = ", ".join(f"{n} := {v}" for n, v in zip(names, vals))
        w(f"  t = collections::append(t, DungeonFeature[{body}]) ' {df_names.get(i, '(unnamed ' + str(i) + ')')}")
    w("  RETURN t")
    w("END FUNC")
    w("")
    w("PUBLIC LET dungeonFeatureCatalog AS List OF DungeonFeature = buildDungeonFeatureCatalog()")
    w("")
    w("' autoGeneratorCatalog_Brogue.")
    w("FUNC buildAutoGeneratorCatalog() AS List OF AutoGenerator")
    w("  MUT t AS List OF AutoGenerator = []")
    for i, a in enumerate(ags):
        body = ", ".join(f"{f} := {v}" for f, v in zip(ag_fields, a))
        w(f"  t = collections::append(t, AutoGenerator[{body}]) ' {i}")
    w("  RETURN t")
    w("END FUNC")
    w("")
    w("PUBLIC LET autoGeneratorCatalog AS List OF AutoGenerator = buildAutoGeneratorCatalog()")
    w("")
    w("FUNC buildDungeonProfileCatalog() AS List OF DungeonProfile")
    w("  MUT t AS List OF DungeonProfile = []")
    for i, p in enumerate(dps):
        freqs = ", ".join(str(v) for v in p[:-1])
        w(f"  t = collections::append(t, DungeonProfile[roomFrequencies := [{freqs}], corridorChance := {p[-1]}]) ' {dp_names[i]}")
    w("  RETURN t")
    w("END FUNC")
    w("")
    w("PUBLIC LET dungeonProfileCatalog AS List OF DungeonProfile = buildDungeonProfileCatalog()")
    return "\n".join(o) + "\n"


def main():
    with open(os.path.join(ORACLE, "brogue", "Rogue.h")) as f:
        header = f.read()
    members = {e: enum_members(header, e) for e in ENUMS}
    seen = set(DEFINES) | set(GAME_CONSTANTS)
    for e in ENUMS:
        for n in members[e]:
            if n in seen:
                sys.exit(f"gen_tables: {n} (enum {e}) is declared twice")
            seen.add(n)
    constants = DEFINES + [n for e in ENUMS for n in members[e]]
    out = run_oracle(constants).splitlines()
    consts = {}
    for line in out:
        if line.startswith("const "):
            _, name, value = line.split()
            consts[name] = int(value)
    text = emit(members, out, consts)
    if "--check" in sys.argv[1:]:
        with open(OUT) as f:
            if f.read() != text:
                print("gen_tables: src/tables.mfb is stale; rerun gen/gen_tables.py")
                sys.exit(1)
        return
    with open(OUT, "w") as f:
        f.write(text)
    print(f"gen_tables: wrote {os.path.relpath(OUT, ROOT)}")


if __name__ == "__main__":
    main()
