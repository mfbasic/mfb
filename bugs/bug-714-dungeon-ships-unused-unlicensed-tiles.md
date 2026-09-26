# bug-714: `examples/dungeon` ships 288 tile images (5.3 MB), uses 28, and none carry a license or attribution

Last updated: 2026-09-25
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Other (packaging / licensing)

Status: Open
Regression Test: a resource-usage audit script asserting every shipped
`.png` under an example's `resources/` is referenced by that example's
source (see "Validation Plan"); no existing test covers this

`examples/dungeon/project.json` declares its resources with a glob —
`"src": "resources/dungeon/*.png"` — which bundles every PNG in that
directory into the app, whether or not the program ever loads it.
`examples/dungeon/src/main.mfb:main` calls `loadTiles` with exactly seven
tile-name roots (`"stone"`, `"stoneMissingTiles"`, `"stoneWall"`,
`"stoneWallArchway"`, `"stoneWallCorner"`, `"barrel"`, `"tableShort"`), and
`examples/dungeon/src/render.mfb:loadTiles` multiplies each by the four
compass facings (`compass()`/`facingSuffix`, `examples/dungeon/src/map.mfb`,
`examples/dungeon/src/render.mfb`) — 28 images loaded at runtime. The
`resources/dungeon/` directory holds 288 PNGs (72 distinct tile names ×4
facings), all added in one commit with no accompanying license or
attribution file anywhere in the repository.

**Correct behavior:** an example ships only the assets it uses, and any
third-party asset it ships carries a license/attribution record establishing
it may be redistributed. Today neither holds: 260 of 288 shipped tiles are
dead weight, and the 28 actually used (like the other 260) have no license
of record at all.

References:

- `examples/dungeon/project.json` — the `*.png` glob that bundles the whole
  directory.
- `git log --diff-filter=A --follow -- examples/dungeon/resources` — the
  tileset's only commit, with no license text in the message or an
  accompanying file.
- `LICENSE` (repo root) — covers `mfb` itself; nothing in it or beside the
  tileset addresses third-party asset redistribution.

## Failing Reproduction

Not a runtime failure — a measured packaging fact, run today at HEAD
(`7609b46fa`):

```
$ find examples/dungeon/resources -iname "*.png" | wc -l
     288
$ du -sh examples/dungeon/resources
5.3M	examples/dungeon/resources
$ du -sk examples/dungeon/resources/dungeon
5416K	examples/dungeon/resources/dungeon
$ grep -n "loadTiles(\[" examples/dungeon/src/main.mfb
107:  LET tiles AS Map OF String TO RES canvas::Image = loadTiles(["stone", "stoneMissingTiles", "stoneWall", "stoneWallArchway", "stoneWallCorner", "barrel", "tableShort"]) TRAP(e)
$ grep -n "FUNC loadTiles" -A 10 examples/dungeon/src/render.mfb
115:FUNC loadTiles(names AS List OF String) AS Map OF String TO RES canvas::Image
116:  MUT tiles AS Map OF String TO RES canvas::Image = Map OF String TO RES canvas::Image {}
117:  FOR EACH name IN names
118:    FOR EACH f IN compass()
119:      LET key AS String = name & "_" & facingSuffix(f)
120:      RES img AS canvas::Image = canvas::loadImage(os::appResourcePath("dungeon/" & key & ".png"))
121:      tiles = collections::set(tiles, key, img)
122:    NEXT
123:  NEXT
124:  RETURN tiles
125:END FUNC
$ ls examples/dungeon/resources/dungeon | sed 's/_[NSEW]\.png$//' | sort -u | wc -l
      72
$ mfb build -app examples/dungeon >/dev/null && du -sk examples/dungeon/build/dungeon.app
9076	examples/dungeon/build/dungeon.app
```

- Observed: 288 PNGs shipped (5.3 MB / 5416 KB on disk), 72 distinct tile
  names in `resources/dungeon/`, of which `main.mfb` names only 7 (×4
  facings = 28 files actually opened by `canvas::loadImage` at runtime, ~0.49
  MB of the 5.3 MB shipped — measured by summing the 28 used files'
  individual sizes). The built app bundle (`mfb build -app`, run today) is
  9076 KB (~8.9 MiB / ~9.3 MB) on disk.
- Expected: the shipped resource set matches what `loadTiles`'s call site
  actually names, and any third-party asset among them carries a license
  record establishing the repository may redistribute it.

Search for any accompanying license/attribution/credits file, run today —
none found:

