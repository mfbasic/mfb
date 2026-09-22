# plan-146-C: Shrink arms, 13 `String` rows

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-146-B

Prerequisites: see plan-146-A.

Thirteen `String` builtins return a contiguous window of their first argument
(findings §3.4 "shrink", Appendix B.2): `strings::left right mid stripPrefix
stripSuffix trim trimStart trimEnd trimChars graphemeAt`, and `fs::pathBaseName
pathDirName pathExtension`. Each lowering already computes the window as
`(ptr, len)` into `value` and then copies it into a fresh block
(`emit_materialize_string_from_bytes`, `builder_collection_layout.rs:2376`;
`lower_mid`'s `String` branch, `builder_search.rs:663`, allocates at
`mid_alloc_ok`). The in-place form moves the window to offset 8 of `s`'s own block,
stores the new length and the NUL, and adds the freed bytes to the shadow. After
C, none of the 13 allocates at S1 or S2.

## 1. Goal

- One arm, `ArmId::StrWindow`, serves all 13 rows through a table
  `STRING_WINDOW_FNS: &[(&str /* bare name */, RangeInclusive<usize> /* arity */, WindowFn)]`.
  This mirrors plan-142-C's single `Math` arm for 16 functions. Its marker is
  `inplace_str_window`.
- The 13 rows flip `Pending("C")` → `Arm([StrWindow])`, and their `cases.tsv` lines
  flip `pending:C` → `arm`. The matrix and the harness pass for them at S1 and S2.
- Failure atomicity: every error a row can raise (Phase 1 lists them) is raised
  before the first byte moves. A runtime case per raising row shows `s` unchanged
  after a trapped failure.
- The value is exactly the copying lowering's: a differential runtime case per row
  compares `LET t = f(s, …)` with `s = f(s, …)` over inputs that cover each
  window edge (empty result, whole string, multi-byte UTF-8 at both ends).

### Non-goals

- The copying lowerings stay. They serve `LET t = f(s, …)` and every
  non-self-update call, and the arm shares their window step, not their output.
- No change to any result.
- S9 stays declined (letter G).
- The `AttributedString` overloads stay deferred (plan-146-A Open Decision 1).

## 2. Current State

| row | lowering (findings Appendix B.2) | window step |
|---|---|---|
| `left`, `right` | `gen_left_right::lower_strings_left_right` | `(ptr, len)` then materialize |
| `mid` | `lower_mid`, `String` branch (`builder_search.rs:663`) | span then `mid_alloc_ok` copy |
| `stripPrefix`, `stripSuffix` | `gen_strip::lower_strings_strip` | window (the whole of `value` when the affix is absent) |
| `trim`, `trimStart`, `trimEnd` | `gen_trim::lower_strings_trim` | `[start, end)` |
| `trimChars` | `func_trim_chars::lower` | window |
| `graphemeAt` | `func_grapheme_at::lower` | one grapheme's span |
| `pathBaseName`, `pathExtension` | `gen_path_builder.rs` | last component / extension span |
| `pathDirName` | `gen_path_builder.rs` | directory span, **or** `.` / `/` (a fresh copy since bug-667) |

`strings.mid` reaches the collection `Mid` arm first through the shared bare name
and declines at G10 (findings Appendix B.1). `StrWindow` goes after it in
`SELF_UPDATE_ARMS`. Both decline without emitting, so the order only has to be
recorded, as `append`/`bulk_append` are.

## 3. Design

**Split each lowering at the window.** Each of the six lowering functions gets a
`*_window` half that emits the argument checks and leaves `(start, len)` in two
frame slots, with `start` as an address inside `value`'s bytes. The copying path
becomes `window` + `emit_materialize_string_from_bytes`. That is byte-identical
for the copying path, and Phase 2's gate checks it.

**The arm.** `try_inplace_string_window_assign`:

1. `resolve_string_self_update(site, value, name ∈ STRING_WINDOW_FNS, arity, needs_shadow = true)`.
2. Load the block from `site.dest.block_slot()` and run the row's window step on
   it. Every check and raise happens here, before any write.
