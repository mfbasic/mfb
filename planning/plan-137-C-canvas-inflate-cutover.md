# plan-137-C: canvas PNG decoding on `compress::` inflate

Last updated: 2026-09-13
Effort: medium (1h–2h)
Depends on: plan-137-B (a strict, fast `compress::zlibDecode`). If plan-137-B is not complete,
this plan cannot start, full stop. Whole-feature prerequisites: plan-137-A §Prerequisites.

Canvas carries the tree's only other DEFLATE decoder — `__canvas_inflate` /
`__canvas_zlibInflate` in `src/codegen/builtins/canvas/helper_inflate.rs` — which is slow by
design and lenient (no Adler-32 check, DEC-57; accepts over-subscribed trees, DEC-58). This
letter points `__canvas_pngDecode` at `compress::zlibDecode` and deletes canvas's decoder.
Behavioural outcome: every PNG canvas decodes today still decodes to the same pixels; a PNG
whose zlib stream is malformed (bad Adler-32, over-subscribed tree) is now refused with
`ErrBadImageFile`; there is one inflate in the tree.

References:

- `src/codegen/builtins/canvas/helper_png.rs` — `__canvas_pngDecode` (the one call site:
  `LET raw AS List OF Byte = __canvas_zlibInflate(idat, expected)`), the 1032:1 ratio guard,
  the `len(raw) < expected` check.
- `src/codegen/builtins/canvas/func_load_image.rs` — `__canvas_loadImage`, which maps an empty
  decode to `FAIL error(77050023, "image file is malformed: " & path)`.
- `tests/canvas/rt_canvas_image_decode.rs` — the 15 decode tests (message-matching asserts,
  timeout bounds).
- `src/ir/lower.rs` — the late-pass chain; `color::augmented_project` and its plan-122-B comment
  is the precedent for a builtin whose injected source imports another builtin.
- `src/codegen/registry/mod.rs` `synthetic_files` — the per-package skips for late-pass packages.
- `.ai/canvas-threading.md` (decoding happens on the worker; nothing here touches the graphics
  thread), `.ai/resources-packages.md` (late-pass seam, companion size).

## Prerequisites

