# bug-718: `examples/bugs/{tui,canvas}` names a swat-the-bug game like a bug-repro folder, and paces frames with `io::pollInput` instead of `canvas::waitFrame`

Last updated: 2026-09-25
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Other

Status: Open
Regression Test: none yet — Phase 1 adds a naming-convention grep guard (see Phases)

`examples/bugs/tui` and `examples/bugs/canvas` are not bug reproductions. They
are two editions (terminal and canvas) of a mouse-driven "swat the bug"
arcade game — plants sprout, a bug appears among them, clicking the bug
scores, missing it or hitting a plant costs points, and score gates ten
difficulty levels. Confirmed from the commits that introduced them:

```
$ git log --oneline --all -- examples/bugs | grep -v "t3 checkpoint"
0a10c5715 examples: load fonts by name with canvas::loadSystemFont
2fcfda48e examples/bugs: a canvas edition of the bugs game, drawn with shapes
bc66cc75e examples/bugs: a mouse-driven swat-the-bug game for term::
```

`git show bc66cc75e -s --format=fuller` (subject: "examples/bugs: a
mouse-driven swat-the-bug game for term::") and `git show 2fcfda48e -s
--format=fuller` (subject: "examples/bugs: a canvas edition of the bugs game,
drawn with shapes") both describe game mechanics end to end (plant sprouting,
bug lifetimes, scoring, ten themed levels, pause/restart keys) with no mention
of reproducing any defect. The directory and both `project.json` files are
named "bugs" purely because the game's own title is "BUGS" (rendered as a
literal `[BUGS]` banner — see `examples/bugs/canvas/src/main.mfb:640`,
`LET titleText AS String = "BUGS"`), not because the content belongs under a
bug-repro convention. This collides with the repo's actual `bugs/bug-NN-*.md`
tracking convention and with any reader's expectation that `examples/bugs`
holds minimal repros.

**Correct behavior.** The example directory and project name should reflect
what the example is — a game — e.g. `examples/swat` with project names like
`swat-tui` / `swat-canvas` (naming choice left to Fix Design), and the canvas
edition's main loop should pace frames with `canvas::waitFrame`, the primitive
its sibling canvas examples (`water`, `wind`) already use for this purpose,
instead of driving redraw timing off `io::pollInput`'s timeout.

References:

- `bugs/bug-699-special-keys-arrive-unstandardized-per-surface.md` — in-flight
  plan to remove `io::readChar`/`io::readByte`/`io::pollInput` repo-wide; its
  Phase-migration table lists `bugs/canvas` and `bugs/tui` as two of the 13
  examples to migrate (see Blast Radius below for the exact citation).
- `planning/completed/plan-155-C-canvas-wait-frame.md` — introduced
  `canvas::waitFrame` as the canvas frame-pacing primitive.
- `examples/water/src/main.mfb`, `examples/wind/src/main.mfb` — sibling canvas
  examples that already use `canvas::waitFrame` for pacing (cited below).

## Failing Reproduction

This is a naming/consistency defect, not a crash, so "reproduction" is showing
the misleading state exists today and still builds cleanly (i.e. nothing here
is broken by a build failure — the fix is a rename + a pacing-primitive swap,
not a bug fix to broken code).

**1. The directory and both projects are named "bugs" with no game
indicator:**

```
$ ls -la examples/bugs/
drwxr-xr-x@  4 justinzaun  staff  128 Sep 21 14:15 .
drwxr-xr-x@ 22 justinzaun  staff  704 Sep 24 22:57 ..
drwxr-xr-x@  5 justinzaun  staff  160 Sep 21 14:19 canvas
drwxr-xr-x@  5 justinzaun  staff  160 Sep 21 09:38 tui

$ cat examples/bugs/tui/project.json | grep name
  "name": "bugs",

$ cat examples/bugs/canvas/project.json | grep name
  "name": "bugs",
```

Contrast with every sibling example, whose `project.json` name matches its
own directory:

```
$ grep '"name"' examples/water/project.json examples/wind/project.json examples/snake/project.json
examples/water/project.json:  "name": "water",
examples/wind/project.json:  "name": "wind",
examples/snake/project.json:  "name": "snake",
```

**2. Repo-wide blast radius of the literal string `examples/bugs`:**

```
$ grep -rln "examples/bugs" --include="*.md" --include="*.sh" --include="*.json" --include="*.mfb" --include="*.rs" .
bugs/completed/bug-669-mouse-decoder-holds-lone-escape-and-drops-owed-bytes.md
bugs/completed/bug-670-metal-text-run-draws-every-glyph-with-the-last-glyphs-bitmap.md
planning/completed/plan-155-C-canvas-wait-frame.md
examples/bugs/tui/src/main.mfb
examples/bugs/canvas/src/main.mfb
```

`scripts/build-examples.sh` does **not** appear in this list — it discovers
example projects dynamically (`find "$EXAMPLES_DIR" -name project.json`, then
derives the display name from the path segment under `examples/`), so it
requires no edit for a rename. This contradicts the assumption that it would
need updating; verified by reading the discovery loop directly
(`scripts/build-examples.sh:101-131`).

**3. `bugs/bug-699-special-keys-arrive-unstandardized-per-surface.md` does
reference these two examples, but not via the literal string
`examples/bugs`** — it uses the shorthand `bugs/canvas` / `bugs/tui` in its
migration table, which the grep above (anchored on the literal path) misses:

```
$ grep -n "bugs/canvas\|bugs/tui" bugs/bug-699-special-keys-arrive-unstandardized-per-surface.md
340:  | `bugs/canvas` | `readChar`, `pollInput` | yes |
341:  | `bugs/tui` | `readChar`, `pollInput` | yes |
```

Both rows mark them `readChar`, `pollInput`, migrate: yes — confirming
independently that both editions use `pollInput` and are already slated for
migration off it by bug-699's Phase 6 (a separate, in-flight effort this bug
does not duplicate — see Non-goals).

**4. Both editions build cleanly today (current-state baseline, before any
rename):**

```
$ target/release/mfb build examples/bugs/tui
Building bugs (executable) for macos-aarch64
Wrote executable to examples/bugs/tui/build/bugs.out

$ target/release/mfb build examples/bugs/canvas
Building bugs (executable) for macos-aarch64
Wrote executable to examples/bugs/canvas/build/bugs.app
```

Note the output artifacts are themselves named `bugs.out` / `bugs.app` from
the `"name": "bugs"` project field — reinforcing the misleading identity all
the way to the built binary. (`build/` is gitignored —
`git check-ignore -v examples/bugs/tui/build/bugs.out
examples/bugs/canvas/build/bugs.app` confirms both paths match `.gitignore:24`
— so these builds left the tree clean.)

**5. `io::pollInput(16)` frame-clock, confirmed and re-derived:**

```
$ grep -n "pollInput" examples/bugs/canvas/src/main.mfb examples/bugs/tui/src/main.mfb
examples/bugs/canvas/src/main.mfb:666:  LET r AS Boolean = io::pollInput(ms) TRAP(e)
examples/bugs/tui/src/main.mfb:262:  LET r AS Boolean = io::pollInput(ms) TRAP(e)
```

The canvas edition's main loop calls `ready(frameDelayMs())` once per frame
(`examples/bugs/canvas/src/main.mfb:828`, `IF ready(frameDelayMs()) THEN`),
where `ready` wraps `io::pollInput(ms)` (`:665-669`) and `frameDelayMs`
returns the constant 16 (`examples/bugs/canvas/src/main.mfb:89-91`,
`FUNC frameDelayMs AS Integer / RETURN 16`). Re-deriving the frame rate this
implies: a 16 ms poll interval is 1000 ms / 16 ms = 62.5 cycles/second, i.e.
**~60 fps** — not "~833" of anything; 1/62.5 s = 16 ms = 16 000
microseconds, so an "~833" figure would be a miscalculation (there is no
factor in this code that produces 833). The tui edition uses the identical
constant and pattern (`examples/bugs/tui/src/main.mfb:114` `frameDelayMs`,
`:428` `IF ready(frameDelayMs()) THEN`), but tui has no `canvas::waitFrame`
alternative since it targets `term::`, not `canvas::` — the inconsistency
this bug fixes is canvas-only.

**Sibling canvas examples use `canvas::waitFrame` instead:**

```
$ grep -n "waitFrame" examples/water/src/main.mfb examples/wind/src/main.mfb
examples/water/src/main.mfb:239: The loop is paced by the display: canvas::waitFrame ends each pass, so it runs
examples/water/src/main.mfb:344:    canvas::waitFrame()
examples/wind/src/main.mfb:270: core to compute frames nobody sees. canvas::waitFrame ends each pass: it returns
examples/wind/src/main.mfb:486:    canvas::waitFrame()
```

```
$ grep -rn "waitFrame" examples/bugs/
(no output)
```

`examples/bugs/canvas` never calls `canvas::waitFrame`; it is the only
`canvas::` example in the repo that paces its loop with a fixed
`io::pollInput` timeout instead of the display-synced primitive its siblings
use.

## Root Cause

Both examples were authored (`git show bc66cc75e`, `git show 2fcfda48e`) as a
terminal game and its canvas port, and named after the game's on-screen title
("BUGS") rather than the repo's example-naming convention (directory name ==
project name == what it does, as every sibling in `examples/` follows —
verified above for `water`/`wind`/`snake`). Concretely:

- `examples/bugs/tui/project.json:2` — `"name": "bugs"`.
- `examples/bugs/canvas/project.json:2` — `"name": "bugs"`.
- `examples/bugs/canvas/src/main.mfb:665-669` (`FUNC ready`) wraps
  `io::pollInput(ms)`; `:89-91` (`FUNC frameDelayMs`) fixes `ms` at 16; `:828`
  (`IF ready(frameDelayMs()) THEN`) uses that as the once-per-loop frame gate
  inside `FUNC main` (`:704`, loop `:737-849`). The tui edition
  (`examples/bugs/tui/src/main.mfb:114`, `:428`) mirrors the same pattern by
  necessity (`term::` has no `waitFrame` equivalent), but the canvas edition
  had `canvas::waitFrame` available (as `water`/`wind` demonstrate) and did
  not use it — likely because it was written as a direct, mechanical port of
  the tui edition's input-polling loop rather than adapted to the canvas
  surface's own pacing idiom.

## Goal

- `examples/bugs/` is renamed to a name that describes the game, not a bug
  repro (e.g. `examples/swat`), and each sub-project's `project.json` `name`
  is distinct from and matches its own role (e.g. `swat-tui`, `swat-canvas`),
  per the convention every other example in `examples/` already follows.
- `examples/bugs/canvas` (post-rename, the canvas edition) paces its main
  loop with `canvas::waitFrame` instead of `io::pollInput(frameDelayMs())`,
  matching `water` and `wind`.
- Every reference to the old path/names found in the Blast Radius section is
  updated in the same change, including bug-699's migration table and any
  script that assumes the old path.

### Non-goals (must NOT change)

- Game mechanics, difficulty curve, scoring, level themes, visuals, timing
  constants (`frameDelayMs`'s *value* of 16 stays — only its *use site*
  changes from an `io::pollInput` argument to being superseded by
  `canvas::waitFrame`'s own vsync-driven cadence), or input handling beyond
  the pacing primitive itself.
- bug-699's own scope (removing `io::readChar`/`io::readByte`/`io::pollInput`
  repo-wide). That bug already lists both editions for migration; this bug
  only renames the directory/projects and swaps the canvas edition's
  frame-pacing call. Do not use this bug to preempt or duplicate bug-699's
  Phase 6 removal — the tui edition keeps `io::pollInput` since `term::` has
  no substitute, and this bug does not touch tui's input path at all.
- The `.gitignore` `build/` rule or the artifact naming scheme in
  `scripts/build-examples.sh` — the rename requires no change there, since
  discovery is dynamic (confirmed above).

## Blast Radius

- `examples/bugs/` (directory) — renamed by this bug.
- `examples/bugs/tui/project.json:2` (`"name": "bugs"`) — renamed by this bug.
- `examples/bugs/canvas/project.json:2` (`"name": "bugs"`) — renamed by this
  bug.
- `examples/bugs/canvas/src/main.mfb:640` (`LET titleText AS String = "BUGS"`)
  — the on-screen game title string; left alone (Non-goals: visuals unchanged)
  even though it shares the word "bugs" — it is the game's title, not a path.
- `examples/bugs/canvas/src/main.mfb:665-669,828` (`ready`/`frameDelayMs`
  wiring `io::pollInput`) — fixed by this bug (swapped to
  `canvas::waitFrame`).
- `examples/bugs/tui/src/main.mfb:262-266,428` (same `pollInput` pattern) —
  NOT changed by the pacing fix (no `canvas::waitFrame` equivalent for
  `term::`); only its path (`examples/bugs/tui` -> new path) changes.
- `bugs/completed/bug-669-mouse-decoder-holds-lone-escape-and-drops-owed-bytes.md`,
  `bugs/completed/bug-670-metal-text-run-draws-every-glyph-with-the-last-glyphs-bitmap.md`
  — both are `bugs/completed/*` docs whose text cites `examples/bugs` as a
  historical repro location; verify at Phase 1 whether each citation refers to
  today's game (needs updating to the new path) or to some other, unrelated
  use of the same literal substring (out of scope) — not distinguished by this
  audit, left for Phase 1's line-by-line check (see below).
- `planning/completed/plan-155-C-canvas-wait-frame.md` — cites `examples/bugs`
  (context TBD at Phase 1: whether it lists `bugs/canvas` as a still-pending
  `waitFrame` adopter, which would make it doubly relevant to this bug).
- `bugs/bug-699-special-keys-arrive-unstandardized-per-surface.md:340-341` —
  migration table rows `bugs/canvas` / `bugs/tui`, `readChar`/`pollInput`,
  `migrate: yes` — must be updated to the new path/names in the same change
  that performs the rename, so the in-flight bug-699 migration doesn't point
  at a path that no longer exists.
- `scripts/build-examples.sh` — audited, **not** affected (dynamic discovery,
  `scripts/build-examples.sh:101-131`); listed here to record that it was
  checked, not skipped.
- `.gitignore:24` (`build/`) — a generic pattern, not example-specific; not
  affected.

## Fix Design

Rename `examples/bugs` to `examples/swat` (or another game-appropriate name —
left as an open decision, see below) via `git mv` to preserve history, update
both `project.json` `name` fields to reflect the sub-edition (e.g.
`swat-tui`, `swat-canvas`) rather than both sharing one generic name, update
the two `bugs/completed/*.md` citations and `planning/completed/plan-155-C-canvas-wait-frame.md`
if their content still points at the old path (verify each first — completed
docs describe history and may be intentionally citing the old path/commit,
in which case they are left alone as a historical record and only touched if
they assert the *current* path), and update
`bugs/bug-699-special-keys-arrive-unstandardized-per-surface.md`'s migration
table rows to the new path. Separately, replace the canvas edition's
`ready(frameDelayMs())` frame gate with `canvas::waitFrame()`, following the
`water`/`wind` pattern exactly (call it once per loop pass after
`canvas::present`, drop the now-unused `frameDelayMs`/`ready`-for-pacing call
site — `ready` itself stays, since it is also used for the escape-sequence
drain at `:679-683`).

Rejected alternative: leaving `examples/bugs` where it is and only adding a
README disclaiming it as a game. Rejected because the misleading signal is in
the path and the project name themselves (visible in `ls examples/`,
`grep -r "kind.*executable" examples/*/project.json`, and the built artifact
name), which a README next to it does not fix for anyone scanning the
directory tree or the build manifest.

## Phases

### Phase 1 — audit + failing check (no behavior change)

- [ ] Add a repo consistency check (e.g. a `scripts/` script or a
      `cargo test` under `tests/`) asserting every `examples/<dir>/project.json`
      `name` matches `<dir>` (or a documented multi-project exception list for
      cases like `browser/{dom,fetch,display,app}` that intentionally differ).
      Confirm it fails today specifically on `examples/bugs/tui` and
      `examples/bugs/canvas` (`name: "bugs"` != dir `tui`/`canvas`, and dir
      `bugs` itself carries no game signal).
- [ ] Read each hit in `bugs/completed/bug-669-*.md`,
      `bugs/completed/bug-670-*.md`, and
      `planning/completed/plan-155-C-canvas-wait-frame.md` to classify: cites
      the current game (needs path update) vs. an unrelated/historical
      reference (leave as-is). Record the verdict per file in this section.

Acceptance: the new check fails today, naming both bogus projects; the three
completed-doc citations each have a recorded verdict.
Commit: —

### Phase 2 — the rename and pacing fix

- [ ] `git mv examples/bugs examples/swat` (or the chosen name), preserving
      history.
- [ ] Update `examples/swat/tui/project.json` and
      `examples/swat/canvas/project.json` `name` fields to distinct,
      descriptive names.
- [ ] Replace `examples/swat/canvas/src/main.mfb`'s `ready(frameDelayMs())`
      main-loop gate with `canvas::waitFrame()`, matching
      `examples/water/src/main.mfb:344` / `examples/wind/src/main.mfb:486`.
- [ ] Update `bugs/bug-699-special-keys-arrive-unstandardized-per-surface.md:340-341`
      migration-table rows to the new path/names.
- [ ] Update any completed-doc citation from Phase 1 that was verdicted
      "needs path update."

Acceptance: the Phase 1 consistency check passes; the canvas edition builds
and runs with `canvas::waitFrame` pacing; bug-699's table cites the new path;
no game mechanic/visual/timing changed (diff review against Non-goals).
Commit: —

### Phase 3 — validation

- [ ] `target/release/mfb build examples/swat/tui` and
      `target/release/mfb build examples/swat/canvas` — both succeed.
- [ ] `scripts/build-examples.sh` — full run succeeds (confirms the dynamic
      discovery picks up the new path with no script edit, per the Phase-1
      audit above).
- [ ] Manual play-through of both editions (mouse clicks, pause/resume/quit,
      at least one level-up) to confirm mechanics are unchanged.

Acceptance: both builds succeed; `scripts/build-examples.sh` exits 0; a
play-through confirms no mechanic/visual regression.
Commit: —

## Validation Plan

- Regression test: the Phase 1 naming-consistency check, passing post-rename.
- Runtime proof: Phase 3's builds plus a manual play-through of both editions.
- Doc sync: `bugs/bug-699-special-keys-arrive-unstandardized-per-surface.md`
  migration table; the two `bugs/completed/*.md` docs and
  `planning/completed/plan-155-C-canvas-wait-frame.md` if Phase 1 verdicts
  them as needing it.
- Full suite: `scripts/build-examples.sh` (all targets); any project-wide
  `cargo test` gate that covers example manifests.

## Open Decisions

- Final name: `examples/swat` (recommended, matches the game's core verb) vs.
  `examples/bug-swat` vs. `examples/swatter` — pick at Phase 2 start; does not
  block the audit in Phase 1.
- Per-edition project names: `swat-tui`/`swat-canvas` (recommended, mirrors
  the directory) vs. `tui`/`canvas` alone (ambiguous against other examples
  that use bare surface names) — pick at Phase 2 start.

## Summary

No code is broken today — both editions build and run correctly (verified:
`target/release/mfb build examples/bugs/tui`,
`target/release/mfb build examples/bugs/canvas`, both exit 0). The defect is
identity: the directory, both `project.json` names, and the built artifacts
(`bugs.out`, `bugs.app`) all say "bugs" while the content is a game, colliding
with the repo's own `bugs/bug-NN-*.md` convention; and the canvas edition
paces frames with `io::pollInput(16)` (~60 fps, re-derived from 1000 ms / 16
ms = 62.5) instead of the `canvas::waitFrame` primitive its `water`/`wind`
siblings use. The engineering risk is entirely in the blast-radius sweep
(catching every stale reference, especially the in-flight bug-699 migration
table) — the rename and the one-line pacing swap are mechanical.