3. Move `len` bytes from `start` to `block + 8` with a forward byte loop. The
   destination is never after the source, so the overlap is safe. Skip the move
   when `start == block + 8`.
4. `emit_string_set_len(site, len)`: `shadow += oldLen − len`, store `len`, write
   the NUL.
5. `close_inplace_dest` publishes (S2), and the seam does that already.

**`pathDirName`'s constants.** For `.` and `/` the window step yields a 1-byte
constant instead of a span. The arm writes the byte itself. When `oldLen == 0`
(`pathDirName("")` → `.`), the result is one byte longer than `s`, so the arm calls
`emit_string_reserve(site, 1)` first. That allocates before any write, as the
failure-atomicity rule requires. Phase 1 confirms which inputs take each constant.

**The shadow predicate** (`is_string_self_update`) adds the 13 bare names, so every
local or global that is the target of one of them gets a shadow.

Risk: the window split in six lowerings. A slip there changes the copying path
too, so the split is its own phase with a byte-identity gate. The arm's own
correctness risk is the overlapping move and the shadow arithmetic. The
differential runtime cases cover both.

Rejected alternatives:

- **One arm per function (13 arm ids).** Thirteen copies of the same move-and-set
  tail, and thirteen markers for the matrix to track.
- **Keep the old length and slice by offset** (store a start offset in the block):
  a `String` block has no offset field, and every reader assumes the bytes start at
  `+8` (`03_heap-values.md`).

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work. `- [~]` partial. Moot tasks struck through with evidence, never
> deleted. **An unticked box means NOT DONE.**

### Phase 1: Read the raise points

- [x] For each of the 13 rows, read its lowering and record here every error it
      can raise and whether each is raised before the window is known (e.g. a
      negative `count`, `graphemeAt` out of range). Record the `pathDirName` inputs
      that take `.` and `/`.

      | row | raises | before the window? |
      |---|---|---|
      | `left`, `right` | `ErrInvalidArgument` for `count < 0` (`strings/gen_left_right.rs:lower_strings_left_right`, label `strings_lr_invalid`, `builder.raise_error("strings.left"/"strings.right", …)`) | yes — the first two instructions after loading `count` |
      | `mid` | `ErrIndexOutOfRange` (`collection/search/builder_search.rs:lower_mid`, label `mid_invalid_range`) for `start < 0`, `count < 0`, `start + count` wrapping, or a range past the end | yes — every branch to `mid_invalid_range` is taken before the span is known; the raise is *emitted* after the copy, so the arm emits it after its move the same way |
      | `stripPrefix`, `stripSuffix` | none (`strings/gen_strip.rs:lower_strings_strip`) | — |
      | `trim`, `trimStart`, `trimEnd` | none (`strings/gen_trim.rs:lower_strings_trim`) | — |
      | `trimChars` | none (`strings/func_trim_chars.rs:lower`) | — |
      | `graphemeAt` | `ErrIndexOutOfRange` (`strings/func_grapheme_at.rs:lower`, label `strings_grapheme_at_invalid`) for `index < 0` or `index >= count`; the copying path also inherits `ErrOutOfMemory` from the `List OF String` it builds (`gen_graphemes.rs:lower_strings_graphemes`) | yes for the range check; the arm's own walk raises before it moves a byte and allocates nothing (Correction C1) |
      | `pathBaseName`, `pathExtension` | none (`fs/gen_path_builder.rs:lower_fs_path_base_name` / `_extension`) | — |
      | `pathDirName` | none (`fs/gen_path_builder.rs:lower_fs_path_dir_name`) | — |

      Every row also inherits `ErrOutOfMemory` from `emit_materialize_string_from_bytes`
      — the copying path's allocation, which the arm does not make.

      `pathDirName`'s constants (`lower_fs_path_dir_name`, labels `dot` and `root`):
      `.` for `length == 0` (`fs::pathDirName("")`) and for a path whose backward
      scan finds no `/` (`"abc"`); `/` for `length == 1` with the byte `/`
      (`fs::pathDirName("/")`) and for a slash found at index 0 (`"/abc"`). The `.`
      route is the only window that does not point into the binding — it points at
      `load_string_constant(".")` — and the only one that can be LONGER than the
      binding (`""` → `.`).
