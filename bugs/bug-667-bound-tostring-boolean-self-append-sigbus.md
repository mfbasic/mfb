# bug-667: a bound `toString(<Boolean>)` grown in place frees read-only data (SIGBUS)

Last updated: 2026-09-21
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Tests:
- `tests/rt-behavior/general/tostring_boolean_self_append` (A)
- `tests/rt-behavior/general/tostring_string_owning_store` (B)
- `tests/rt-behavior/fs/pathdirname_constant_owned` (C)
- `tests/rt-behavior/general/string_global_default_owned` (D)

`toString(<Boolean>)` returns a pointer to one of two read-only string constants
(`TRUE`/`FALSE`) rather than a fresh arena block. A `MUT` binding initialized
from it and then grown by the in-place self-append (`s = s & "!"`) treats that
pointer as its own buffer: the regrow path `arena_free`s the old buffer, which
writes the free list into read-only memory, and the program dies with SIGBUS
(exit 138) before printing anything.

**The correct behavior:** `MUT a = toString(flag); a = a & "!"` prints
`TRUE!`, and the program exits 0. A `String` produced by `toString` behaves in
every respect like any other `String` value.

References:

- Found while landing plan-140-C (`toString(ENUM)`), whose first design returned
  the member name as a read-only pointer the same way and crashed identically;
  plan-140-C Correction C-C3.
- `src/docs/spec/memory/` — ownership: every live value has exactly one owner.

## Failing Reproduction

```
IMPORT io

FUNC flag(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  MUT a = toString(flag(1))
  a = a & "!"
  io::print(a)
  RETURN 0
END FUNC
```

- Observed (main at b6a10efbc, macOS aarch64): no output, exit 138.
- Expected: `TRUE!`, exit 0.

Contrast: `MUT b = "lit"; b = b & "!"` works (a literal is recognized by
`static_string_value` and copied on bind); `MUT a = typeName(x); a = a & "!"`
works (folded to a constant first).

## Scope extension (2026-09-21) — three more producers of the same class

Found by the plan-143 audit (`planning/plan-143-findings/string-self-update-audit.md`
§3.2 F1, F3) and by this bug's own producer audit. Each is a `String` that an owning
store takes as its own although the binding does not own the block:

- **B — `toString(<String>)` is an identity.** `lower_to_string`'s `String` arm
  (`src/codegen/string/repr/builder_strings.rs`) returns the argument's own block,
  and `value_needs_owning_copy` does not class a call as an aliasing source. So
  `s = toString(s)` frees `s`'s block and stores it (the reassign path's
  `emit_owned_value_drop`), at a local, a global (`StoreGlobal`) and a by-ref lambda
  capture alike; `LET t = toString(s)` aliases `s`; `RETURN toString(p)` hands back
  the caller's argument; `toString("lit")` hands back rodata. Repro: the
  `tostring_string_owning_store` fixture — observed `######## ########` then exit
  139 (the first binding reads the junk string allocated after it).
- **C — `fs::pathDirName` returns rodata** `.` for a path with no directory part and
  `/` for the root (`load_string_constant`, `lower_fs_path_dir_name` in
  `src/codegen/builtins/fs/gen_path_builder.rs`). `LET a = fs::pathDirName("abc")`
  prints `[.]` and dies with SIGBUS at scope end. Repro: `pathdirname_constant_owned`
  — observed `[.]`, `[/]`, `[/]`, exit 138.
- **D — a module-level `MUT s AS String` with no initializer holds the rodata empty
  string** (`_mfb_str_empty`). The `StoreGlobal` no-value path stores
  `lower_default_value`'s constant pointer as-is (`builder_control.rs`,
  `NirOp::StoreGlobal`), while the local `Bind` path copies it into the arena. The
  first reassignment frees rodata: SIGBUS for a plain reassignment and for the
  in-place self-append alike. Repro: `string_global_default_owned` — observed no
  output, exit 138.

Producer audit (`grep -rn 'emit_load_string_constant(\|load_string_constant(' src/codegen`,
plus `load_empty_string_constant`): the enum arm (already copies, plan-140-C), the
Boolean arm (A), `pathDirName` (C), `static_string_value` literals (copied by
`value_needs_owning_copy`), `typeName` (folded to a static string first: `MUT t =
typeName(n); t = t & "!"` prints `Integer!` — probe `/tmp/b667/audit`), the error
emitters (the filename goes to a runtime helper as an argument, never returned), the
default local (copied on bind) and the default global (D). No other producer returns
a constant as a call's value.

Found alongside, not this class: `typeName(<user FUNC call>)` fails to compile —
`error: native code cannot determine typeName argument type` — filed separately.