See plan-137-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-137-B complete | `ls planning/completed/plan-137-B-*` → one file | NOT MET (2026-09-13 re-run: `ls planning/completed | grep -c plan-137` → 0; blocked on plan-137-A's bug-621 row) |

## 1. Goal

- `__canvas_pngDecode` inflates IDAT through `compress::zlibDecode(idat, expected)`, converting
  any raised error into its existing `[]` failure result.
- `helper_inflate.rs` is deleted and every `__canvas_inflate*` / `__canvas_huff*` /
  `__canvas_bitAt` / `__canvas_bitsAt` / fixed-table helper with it; `__canvas_pow2` survives
  only if still used (`helper_png.rs` sample extraction uses it — re-grep).
- A program that `IMPORT canvas` and never `IMPORT compress` builds and decodes PNGs (the
  transitive import is injected by a late pass).
- All 15 existing decode tests pass unchanged; two new tests pin the new refusals.

### Non-goals (explicit constraints)

- No change to what canvas *accepts* beyond the two zlib-strictness refusals (and whatever
  further refusals plan-137-B's oracle established zlib makes — list them in Phase 1).
- No change to the PNG chunk walk, header caps (16384 per side, 16,777,216 pixels), filters,
  interlace, colour conversion, or error codes/messages.
- PNG **chunk CRC** verification (the other half of DEC-57) is **not** added here — it is PNG
  format work, not inflate; it stays an open audit item, recorded in Corrections if still open.
- Not a change to `canvas::loadImage`'s surface or man page beyond noting stricter refusal.
- The 1032:1 ratio pre-check stays (it refuses before inflating at all).

## 2. Current State

- One call site: `grep -rn "__canvas_zlibInflate(" src | grep -v helper_inflate.rs` → `helper_png.rs`
  (1 line) (plan-137-A §2).
- `__canvas_zlibInflate(data, limit)` returns `[]` on any failure and never raises; `limit` is
  the exact expected filtered size; `__canvas_pngDecode` then refuses `len(raw) < expected`.
- Injection today: canvas's companion is injected by the generic `registry::augment_project`;
  its `IMPORT color` is served by the `color` late pass placed after it in `src/ir/lower.rs`,
  and `synthetic_files` skips `color` to avoid double injection.
- Canvas fixtures are not in byte-identity (`.ai/testing-gates.md`: `grep -rln "IMPORT canvas\|IMPORT app" tests/byte-identity/` → none).

### Measured populations

| What | Count | Command |
|---|---|---|
| Canvas decode tests | 15 | `grep -c "#\[test\]" tests/canvas/rt_canvas_image_decode.rs` |
| Acceptance fixtures importing canvas (their `.ir`/`.ast` goldens are EXPECTED to shift) | UNMEASURED | Phase 1: `grep -rl "IMPORT canvas" tests --include=main.mfb \| wc -l` |
| `__canvas_pow2` users outside `helper_inflate.rs` | UNMEASURED | Phase 1: `grep -rn "__canvas_pow2" src/codegen/builtins/canvas \| grep -v helper_inflate.rs` |
| The TRAP idiom an injected helper uses to turn a raised error into a value | UNVERIFIED | Phase 1: `grep -rn "TRAP\|RECOVER" src/codegen/builtins --include='helper_*.rs' \| head` |
| Size delta of `IMPORT canvas` binaries | UNMEASURED | Phase 1 before / Phase 3 after, `.ai/resources-packages.md` size probe |

## 3. Design Overview

Two pieces: (1) **injection** — a `compress` late pass so canvas's `IMPORT compress` is served;
(2) **call-site swap** — trap `compress::zlibDecode` and keep canvas's `[]`-on-failure contract.

Correctness risk: the late pass's position. It must run after the generic pass (which injects
canvas's companion) and must not double-inject for a program that imports `compress` directly.
The `color` pass is the exact template. **Expected diffs:** `.ir`/`.ast` goldens of canvas-importing
acceptance fixtures (companion text changes). Unexpected: any `build.log` / run-output change,
any `.ncodesum` change (canvas has none — so any is a bug).

Rejected: keeping canvas's decoder (two bug surfaces); making canvas call a canvas-private copy
of compress's helpers (duplicates B's code and its strictness proofs).

## 4. Detailed Design

- `src/codegen/builtins/compress/mod.rs`: `pub(crate) fn augmented_project(...)` mirroring
  `color::augmented_project` (`registry::inject_late_pass`).
- `src/codegen/registry/mod.rs` `synthetic_files`: skip `compress` with a comment in the style of
  the `color` skip.
- `src/ir/lower.rs`: call `compress::augmented_project` after the generic pass and before
  `lower_augmented_project`, with a comment naming canvas as the transitive importer.
- `src/codegen/builtins/canvas/mod.rs`: `add_imports` gains `"compress"`; drop the
  `helper_inflate` registration and `mod`.
- `helper_png.rs`: replace the call with the Phase 1 TRAP idiom around
  `compress::zlibDecode(idat, expected)`, yielding `[]` on any trapped error. `ErrTooLarge` maps to
  `[]` exactly as the old `limit` refusal did.

## Compatibility / Format Impact

`canvas::loadImage` refuses PNGs with malformed zlib data that it previously decoded (bad
Adler-32; over-subscribed Huffman trees; any further class plan-137-B established). Same error
(`ErrBadImageFile`), same message. Everything else unchanged.

## Phases

> **NOTE — keep the checkboxes current as you go** (see plan-137-A). **An unticked box means NOT DONE.**

### Phase 1 — census

- [ ] Fill every UNMEASURED/UNVERIFIED row in §2; list plan-137-B's refusal classes that canvas
      did not previously enforce (Non-goals).
- [ ] Record the pre-change size of a minimal `IMPORT canvas` program and the decode time of the
      plan-137-B Phase 1 PNG.

Acceptance: §2 has no UNMEASURED rows.
  Check: the commands in §2 (est. 10 min).
Commit: —

### Phase 2 — failing tests first

- [ ] `tests/canvas/rt_canvas_image_decode.rs`: `a_png_whose_zlib_adler32_is_wrong_is_refused` and
      `a_png_whose_huffman_tree_is_oversubscribed_is_refused` (build the PNGs with the test file's
      own `crc32`/`adler32`/`zlib_stored` helpers plus a hand-built dynamic block); assert
      `ErrBadImageFile` with the `"malformed"` message. Confirm both **fail** today (canvas decodes them).

Acceptance: the two new tests fail against HEAD for the documented reason.
  Check: `cargo test --test rt_canvas_image_decode -- adler32_is_wrong oversubscribed` → 2 failed (est. 6 min).
Commit: —

### Phase 3 — the cutover

- [ ] Late pass + skip + `lower.rs` call (§4).
- [ ] Canvas imports `compress`; call site swapped; `helper_inflate.rs` deleted; unused canvas
      helpers deleted (grep each name before deleting).
- [ ] Sync the expected `.ir`/`.ast` goldens of canvas-importing fixtures with a targeted glob;
      `git diff --stat` shows only `.ir`/`.ast` under those fixtures.

Acceptance: all 17 decode tests pass; the late pass serves a canvas-only program.
  Check: `cargo test --test rt_canvas_image_decode` → 17 passed;
  `scripts/test-accept.sh target/release/mfb /tmp/p137c '<canvas fixture glob>'` → 0 mismatches after sync (est. 15 min).
Commit: —

### Phase 4 — record

- [ ] Post-change size and decode time (Phase 1 program and PNG) recorded in Corrections.
- [ ] `canvas::loadImage` man `desc` mentions that malformed compressed data is refused (no
      internals). Spec: `grep -rln "helper_inflate\|__canvas_inflate\|__canvas_zlibInflate" src/docs`
      → none on 2026-09-13, so no citation is expected to dangle; re-run it, and if the canvas spec
      page describes PNG decompression in prose, point it at `mfb spec stdlib compress`.
- [ ] `planning/completed/audit-3-decoders.md` DEC-58 and the Adler half of DEC-57: add a dated
      resolution line citing this letter's commit (the PNG-CRC half stays open).

Acceptance: docs cite no deleted symbol.
  Check: `grep -rn "helper_inflate\|__canvas_inflate\|__canvas_zlibInflate" src tests` → none;
  `cargo test -p mfb --bins citations_resolve` → pass (est. 5 min).
Commit: —

## Validation Plan

- Tests: `rt_canvas_image_decode` (15 existing + 2 new).
- Coverage check: the new tests execute the `compress` path through canvas (they would fail on
  the old decoder — Phase 2 proves they are in the path).
- Runtime proof: the headless decode tests are the runtime proof on macOS; on box 2223, run
  `scripts/test-canvas-vulkan.sh` only if it exercises `loadImage` (check its body first; if not,
  record that and rely on B's 2223 decoder proof).
- Doc sync: canvas man `desc`, canvas spec page, audit-3 resolution lines.
- Final gate: plan-137-E.

## Open Decisions

- Canvas strictness — carried from plan-137-A; recommended: accept the stricter refusals.

## Corrections

<Filled in during execution.>

## Summary

Small code change, real seam risk: the transitive-import late pass. The decoder's correctness was
proven in B; this letter proves canvas still decodes everything it should and nothing it shouldn't.