Commit: —

### Phase 2: Split the lowerings at the window, byte-identical

- [x] `*_window` halves in `gen_left_right.rs`, `gen_strip.rs`, `gen_trim.rs`,
      `func_trim_chars.rs`, ~~`func_grapheme_at.rs`~~, `gen_path_builder.rs` (three
      functions), and `lower_mid`'s `String` branch. Each copying lowering calls
      its half and then materializes.
      `func_grapheme_at.rs` is NOT split: its window comes out of a freshly built
      `List OF String`, so the arm gets a new non-allocating walk instead
      (Correction C1). `lower_mid`'s half takes the registers `lower_mid` allocated
      (`MidStringRegs`) and returns the copying path's own `result_slot`/`alloc_ok`
      (`MidWindow`), so both paths keep one slot, register and label order;
      `fs_path_extension_window` likewise returns its `done` label.

Acceptance: codegen is unchanged.
  Check: `cargo build --release && cargo test --test golden` → 0 `.ncode` diffs
  (est. 20 min: every fixture that calls one of the 13 copies through the split
  code, and the artifact gate is the only check that sees them all). A diff is a
  split bug: objdump one fixture and fix it.
Result: `artifact-gate [all]: 1485 tests, 1660 build(s), 2098 golden(s) checked,
0 diff(s)` (204.82 s).
Commit: —

### Phase 3: The arm

- [x] `ArmId::StrWindow`, `STRING_WINDOW_FNS`, `try_inplace_string_window_assign`,
      the marker, and the `SELF_UPDATE_ARMS` entry after `Mid`. (Its `FIELD_NEVER`
      row at all 15 field sites too, plan-146-B Correction B4.)
- [x] Add the 13 names to `is_string_self_update` (`STRING_SHADOW_ARMS`).
- [x] The 13 rows → `Arm([StrWindow])`; 13 `cases.tsv` lines → `arm`.
- [x] Runtime: `tests/rt-behavior/strings/self-update-window-valid/` holds the
      differential (copy vs self-update) for every row and edge, and a trapped
      failure per raising row that prints `s` unchanged. It runs at a local and at
      a global. 39 differentials (empty result, whole string, multi-byte UTF-8 at
      both ends, absent affix, all-whitespace, a combining cluster, the four
      `pathDirName` routes), 3 shrink-then-grow cases (the shadow arithmetic), and
      5 trapped failures — every line `ok` / `raised=TRUE s=[abcdef]`.
- [x] `tests/rt-behavior/fs/pathdirname_constant_owned` (bug-667's fixture, lines
      19–20 are `e = fs::pathDirName(e)`) passes unchanged: `acceptance tests
      passed (1 test(s) ran)`, no golden touched.