```
$ find examples/dungeon -iname "*license*" -o -iname "*attrib*" -o -iname "*credit*" -o -iname "*readme*"
$ find /Users/justinzaun/Development/mfb -iname "LICENSE*" -maxdepth 2
/Users/justinzaun/Development/mfb/LICENSE
```

(`LICENSE` at the repo root covers `mfb`'s own source; it says nothing about
this tileset.)

## Root Cause

Two independent facts, cited to their exact source:

1. **Unused assets shipped.** `examples/dungeon/project.json`'s resources
   entry is a directory glob (`resources/dungeon/*.png` → `dungeon/`), not a
   list of the specific files `loadTiles` names. `main.mfb:107`'s
   `loadTiles([...])` call is the single source of truth for which tiles the
   program uses (7 names), and `render.mfb:loadTiles` (lines 115–125)
   multiplies each by `compass()`'s four facings — 28 files. The other 260
   files in `resources/dungeon/` (65 further tile names, ×4) are never
   named by any `canvas::loadImage` call anywhere in
   `examples/dungeon/src/*.mfb` (confirmed: `loadTiles` is the only call
   site, `grep -rn "canvas::loadImage" examples/dungeon/src`).
2. **No license of record.** `git log --diff-filter=A --follow --format='%H %ad %s' -- examples/dungeon/resources`
   shows one commit (`f97d4f19d`, 2026-09-21, "examples/dungeon: isometric
   dungeon tile set, four facings per tile") that adds all 288 files with a
   bare commit message — no license, source URL, or author credit anywhere
   in the commit, the working tree, or the repository. The tile-name
   vocabulary (`stoneWallDoorBars`, `stoneWallGateOpen`,
   `tableRoundItemsChairs`, `woodenSupportsBeam`, etc. — see the full list
   via `ls examples/dungeon/resources/dungeon | sed 's/_[NSEW]\.png$//' |
   sort -u`) resembles naming conventions used by several publicly-known
   CC0/asset-pack tile sets, but this is a **guess** — no web/network lookup
   was performed this session (forbidden by task scope) to confirm a source,
   and none is recorded anywhere in the repository to confirm or refute
   against.

## Goal

- `examples/dungeon` ships only the tile images `loadTiles`'s call site
  actually names (28 files), or the resources declaration is scoped to
  exactly that list, so a future edit to `main.mfb`'s tile-name list and the
  shipped set cannot silently diverge again.
- The tileset's origin and license are recorded in the repository (a
  `NOTICE`/`LICENSE`/`CREDITS` file beside the resources, or the unused
  assets are removed entirely and the used ones re-sourced from a
  confirmed-licensed set) — this is an **owner decision**, not something
  this bug can resolve unilaterally (see Open Decisions).

### Non-goals (must NOT change)

- The dungeon's visual appearance, tile art, or `loadTiles`'s call site
  (the 7 names it passes) — this bug is about what ships alongside the
  program, not what the program draws.
- `compass()`/`facingSuffix`'s four-facing scheme — unrelated to which tile
  *names* are used.

## Blast Radius

`grep -rln '"src": "resources/' examples/*/project.json examples/*/*/project.json`
(2026-09-25) — every example that bundles resources via a directory glob is
a candidate for the same "ships more than it uses" shape; checked each:

- `examples/dungeon/project.json` — confirmed above: 260 of 288 unused.
  Fixed by this bug.
- `examples/wind/project.json` (`resources/wind/*.bin`) — one file,
  `coastline.bin`, used by `land.mfb:loadCoastline` — glob matches exactly
  one used file. Unaffected.
- `examples/water/project.json`, `examples/ai_chat/project.json`,
  `examples/browser/*/project.json`, `examples/brogue/project.json`, and any
  other example with a `resources` block — not individually re-audited file-
  by-file this session; flagged here as the same *pattern* (a glob can ship
  more than is used) but **not observed to fail** for any of them — out of
  scope for this bug, which addresses the one directory measured to
  actually diverge (dungeon, 260 unused files, 4.8 MB).
- No other example in the repository was found with a license/attribution
  gap this session beyond dungeon's tileset — not exhaustively audited;
  flagged as a candidate follow-up if any other example is later found to
  bundle third-party binary assets without a license record.

## Fix Design

Two independent tracks, both gated on the Open Decision below:

