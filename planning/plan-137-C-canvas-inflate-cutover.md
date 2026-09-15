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
| plan-137-B complete | `ls planning/completed/plan-137-B-*` → one file | MET (2026-09-14 re-run: `planning/completed/plan-137-B-compress-inflate-decoders.md`, archived in `afd7eaca5`/`a793cf281`; plan-137-A §Prerequisites re-run the same day — bug-621 in `bugs/completed/`, `cargo build --release --bin mfb` → `Finished … 47.33s`, Python zlib `1.2.12` / Node `1.3.1-470d3a2`, `flate2` `1.1.9`, `compress` present) |

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
  **Listed 2026-09-14** from `helper_inflate.rs`. Canvas's decoder already refuses: input under 6 bytes,
  `CM ≠ 8`, a bad `FCHECK`, `FDICT`, stored `LEN ≠ ~NLEN`, a code-length repeat with no previous length, a repeat
  overrunning `HLIT + HDIST` (checked after the loop), a literal/length symbol above 285, a distance symbol above 29,
  a distance before the start of output, and output past the limit. The classes it gains through
  `compress::zlibDecode` (plan-137-B §1, `tools/oracles/compress/probe.sh`): a wrong Adler-32 (DEC-57; canvas
  never reads the trailer), an over-subscribed code set (DEC-58), an incomplete literal/length or distance set
  other than a single 1-bit code, a literal/length set with no end-of-block code, and `CINFO > 7` (canvas's
  comment names the 32 KiB window, but only `CM` is tested).
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
| Acceptance fixtures importing canvas (their `.ir`/`.ast` goldens are EXPECTED to shift) | 4 (2026-09-14) | Phase 1: `grep -rl "IMPORT canvas" tests --include=main.mfb \| wc -l` |
| `__canvas_pow2` users outside `helper_inflate.rs` | 2 (2026-09-14): `helper_png.rs:163` and `:170` (sample extraction). Its only definition is `helper_inflate.rs:47` (`FUNC __canvas_pow2`), so the definition must move to a surviving canvas helper before `helper_inflate.rs` is deleted | Phase 1: `grep -rn "__canvas_pow2" src/codegen/builtins/canvas \| grep -v helper_inflate.rs` |
| The TRAP idiom an injected helper uses to turn a raised error into a value | Verified (2026-09-14): `LET x AS T = call(...) TRAP(e)` with a handler ending in `RECOVER value` (`net/helper_decode_query_component.rs:16–17`, `RECOVER s`) or `RETURN value` (`strings/helper_scalar_seam.rs:64–65`, `RETURN ""`) | Phase 1: `grep -rn "TRAP\|RECOVER" src/codegen/builtins --include='helper_*.rs' \| head` |
| Size delta of `IMPORT canvas` binaries | Before (2026-09-14, macos-aarch64 `--app`): **1,904,108 B** for a program that only imports `canvas` and **1,904,108 B** for one that also calls `canvas::loadImage` — canvas's PNG and inflate helpers are `RegistryHelper::always`, so every canvas program carries them. After (2026-09-14, the Phase 3 compiler): **1,953,644 B** for both probes (`stat -f %z` of each bundle's `Contents/MacOS` binary), **+49,536 B**. The generic pass still injects canvas's `always` companion into every canvas program, and that companion calls `compress::zlibDecode`, so every canvas program carries `compress`'s decoder, as it carried canvas's own before | Phase 1 before / Phase 3 after, `.ai/resources-packages.md` size probe |

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
  **Corrected before this letter started (2026-09-14, by reading the code; confirm in Phase 1):** a plain
  mirror injects nothing for `compress`. `late_pass_file` injects `pkg.get_mfb()`, which renders only
  `always_helper_bodies` — whose filter drops every `HelperGate::WhenUsed` helper ("Gated helpers … are
  injected as separate files by `Registry::augment_project` and are excluded here") — plus `Body::Mfb`
  member bodies. `color`'s late pass works because its 8 helpers are all `RegistryHelper::always`; every
  `compress` helper is `WhenUsed` and every `compress` member is `Body::Rewrite`, so `get_mfb()` renders
  only the `IMPORT` lines and the late pass adds no decoder source. Canvas's `compress::zlibDecode` call
  would then reference an undefined `__compress_zlibDecode`. The late pass must select `compress`'s gated
  helpers against the augmented project the way `synthetic_files` does (their `WhenUsed` gates open on the
  callee names canvas's injected companion contributes); the choice between that and any other shape is
  made in Phase 1 by the probe below, not assumed.
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

- [x] Fill every UNMEASURED/UNVERIFIED row in §2; list plan-137-B's refusal classes that canvas
      did not previously enforce (Non-goals).
- [x] (Added 2026-09-14) Probe the late-pass gating defect recorded in §4: a scratch late pass mirroring
      `color::augmented_project` for `compress`, a `--app` program that imports only `canvas` and loads a PNG,
      and the resulting build diagnostic (expected: an undefined `__compress_zlibDecode`). Then choose and
      record the injection shape that makes the gated helpers ride in, with the probe that shows it building.
      (2026-09-14, scratch worktree at `a793cf281`:
      - The mirror is `compress::augmented_project` → `inject_late_pass(ast, "compress", …)`, called after `color` in
        `resolver::augment_project`, plus the `synthetic_files` companion skip and canvas `add_imports` gaining
        `"compress"`. The call site is `LET raw … = compress::zlibDecode(idat, expected) TRAP(err) RETURN [] END TRAP`.
      - `mfb build --app /tmp/p137latepass` with that mirror → `error: NIR call target '#compress_zlibDecode' does not
        resolve`: the rewrite target, a `WhenUsed` helper, never rode in. The defect is confirmed.
      - Wiring the pass only into `ir::lower`'s chain gives the same diagnostic, because that chain is `#[cfg(test)]`.
        The build path is `resolver::augment_project`.
      - **Chosen shape:** `late_pass_file` becomes `late_pass_files`. It returns the `get_mfb` companion, then each of the
        package's `HelperGate::WhenUsed` helpers whose gate the late pass's own view opens, parsed with
        `parse_source_builtin` exactly as `synthetic_files` does. `synthetic_files` skips `compress` in both its companion
        loop and its gated-helper loop.
      - The generalisation changes nothing for the other late passes: `grep -rn "HelperGate::"` over
        `builtins/{color,http,net,encoding}` → none, and `builtins/collections` → one `RegistryHelper::always`.
      - With that shape, `mfb build --app /tmp/p137latepass` → `Wrote executable to ./build/latepass.app`.
      - The compress helpers' `collections::` calls (`append`/`get`/`getOr`/`mid`/`set`) are all native members, so the
        parse-time `collections` pass, which a canvas-only program never triggers, is not needed.)
- [x] Record the pre-change size of a minimal `IMPORT canvas` program and the decode time of the
      plan-137-B Phase 1 PNG.
      (Size: 1,904,108 B for both probes, §2. Decode time: 43,691.6 ms for the 4096×4096 PNG's 67,112,960 B through
      `canvas::loadImage` (plan-137-B §2, 2026-09-14). Still the pre-change figure: `git log -1 -- src/codegen/builtins/canvas`
      → `986ab96b8 2026-09-13`, before that measurement, and nothing under `canvas/` has changed since.)

Acceptance: §2 has no UNMEASURED rows.
  Check: the commands in §2 (est. 10 min).
Commit: —

### Phase 2 — failing tests first

- [x] `tests/canvas/rt_canvas_image_decode.rs`: `a_png_whose_zlib_adler32_is_wrong_is_refused` and
      `a_png_whose_huffman_tree_is_oversubscribed_is_refused` (build the PNGs with the test file's
      own `crc32`/`adler32`/`zlib_stored` helpers plus a hand-built dynamic block); assert
      `ErrBadImageFile` with the `"malformed"` message. Confirm both **fail** today (canvas decodes them).
      (2026-09-14, on the post-merge tree `0d3229ee9`: `cargo test --test rt_canvas_image_decode -- adler32_is_wrong
      oversubscribed` → `0 passed; 2 failed`, panics `a PNG with a wrong zlib Adler-32 decoded as a 7x5 image` and
      `a PNG with an over-subscribed Huffman code decoded as a 1x1 image`, the documented reason. The over-subscribed
      IDAT is a fixed hex stream, `OVERSUBSCRIBED_IDAT`, rather than one assembled by a builder: one dynamic block whose
      literal/length lengths over-subscribe the tree. Python `zlib.decompress` refuses it, and a transcription of
      canvas's bit walker decodes it to `[0, 0]`.)

Acceptance: the two new tests fail against HEAD for the documented reason.
  Check: `cargo test --test rt_canvas_image_decode -- adler32_is_wrong oversubscribed` → 2 failed (est. 6 min).
Commit: b006a877b

### Phase 3 — the cutover

- [x] Late pass + skip + `lower.rs` call (§4).
      (Shipped without the `synthetic_files` skip; see Corrections. `registry::late_pass_files` returns the companion plus the
      opened `WhenUsed` helpers, and `inject_late_pass`/`_hir` skip a path already present. `compress::augmented_project` /
      `augmented_hir_project` run after `color` in `resolver::augment_project`, `resolver::augment_hir_project` and
      `ir::lower`'s test chain. Checks:
      - `mfb build --app /tmp/p137latepass` (imports only `canvas`) → `Wrote executable`.
      - `bash scripts/artifact-gate.sh target/release/mfb compress` → `7 golden(s) checked, 0 diff(s)`.
      - New unit tests `a_canvas_program_gets_the_zlib_decoder_without_importing_compress`,
        `a_program_importing_compress_and_canvas_gets_each_helper_once` and
        `a_compress_program_that_only_checksums_carries_no_decoder` in `compress/mod.rs`
        → `cargo test --bin mfb codegen::builtins::compress::tests` → `ok. 6 passed; 0 failed`.)
- [x] Canvas imports `compress`; call site swapped; `helper_inflate.rs` deleted; unused canvas
      helpers deleted (grep each name before deleting).
      (`add_imports` gains `"compress"`. The call is `compress::zlibDecode(idat, expected) TRAP(err)` / `RETURN []` /
      `END TRAP`, and `len(raw) < expected` stays. Before deleting, each of the 17 `FUNC`s in `helper_inflate.rs` was
      grepped with `grep -rnw <name> src tests`, excluding the file itself:
      - `__canvas_pow2` is used twice, at `helper_png.rs:163`/`:170`; its definition moved into `PNG_SAMPLES` beside those uses.
      - `__canvas_zlibInflate` is used once, at the swapped call site.
      - `__canvas_inflate` and `__canvas_huffDecode` each appear once, in doc comments only, both reworded.
      - The other 13 are unused.
      The only other citation is `func_load_image.rs`'s doc comment, repointed. `planning/todo.md`'s
      "two inflate implementations" design question is marked resolved.)
- [x] ~~Sync the expected `.ir`/`.ast` goldens of canvas-importing fixtures with a targeted glob;
      `git diff --stat` shows only `.ir`/`.ast` under those fixtures.~~ — moot: the four canvas-importing fixtures carry
      only `build.log` (Corrections). Replaced by the check that they pass unchanged:
      `scripts/test-accept.sh target/release/mfb /tmp/p137c canvas-setgroup-consumes-items canvas_color_surface_removed_invalid
      canvas_color_type_removed_invalid canvas-drawitem-thread-plane-invalid` → `acceptance tests passed (4 test(s) ran)`;
      the commit touches no golden.

Acceptance: all 17 decode tests pass; the late pass serves a canvas-only program.
  Check: `cargo test --test rt_canvas_image_decode` → 17 passed;
  `scripts/test-accept.sh target/release/mfb /tmp/p137c '<canvas fixture glob>'` → 0 mismatches after sync (est. 15 min).
  (2026-09-14: `cargo test --test rt_canvas_image_decode` → `ok. 17 passed; 0 failed` in 92.68 s; the four canvas fixtures →
  `acceptance tests passed (4 test(s) ran)`; `/tmp/p137latepass` builds.)
Commit: —

### Phase 4 — record

- [x] Post-change size and decode time (Phase 1 program and PNG) recorded in Corrections.
      (Corrections, "Post-change size and decode time".)
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

- **§4's late pass cannot be a plain mirror of `color::augmented_project`** (recorded by plan-137-B work on
  2026-09-14, before this letter started). Evidence by code: `src/codegen/registry/mod.rs` `late_pass_file`
  → `RegistryPackage::get_mfb` → `always_helper_bodies`, which excludes `WhenUsed`/`WhenImported`/
  `WhenBothImported` helpers; `grep -rhoE "HelperGate::[A-Za-z]+|RegistryHelper::[a-z_]+"` over
  `builtins/color/` → `8 RegistryHelper::always`, over `builtins/compress/` → only `HelperGate::WhenUsed`.
  §4 is annotated and Phase 1 gains the probe that confirms it and picks the shape.
- **§4's `synthetic_files: skip compress` would shift the compress byte-identity goldens; the late pass dedupes
  by path instead** (2026-09-14). Evidence: the scratch compiler with the skip (companion and gated-helper loops) plus
  the gate-aware late pass gave `bash scripts/artifact-gate.sh target/release/mfb compress` → `7 golden(s) checked, 6
  diff(s)` (`.ir` and all five `.ncodesum`). Localized on that one fixture: the golden and the rebuilt `.ir` have the same
  78 function names (`grep -oE '^    "name": "[^"]+"' | sort`, `diff` empty), and `diff <(sort golden) <(sort rebuilt)`
  shows 0 differing lines. The only change is order: `#encoding_*` now comes before `#compress_crc32Tables`, because a
  skipped `compress` rides in after the `encoding` late pass. The golden is not wrong, so it stays. The shipped shape does not skip `compress` in
  `synthetic_files` at all. `inject_late_pass`/`inject_late_pass_hir` append only the files whose `path` the project
  does not already have. `late_pass_files` labels helpers exactly as `synthetic_files` does
  (`builtins/<helper>.mfb`). A program that imports `compress` keeps the helpers the generic pass injected, where it
  injected them; a canvas-only program gets them from the late pass; a program with both gets only the helpers canvas's
  companion newly reaches.
- **Post-change size and decode time** (2026-09-14, macos-aarch64, the Phase 3 compiler).
  - **Size:** 1,904,108 B → **1,953,644 B** (+49,536 B, +2.6%) for both the canvas-only and the `loadImage` probe (§2).
    The strict decoder is larger than canvas's reference walker: two-level tables and the tables built at program start.
    Every canvas program carries it, as it carried the walker.
  - **Decode time:** `/tmp/p137canvas/app` rebuilt at `-O1 --app`, run headless by `/tmp/p137canvas/run.py` (in-program
    `datetime::monotonicNanos` around `canvas::loadImage`), one run each as in plan-137-B §2.
    | Image | Before | After |
    |---|---|---|
    | 256² | 203.4 ms | 49.4 ms |
    | 1024² | 3,274.6 ms | 776.5 ms |
    | 4096² (67,112,960 B inflated) | 43,691.6 ms (1.47 MiB/s) | **12,085.1 ms** (5.30 MiB/s whole-`loadImage`) |

    The 4096² decode is 3.6× faster. The remaining time beyond `zlibDecode`'s 3,847.0 ms on the same stream
    (plan-137-B Phase 4) is unfiltering and RGBA conversion.
- **The canvas-importing fixtures carry no `.ir`/`.ast` goldens** (2026-09-14). §2 and Phase 3 expected their
  `.ir`/`.ast` to shift. `ls <fixture>/golden` for all four (`syntax/resources/canvas-setgroup-consumes-items`,
  `syntax/canvas/canvas_color_{surface,type}_removed_invalid`, `syntax/threads/canvas-drawitem-thread-plane-invalid`)
  → `build.log` only. There is nothing to sync, and any `build.log` change is a bug. Phase 3's sync task therefore
  becomes a check that the four fixtures pass unchanged.

## Summary

Small code change, real seam risk: the transitive-import late pass. The decoder's correctness was
proven in B; this letter proves canvas still decodes everything it should and nothing it shouldn't.
