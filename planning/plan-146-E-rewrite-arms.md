# plan-146-E: Rewrite arms for the six native rewrites

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-146-D

Prerequisites: see plan-146-A.

Six natively lowered `String` builtins produce a result whose length can change
either way (findings §3.4 "rewrite"): `strings::upper`, `lower`, `caseFold`,
`normalizeNfc`, `strings::replace`, and `fs::pathNormalize`. The result cannot be
written over `s` as it is computed, because the writer can overtake the reader
(`ß` → `SS`, a longer replacement). The in-place form builds the result in the
function's **self-update scratch**, a per-function buffer grown geometrically and
reused across statements (plan-142-B Correction B1). Then it copies the result
back into `s`'s block, growing it through the shadow when needed. The case maps
also get the fast path findings §3.4 names: on all-ASCII input they are
same-length and map bytes where they lie, with no scratch. After E, none of the
six allocates per statement at S1 or S2.

## 1. Goal

- `ArmId::StrRewrite`, one arm for the six rows through a table
  `STRING_REWRITE_FNS`. Its marker is `inplace_str_rewrite`.
- The 6 rows flip `Pending("E")` → `Arm([StrRewrite])`, and their `cases.tsv` lines
  flip `pending:E` → `arm`.
- Failure atomicity: every raise (Phase 1 lists them) happens while the result is
  still in the scratch. `s` is written only by the final copy-back, after
  `emit_string_reserve` has allocated if it needed to.
- A differential case per row: ASCII and non-ASCII input for the case maps
  (`ß`, `İ`, Greek final sigma for `lower`), decomposed vs precomposed input for
  `normalizeNfc`, `replace` with a shorter, equal, longer and empty replacement and
  no match, and `pathNormalize` with `..`, `.`, repeated `/`, and a path that
  normalizes to `.`.

### Non-goals

- The copying lowerings stay for non-self-update calls.
- No change to any result.
- The 10 MFBASIC-body rewrites (`encoding`, `net::percentDecode`, `regex::replace`)
  are letter F's (plan-146-A Open Decision 2).
- S9 stays declined (letter G).

## 2. Current State

| row | lowering (findings Appendix B.2) | result block |
|---|---|---|
| `upper`, `lower`, `caseFold` | `Body::abi_inline` → `gen_case_map::lower_strings_case_map` | `emit_arena_alloc_call` of `byteLen + 9` (ASCII) or of the counted mapped length (Unicode count loop) |
| `normalizeNfc` | `Body::abi_inline` → `func_normalize_nfc::lower` | three `emit_arena_alloc_call` sites |
| `strings::replace` | `Body::Intrinsic` → `lower_replace` (`string/repr/builder_strings.rs:15`) | alloc when a match is replaced, `copy_flat_block` of `value` when none is |
| `pathNormalize` | `Body::abi_inline` → `lower_fs_path_normalize_nl` (`gen_path_builder.rs`) | normalized bytes assembled and materialized |

- The scratch: `prescan_self_update_scratch` (`self_update.rs:503`) gives a
  function a `su_scratch` slot when it holds a self-update whose target
  `target_needs_self_update_scratch` names (`SCRATCH_ARMS`, `:356`).
  `emit_reserve_self_update_scratch` (`:537`) grows it. `replace` is already in
  `SCRATCH_ARMS` for the collection arm, and `strings.replace` resolves to the same
  bare name, so a function holding `s = strings::replace(s, …)` already gets a
  scratch slot today.
- In-tree producers: `examples/browser/{app,dom,display}` hold
  `t = strings::replace(t, …)` 8 times, and the `encoding::htmlEscape` helper body
  holds `out = strings::replace(out, …)` 5 times
  (`encoding/func_html_escape.rs:43-47`). B's fixture grep, Appendix.

## 3. Design

**Split each lowering at its output.** Each lowering gets an `*_into` half that
writes the result bytes to a caller-supplied `(dest_ptr, capacity)` and returns
the length. The copying path allocates the arena block and calls `*_into`. Phase 1
decides the shape per row, from how the lowering sizes its output:

- **(a) exact length known before writing** (case maps' count loop, `replace`
  after a match-count pass): reserve that many scratch bytes, then write.