1. **Unused assets.** Once the license question is settled, either (a)
   delete the 260 unused PNGs from `resources/dungeon/` and leave the glob
   as-is (simplest — the glob then matches exactly what's used), or (b) keep
   a larger shipped set (e.g. if the owner wants headroom for tiles a future
   change will use) but change `project.json`'s resources entry to an
   explicit file list or a narrower glob matching only the names
   `loadTiles`'s call site actually passes, so the two cannot silently
   diverge. Recommended: (a) — delete what is unused; add back explicitly
   when a future change actually uses more tiles, keeping the "ships only
   what it uses" invariant enforceable by the audit script (Validation Plan)
   rather than by discipline alone.
2. **License.** If the tileset's origin/license can be identified (owner
   has out-of-band knowledge, or a source is located), add a
   `NOTICE`/`CREDITS` file beside `resources/dungeon/` recording it. If it
   cannot be identified with confidence, the owner must decide whether to
   keep, replace, or remove the art — this bug cannot make that call.

Rejected alternative: leave the unused 260 files in place "in case a future
change needs them." Rejected because (a) it is exactly the shape this bug
measures as waste — 4.8 MB of a 5.3 MB directory, unreferenced by any code
in the repository — and (b) it does nothing to resolve the license gap,
which applies equally to the 28 used files.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add the resource-usage audit script (Validation Plan) and confirm it
      fails today for `examples/dungeon` (260 unreferenced files) and
      passes for `examples/wind` (0 unreferenced).
- [ ] Owner identifies (or rules out identifying) the tileset's source and
      license; record the outcome under Open Decisions.

Acceptance: the audit script fails for dungeon with the exact unreferenced-
file list; the owner has recorded a license decision (or "unknown, decide
disposition") here.
Commit: —

### Phase 2 — the fix

- [ ] Per the Open Decision: delete the unused files (or narrow the
      manifest) and add the license/attribution record, or remove/replace
      the art entirely if the owner decides the license cannot be
      established.
- [ ] `examples/dungeon/project.json`'s resources entry stays consistent
      with whatever is left (explicit list or a glob that now matches
      exactly the shipped set).

Acceptance: the Phase 1 audit script passes for `examples/dungeon`; the
license question has a recorded, actionable outcome (a NOTICE file, or a
recorded decision to remove/replace the art).
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `bash scripts/build-examples.sh` exits 0 (confirms `loadTiles`'s 28
      files are still all present and the app still builds and, per the
      example's own manual verification, still runs).
- [ ] Re-run `du -sh examples/dungeon/resources` and the app-bundle size
      check; confirm the reduction matches the deleted file count.

Acceptance: full examples build passes; resource directory and bundle size
are measurably smaller (or the owner's alternative disposition is in place).
Commit: —

## Validation Plan

- Regression test: a small audit script (recommended home: `tools/resource-usage-audit/`
  per the `scripts/`-vs-`tools/` split in `AGENTS.md`, since this is a
  reusable check applicable to every example, not a one-off) that, per
  example with a `resources` block, extracts every `os::appResourcePath("
  <dst>/<name>")`-style string literal reachable from the source (a
  best-effort static grep is sufficient — dungeon's names are all built from
  a literal prefix plus `compass()`'s four fixed suffixes, so the exact set
  is enumerable) and asserts every shipped file matches one, and every named
  file is shipped.
- Runtime proof: `du -sh`/`find | wc -l` before and after, shown above for
  "before"; `mfb build -app examples/dungeon` still succeeds and
  `loadTiles`'s 28 files are all still present after the deletion.
- Doc sync: none expected — this is example packaging, not a spec/man
  surface.
- Full suite: `bash scripts/build-examples.sh`.

## Open Decisions

- **Blocking: the tileset's license.** No license or attribution record
  exists anywhere in the repository or its history for
  `examples/dungeon/resources/dungeon/*.png` (checked: the adding commit's
  message, every directory under `examples/dungeon`, and the repo-root
  `LICENSE`). Options for the owner:
  - Identify the source and add a `NOTICE`/`CREDITS` file recording its
    license (preferred if the source can be confirmed).
  - Replace the art with a confirmed-licensed or originally-authored set.
  - Accept the risk and record that decision explicitly (not recommended —
    an example shipped from this repository should not carry unattributed
    third-party binary assets).
  This bug's Phase 2 cannot proceed on the unused-file deletion without this
  decision, since the same license question applies to the 28 files that
  would remain.
- Whether to delete the 260 unused files outright or narrow the resources
  manifest to an explicit list (Fix Design option (a) vs. (b)) —
  recommended: (a), delete outright, re-add explicitly if a future change
  needs more tiles.

## Summary

`examples/dungeon` ships 288 tile images for a program that loads 28 of
them, and neither the 260 unused files nor the 28 used ones carry any
license or attribution record. The unused-asset removal is mechanical once
the license question is answered; the license question itself is a
blocking owner decision this bug can only surface, not resolve, since no
web/network lookup was available this session to identify the tileset's
origin.