- [x] RED proof: make the arm skip `emit_string_set_len`'s shadow update. The
      differential case must fail, or the debug build must report a free-size
      mismatch. Record which. Restore.
      **Neither: the harness fails, and the debug build leaks.** The differential
      still agreed (the value is right; only the block's spare bytes are lost), and
      no free-size assertion fired — but `--debug` reported `arena.0.live_bytes
      1248` against 816 with the fix, and 244 allocations against 239 (the
      under-free and the regrows a lost shadow forces). The harness is the sharp
      signal: `MFB_SELF_UPDATE_SITES=Local,Global` over `strings::left`/`trim` →
      `4 of 4 case/site pair(s) failed`, e.g. `strings::left(value AS String, count
      AS Integer) AS String at Local: marked `arm`, but 2000 more runs allocated
      1000 more blocks (1156 at N=2000, 2156 at 2N) — the statement copies`
      (the paired `&` can no longer see the spare bytes). Restored.

Acceptance: `cargo test --bin mfb self_update`,
`MFB_SELF_UPDATE_FILTER` over the 13 lines (`for f in strings::left strings::right strings::mid strings::strip strings::trim strings::graphemeAt fs::path; do …; done`),
and `scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/strings/self-update-window-valid'`
→ pass (est. 10 min).
  Expected golden diffs: fixtures that self-update one of the 13 rows, and fixtures
  that call `json` number formatting (`helper_round_digits.rs:59`,
  `kept = strings::left(kept, …)`) or build a multipart boundary
  (`helper_multipart_boundary.rs:22`). Run `cargo test --test golden`. Every diff
  must trace to one of these producers: objdump one fixture per producer. Any
  other diff is a bug. Re-baseline only the traced fixtures, per AGENTS.md.
Result: `cargo test --bin mfb self_update` → `10 passed`; the harness over the 13
lines at S1/S2 (7 filters) → every one `test result: ok`;
`scripts/test-accept.sh … 'rt-behavior/strings/self-update-window-valid'` →
`acceptance tests passed`. Golden: 5 diffs, all `byte-identity/http`
(`http_codegen_cover_rt`, ×5 targets) — traced to ONE producer, exactly as
predicted: the `.ncode` has a `inplace_str_window` slot in exactly one of its 162
functions, `#http_multipartBoundary` (`b = strings::trim(b)`,
`http/helper_multipart_boundary.rs:22`). The five `.ncodesum` goldens were
regenerated for that fixture alone; the gate then reported `2100 golden(s)
checked, 0 diff(s)`. The predicted `json` producer did NOT move (Correction C2).
Commit: —

## Validation Plan

- Tests: the differential and failure-atomicity fixture, 13 harness lines, 13
  matrix rows.
- Per-letter gate: `cargo test --bin mfb`.

## Open Decisions

None beyond plan-146-A's (Open Decision 4 decides what the freed bytes become;
this letter is written for the recommended option).

## Corrections

- **C1 — `graphemeAt` gets a new window, not a split.** §3 says each of the six
  lowerings is split at its window, but `func_grapheme_at.rs:lower` has no window
  into `value`: it calls `lower_strings_graphemes` (`gen_graphemes.rs`), which
  ALLOCATES a `List OF String` of every cluster, and takes its span out of that
  list's data. An arm built on it would allocate one list per statement and fail
  the harness bound outright. So the copying lowering is left untouched and the arm
  gets `gen_graphemes::grapheme_at_window`: the same segmentation emitters
  (`emit_utf8_decode_next`, `emit_unicode_property_boundclass`,
  `emit_unicode_property_indic_conjunct_break`, `emit_grapheme_break_branch`,
  `emit_grapheme_state_update`) walked over `value`'s own bytes, stopping at the
  wanted cluster, allocating nothing. Proof it is right: the fixture's
  `graphemeAt0/1/Last/Only` differentials (including an `e`+U+0301 cluster and an
  emoji) agree with the copying path, and the harness line meets the `arm` bound.
- **C2 — one of the two predicted golden producers does not fire.** Phase 3
  predicted diffs from `json/helper_round_digits.rs:59` as well as
  `http/helper_multipart_boundary.rs:22`. Only the `http` one moved. The json line
  is `kept = strings::left(kept, index) & "0" & strings::mid(kept, index + 1, …)` —
  a `&` chain whose leftmost leaf is a CALL, not `kept`, so it is not a self-update
  of `kept` at all (neither the concat arm's G20 nor `resolve_string_self_update`'s
  G2 accepts it). Measured: `mfb build -q -ncode tests/byte-identity/json` → 165
  functions, 0 with a `inplace_str_*` slot. No json golden moved, and none should.

## Summary

C lands one arm for the 13 window-shaped builtins: the window is computed by the
code the copying path already runs, then moved down in place. The split is
proven byte-identical before any behavior changes.