- **(b) grows while writing** (`normalizeNfc`'s three allocation sites,
  `pathNormalize`'s assembly): the `*_into` half takes an upper bound computed
  first, or reserves the scratch in steps (`emit_reserve_self_update_scratch` is
  re-entrant). Phase 1 records which for each.

**The arm.** `try_inplace_string_rewrite_assign`:

1. `resolve_string_self_update(…, needs_shadow = true)`, plus a gate that the
   function has a scratch slot (`self.self_update_scratch.is_some()`), checked
   before emitting (the scratch contract, `:531-536`).
2. Case maps only: if every byte of `s` is < 0x80, map bytes in place and finish
   (same length, no scratch, no shadow change).
3. Otherwise run `*_into` with the scratch as the destination.
4. `emit_string_reserve(site, newLen)`, copy the scratch to `+8`, then
   `emit_string_set_len(site, newLen)`.

**Scratch and shadow prescans.** Add the case-map, `normalizeNfc` and
`pathNormalize` bare names to `SCRATCH_ARMS` (`replace` is there already). Add all
six to `is_string_self_update`.

Risk: the output split in four lowerings (Phase 2, byte-identity gate), and
`normalizeNfc`'s growing output. The differential cases are the correctness proof.

Rejected alternatives:

- **Write over `s` right to left** after computing the final length: correct only
  when the output is at least as long everywhere as the input it replaces, which no
  row guarantees.
- **An arena temporary per statement:** that is a copy, and it breaks the harness
  bound (plan-142-A Correction A6).

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work. `- [~]` partial. Moot tasks struck through with evidence, never
> deleted. **An unticked box means NOT DONE.**

### Phase 1: Read the six lowerings

- [x] Record per row: every raise point, and whether the output length is known
      before the first write (shape a) or not (shape b), with the upper bound shape
      b will use.

      - **`upper` / `lower` / `caseFold`** (`strings/gen_case_map.rs:lower_strings_case_map`).
        Raises: four `raise_error_bare("ErrOutOfMemory")` — the ASCII path's
        `emit_arena_alloc_call` and its `ascii_size_overflow`
        (`emit_checked_size_add_immediate(byteLen, 9)`), and the slow path's alloc
        and `strings_case_map_size_overflow`. No `ErrInvalidArgument` path at all.
        **Shape (a) on both paths**: the ASCII path proves every byte < 0x80 (the
        SWAR scan `ascii_scan`) and then `newLen == byteLen(value)`; the slow path's
        count loop (`count_loop` … `count_done`) computes the full mapped length
        into `length_slot` BEFORE the allocation and before any output byte (1 byte
        per cp < 0x80, else `emit_case_map_lookup` → `emit_utf8_encoded_width` of
        the identity or of each mapped cp — the re-encoded width, bug-175 B). Two
        `emit_arena_alloc_call` sites (`byteLen + 9`, `length_slot + 9`), each
        followed by the length word and a write loop (`ascii_transform_loop` /
        `write_loop`) and a trailing NUL; the slow path ends with
        `emit_write_cursor_assert` against `result + 8 + length_slot`.
        The whole-string ASCII gate §3 step 2 wants already exists here.
      - **`normalizeNfc`** (`strings/func_normalize_nfc.rs:lower`). Raises: six
        `raise_error_bare("ErrOutOfMemory")` — the ASCII copy path's alloc and
        `ascii_size_overflow`; the scalar temp buffer's alloc and
        `strings_nfc_size_overflow` (`emit_checked_size_multiply(scalarCount, 8)`,
        audit-unicode #8); the result's alloc and a second overflow check
        (`…_add_immediate(outputLen, 9)`, bug-378). **Shape (a)**, contrary to the
        plan's table: the composed scalars are built in a `u64` temp buffer first,
        then `byte_len_loop` sums `emit_utf8_encoded_width` over them into
        `output_len_slot` before the result is allocated or written. Three
        `emit_arena_alloc_call` sites: the ASCII copy (`byteLen + 9`), the scalar
        temp (`scalarCount × 8` — a genuine second scratch, of `u64`s, not bytes),
        and the result (`output_len + 9`, written by `encode_loop`).
      - **`strings::replace`** (`string/repr/builder_strings.rs:lower_replace`,
        the `String` branch). Raises: `replace_empty_old` →
        `raise_error("strings.replace", "ErrInvalidArgument")` for
        `byteLen(old) == 0` (bug-533); `raise_error_bare("ErrOutOfMemory")` on the
        alloc and at `replace_overflow` (`emit_checked_size_add(output_len,
        new_len)`, `…_add_immediate(output_len, 9)`); plus the no-match arm's
        `copy_flat_block`, which has its own alloc and `ErrOutOfMemory`.
        **Shape (a)**: the first pass counts non-overlapping leftmost matches and
        accumulates `output_len = byteLen(value) + Σ(newLen − oldLen)` before any
        output byte. Two no-match shortcuts branch to `copy_original`
        (`byteLen(old) > byteLen(value)`, and no match found) — for an arm that is a
        no-op on `s` needing no scratch at all.
      - **`fs::pathNormalize`** (`fs/gen_path_builder.rs:lower_fs_path_normalize`).
        Raises: exactly one, `raise_error_bare("ErrOutOfMemory")` on its single
        `emit_arena_alloc_call`; no argument is ever rejected. **Shape (b)** — the
        only one: it allocates first and builds the result incrementally, tracking
        the length in `out_len_slot`, and `..` TRUNCATES an already-written region
        (`pop_previous`/`pop_scan`/`pop_store` scan the output backwards for the
        preceding `/`). The upper bound is already in the source: the allocation is
        `byteLen(path) + 10` = 8 + `byteLen(path)` + 1 (the `"."` fallback) + 1
        (NUL), so **`byteLen(s) + 1` output bytes suffice** and a scratch reserved
        once never needs to grow. The destination must be READABLE as well as
        writable (`pop_scan`, and the "last byte already `/`?" test).
Commit: d26f9edd4 (Phases 1-3 landed as one commit)

### Phase 2: Split at the output, byte-identical

- [x] `*_into` halves in `gen_case_map.rs`, `func_normalize_nfc.rs`,
      `builder_strings.rs` (`lower_replace`'s `String` branch), and
      `gen_path_builder.rs` (`pathNormalize`).
      Landed as ONE destination parameter rather than four split halves
      (Correction E1): each lowering takes a `StringOut` — `Block` (the copying
      path, byte for byte) or `Scratch` (build the bytes in the function's
      self-update scratch and hand the arm a pointer and a length).
      `lower_strings_case_map_out`, `func_normalize_nfc::lower_out`,
      `lower_replace_out` and `lower_fs_path_normalize_out`.

Acceptance: `cargo build --release && cargo test --test golden` → 0 `.ncode` diffs
(est. 20 min).
Result: `artifact-gate.sh target/release/mfb strings` → `0 diff(s)` after the case
map and after `replace`; `… collections` → `0 diff(s)` (the `List` overload shares
`lower_replace`); `… strings` and `… fs` → `0 diff(s)` after `pathNormalize` and
`normalizeNfc`. The full gate ran at the end of Phase 3.
Commit: d26f9edd4

### Phase 3: The arm

- [x] `ArmId::StrRewrite`, `STRING_REWRITE_FNS`, `try_inplace_string_rewrite_assign`,
      the marker, and the `SELF_UPDATE_ARMS` entry after the collection `Replace`
      arm (which declines a `String` at G10) and after `StrGrow` (plus its
      `FIELD_NEVER` row at all 15 field sites, plan-146-B Correction B4).
- [x] `SCRATCH_ARMS` and `is_string_self_update` gain the names in §3 — the scratch
      one by QUALIFIED target, not bare name (Correction E2).
- [x] The 6 rows → `Arm([StrRewrite])`; 6 `cases.tsv` lines → `arm`, each paired
      with a restoring statement where the rewrite is not idempotent
      (`x = strings::replace(x, "a", "aa") ; x = strings::replace(x, "aa", "a")`).
- [x] Runtime: `tests/rt-behavior/strings/self-update-rewrite-valid/`, with the
      differential cases and a trapped failure per raising row, at a local and a
      global. 32 cases: the ASCII and Unicode case-map paths (`ß` → `SS` and
      `İ` → `i`+combining dot, both LONGER than their input; Greek final sigma),
      NFC decomposed/precomposed/reordered (shorter), `replace` longer/shorter/
      equal/empty/no-match/multi-byte, `pathNormalize` with `..`, `.`, repeated
      `/`, a path that becomes `.`, one that climbs above the root and the empty
      path, the four rows again at a global, three rewrite-then-use cases, and the
      one trapped failure (`strings::replace(s, "", "x")`) — every line `ok` /
      `raised=TRUE s=[abc]`.
- [x] RED proof: skip the ASCII check so every case map takes the in-place byte
      map. The `ß` case must fail. Restore.
      `upperSharpS MISMATCH [STRAßE] want [STRASSE]`, and with it
      `lowerDottedI`, `lowerSigma`, `caseFoldGreek`, `g.upperSharpS` and
      `rewriteThenAppend` — every non-ASCII case. Restored.

Acceptance: `cargo test --bin mfb self_update`, the harness over the six lines, and
`scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/strings/self-update-rewrite-valid'`
→ pass (est. 8 min).
  Expected golden diffs: fixtures that self-update one of the six rows, and every
  fixture that calls `encoding::htmlEscape` (its helper body is 5 `replace`
  self-updates). Run `cargo test --test golden`. Every diff must trace to one of
  these: objdump one per producer. Re-baseline only the traced fixtures.
Result: `cargo test --bin mfb self_update` → the matrix fires `StrRewrite` for all
six rows at S1 and S2; the harness over the six lines at S1/S2 → every filter
`test result: ok`; `scripts/test-accept.sh … 'rt-behavior/strings/self-update-rewrite-valid'`
→ `acceptance tests passed`. Golden: 53 diffs over 11 fixtures
(`byte-identity/{compress,crypto,csv,encoding,json,regex,resource-xfer-slots,strings,tls}`,
`rt-behavior/crypto/crypto-ec-valid`, `syntax/app/app-mouse-surface`), every one
traced to the SAME single producer — `#encoding_htmlEscape`, whose body holds five
`out = strings::replace(out, …)` self-updates and which is injected into all of
them. Verified by rebuilding three of the eleven (`csv`, `tls`, `strings`): each
`.ncode` has exactly ONE function with a `inplace_str_rewrite`/`su_scratch` slot,
`#encoding_htmlEscape`, out of 101 / 121 / 96. No fixture self-updates a rewrite
row in its own source. The 53 `.ncodesum` goldens were regenerated for those 11
fixtures alone; the gate then reported `1488 tests, 1663 build(s), 2104 golden(s)
checked, 0 diff(s)`.
Commit: d26f9edd4

## Validation Plan

- Tests: the differential and failure fixture, 6 harness lines, 6 matrix rows.
- Per-letter gate: `cargo test --bin mfb`.

## Open Decisions

None beyond plan-146-A's.

## Corrections

- **E1 — one destination parameter, not four `*_into` halves.** §3 asks each
  lowering to be split into a half that writes to a caller-supplied
  `(dest_ptr, capacity)`. Three of the four cannot be cut that way without
  duplicating their control flow: the case maps and `replace` interleave the
  allocation between their count pass and their write pass, and `pathNormalize`
  READS its own output while building it (`..` pops the previous component back
  off, `pop_scan`). So each lowering instead takes a `StringOut` telling it where
  its result goes — `Block` (allocate, exactly as before) or `Scratch` (the
  function's self-update scratch) — which keeps ONE implementation of every
  rewrite and leaves the copying path byte-identical (proven by the artifact gate
  after each one). For `normalizeNfc` and `pathNormalize` the scratch region is
  given the SHAPE of a block (an 8-byte length word then the bytes), so every
  `+8` offset in those lowerings is untouched.
- **E2 — `replace` is NOT in `SCRATCH_ARMS`, and must not be added by name.**
  §2 says "`replace` is already in `SCRATCH_ARMS` for the collection arm, so a
  function holding `s = strings::replace(s, …)` already gets a scratch slot
  today". Measured false: `SCRATCH_ARMS` is `filter, distinct, transform, sort,
  sortBy, union, intersection, difference, symmetricDifference, mapValues, merge`
  — no `replace` (the collection `replace` arm keeps its state in the collection).
  The arm declined for that reason alone (`DBG rewrite: bare=replace
  scratch=false`). Adding the bare name would give every
  `xs = collections::replace(xs, …)` function a scratch slot and drop it never
  uses — dead code in every such program, and a golden move. The scratch
  requirement is therefore keyed on the QUALIFIED target
  (`target_needs_string_scratch`: `strings.upper|lower|caseFold|normalizeNfc|replace`,
  `fs.pathNormalize`, `os.resourcePath`), which a `collections::replace` call
  never matches.
- **E3 — `normalizeNfc` needs BOTH its buffers in the scratch, reserved once.**
  Its slow path allocates a `u64` scalar array as well as the result block, and a
  second `emit_reserve_self_update_scratch` would move the first (the reserve
  frees and reallocates rather than growing in place). The arm reserves
  `scalarCount × 12 + 9` once — `× 8` for the scalar array, at most `× 4` bytes of
  UTF-8 output plus a block header — and lays the result region out after the
  array. `strings::normalizeNfc` and the case maps also keep their all-ASCII
  shortcut in the ARM (ASCII is already NFC; an ASCII case map is one byte in, one
  byte out and runs over the binding's own bytes), so neither reaches the scratch
  for ASCII input at all.

## Summary

E lands one arm for the six native rewrites. It builds the result in the reused
self-update scratch and copies it back, with an in-place byte map for ASCII case
maps. It allocates only when `s` must grow past its shadow.
