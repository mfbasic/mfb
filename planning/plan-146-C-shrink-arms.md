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

- [ ] For each of the 13 rows, read its lowering and record here every error it
      can raise and whether each is raised before the window is known (e.g. a
      negative `count`, `graphemeAt` out of range). Record the `pathDirName` inputs
      that take `.` and `/`.

Acceptance: the list is recorded with a `file:symbol` per raise point (est. 30 min).
Commit: —

### Phase 2: Split the lowerings at the window, byte-identical

- [ ] `*_window` halves in `gen_left_right.rs`, `gen_strip.rs`, `gen_trim.rs`,
      `func_trim_chars.rs`, `func_grapheme_at.rs`, `gen_path_builder.rs` (three
      functions), and `lower_mid`'s `String` branch. Each copying lowering calls
      its half and then materializes.

Acceptance: codegen is unchanged.
  Check: `cargo build --release && cargo test --test golden` → 0 `.ncode` diffs
  (est. 20 min: every fixture that calls one of the 13 copies through the split
  code, and the artifact gate is the only check that sees them all). A diff is a
  split bug: objdump one fixture and fix it.
Commit: —

### Phase 3: The arm

- [ ] `ArmId::StrWindow`, `STRING_WINDOW_FNS`, `try_inplace_string_window_assign`,
      the marker, and the `SELF_UPDATE_ARMS` entry after `Mid`.
- [ ] Add the 13 names to `is_string_self_update`.
- [ ] The 13 rows → `Arm([StrWindow])`; 13 `cases.tsv` lines → `arm`.
- [ ] Runtime: `tests/rt-behavior/strings/self-update-window-valid/` holds the
      differential (copy vs self-update) for every row and edge, and a trapped
      failure per raising row that prints `s` unchanged. It runs at a local and at
      a global.
- [ ] `tests/rt-behavior/fs/pathdirname_constant_owned` (bug-667's fixture, lines
      19–20 are `e = fs::pathDirName(e)`) passes unchanged.
- [ ] RED proof: make the arm skip `emit_string_set_len`'s shadow update. The
      differential case must fail, or the debug build must report a free-size
      mismatch. Record which. Restore.

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
Commit: —

## Validation Plan

- Tests: the differential and failure-atomicity fixture, 13 harness lines, 13
  matrix rows.
- Per-letter gate: `cargo test --bin mfb`.

## Open Decisions

None beyond plan-146-A's (Open Decision 4 decides what the freed bytes become;
this letter is written for the recommended option).

## Corrections

## Summary

C lands one arm for the 13 window-shaped builtins: the window is computed by the
code the copying path already runs, then moved down in place. The split is
proven byte-identical before any behavior changes.
