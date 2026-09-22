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

- [ ] Record per row: every raise point, and whether the output length is known
      before the first write (shape a) or not (shape b), with the upper bound shape
      b will use.

Acceptance: recorded here with citations (est. 30 min).
Commit: —

### Phase 2: Split at the output, byte-identical

- [ ] `*_into` halves in `gen_case_map.rs`, `func_normalize_nfc.rs`,
      `builder_strings.rs` (`lower_replace`'s `String` branch), and
      `gen_path_builder.rs` (`pathNormalize`).

Acceptance: `cargo build --release && cargo test --test golden` → 0 `.ncode` diffs
(est. 20 min).
Commit: —

### Phase 3: The arm

- [ ] `ArmId::StrRewrite`, `STRING_REWRITE_FNS`, `try_inplace_string_rewrite_assign`,
      the marker, and the `SELF_UPDATE_ARMS` entry after the collection `Replace`
      arm (which declines a `String` at G10) and after `StrGrow`.
- [ ] `SCRATCH_ARMS` and `is_string_self_update` gain the names in §3.
- [ ] The 6 rows → `Arm([StrRewrite])`; 6 `cases.tsv` lines → `arm`, each paired
      with a restoring statement where the rewrite is not idempotent
      (`x = strings::replace(x, "a", "aa") ; x = strings::replace(x, "aa", "a")`).
- [ ] Runtime: `tests/rt-behavior/strings/self-update-rewrite-valid/`, with the
      differential cases and a trapped failure per raising row, at a local and a
      global.
- [ ] RED proof: skip the ASCII check so every case map takes the in-place byte
      map. The `ß` case must fail. Restore.

Acceptance: `cargo test --bin mfb self_update`, the harness over the six lines, and
`scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/strings/self-update-rewrite-valid'`
→ pass (est. 8 min).
  Expected golden diffs: fixtures that self-update one of the six rows, and every
  fixture that calls `encoding::htmlEscape` (its helper body is 5 `replace`
  self-updates). Run `cargo test --test golden`. Every diff must trace to one of
  these: objdump one per producer. Re-baseline only the traced fixtures.
Commit: —

## Validation Plan

- Tests: the differential and failure fixture, 6 harness lines, 6 matrix rows.
- Per-letter gate: `cargo test --bin mfb`.

## Open Decisions

None beyond plan-146-A's.

## Corrections

## Summary

E lands one arm for the six native rewrites. It builds the result in the reused
self-update scratch and copies it back, with an in-place byte map for ASCII case
maps. It allocates only when `s` must grow past its shadow.