## Root Cause

`lower_boolean_to_string` (`src/codegen/string/repr/builder_strings.rs`) loads a
string constant and returns it with no ownership mark. The bind's owning store,
`lower_value_owned` (`src/codegen/engine/value/builder_values.rs`), copies only
when `value_needs_owning_copy` says so. That predicate knows literals
(`static_string_value`), the rodata-returning Unicode lookups
(`call_returns_rodata_string`) and parameter borrows, but not
`toString(<Boolean>)`. So the binding stores the read-only pointer as if it were
its own, and `try_inplace_concat_assign` → `lower_string_self_append_one`
(`src/codegen/collection/assign/builder_inplace_assign.rs`) regrows and
`arena_free`s it.

## Goal

- The reproduction prints `TRUE!` and exits 0; the same with `FALSE`.
- Every producer above hands an owning store a block the store may own: the four
  regression fixtures print their expected output and exit 0.

### Non-goals (must NOT change)

- `toString(<Boolean>)`'s output text.
- Do not fix it by making the in-place append skip regrow-frees in general:
  that would leak every genuine buffer.

## Blast Radius

- `lower_boolean_to_string` — fixed by this bug.
- `toString(<enum>)` (plan-140-C) — unaffected: it returns a fresh marked copy
  by design, for exactly this reason.
- ~~Every other `toString` arm — unaffected: each returns a fresh arena block
  (helper call or `copy_flat_block`).~~ Wrong: the `String` arm is an identity (B).
  `.ai/codegen-invariants.md` already said so ("`toString(String)` is the IDENTITY
  arm").
- The `String` arm of `lower_to_string` (B), `lower_fs_path_dir_name` (C), the
  `StoreGlobal` default-value path (D).
- Other producers that return string constants without a mark — to audit:
  `grep -rn "emit_load_string_constant" src/codegen` for results returned as a
  call's value.

## Fix Design

Either (a) return a fresh copy from `lower_boolean_to_string` and
`mark_fresh_string` it, as plan-140-C does for enums (costs an allocation per
call, but no second predicate can disagree), or (b) teach
`value_needs_owning_copy` that `toString(<Boolean>)` is read-only (no
allocation, but it depends on the builder statically typing the argument as
`Boolean` at every owning site). Recommended: (a).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/rt-behavior/general/tostring_boolean_self_append` with the
      reproduction; confirm exit 138. → observed: no output, `[exit 138]`.
- [x] Complete the producer audit above. → "Scope extension": B, C, D found;
      `typeName`, literals, the enum arm, the error emitters and the default local
      are sound.
- [x] Add RED fixtures for B, C, D (`tostring_string_owning_store`,
      `pathdirname_constant_owned`, `string_global_default_owned`). →
      `scripts/test-accept.sh target/release/mfb target/accept-actual <4 names>` →
      `4 mismatch(es)`, each `[exit 138]`/`[exit 139]` against the expected output
      (hand-written in the golden `build.log`/`.run`).

Acceptance: the test fails for the documented reason.
Commit: b56b5bd6c

### Phase 2 — the fix

- [x] Apply the chosen design in `lower_boolean_to_string`. → (a): `copy_flat_block` + `mark_fresh_string`.
- [x] B: the `String` arm passes the block through only when it is the statement's
      own fresh temporary (`pending_temp_would_be_claimed`); otherwise it copies and
      marks the copy fresh.
- [x] C: route the `.`/`/` arms through the existing materialize path (the `/` is
      the path's own first byte), and mark the result fresh.
- [x] D: copy the default empty `String` into the arena in `StoreGlobal`'s
      no-value path, as the local `Bind` does.

Acceptance: the Phase 1 test passes. → `scripts/test-accept.sh target/release/mfb
target/accept-actual <the 4 fixtures>` → `acceptance tests passed (4 test(s) ran)`;
plan-143's runtime sweep (`/tmp/plan-143-probes/rt_run.sh`, 67 self-update forms ×
S1/S2/S9, `pathDirName` now on a path with no directory part) → 201/201 pass
(3 failed before: `toString(s)` at every site).
Commit: —

### Phase 3 — full validation

- [ ] `bash scripts/artifact-gate.sh target/release/mfb all`: diffs only where
      `toString(<Boolean>)` codegen changed, each inspected.
- [ ] `scripts/test-accept.sh target/debug/mfb target/accept-actual` → all pass.

Commit: —

## Summary

A one-arm ownership gap: a read-only result reaches an owning store that does
not recognize it. The risk is in the audit for other constant-returning
producers, not in the fix.
