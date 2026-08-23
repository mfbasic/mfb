# bug-451: binding `typeName(<package call>)` frees a rodata constant (SIGBUS at exit)

Last updated: 2026-08-22
Effort: small (one-line resolver realignment in the ownership classifier)
Severity: HIGH (valid program crashes uncatchably at scope exit — arena free-list
corruption writing into read-only memory)
Class: Correctness (memory-ownership; a valid program crashes)

Status: Completed
Regression Test: `tests/rt-behavior/general/func_typename_builtin_calls` — extended
with bound-variable cases (`LET x AS String = typeName(math::abs(f))`, …); its
`golden/build.log` now ends `[exit 0]` where the unfixed compiler produced
`[exit 138]` (SIGBUS).

## Symptom

```mfbasic
IMPORT io
IMPORT math
SUB main()
  LET t AS String = typeName(math::abs(0 - 1))   ' folds to "Integer"
  io::print(t)                                    ' prints "Integer" correctly
END SUB                                            ' <- SIGBUS here, exit 138
```

The program runs and prints the **correct** value, then dies at scope exit with
`EXC_BAD_ACCESS (code=2)` — a *write* fault at a low, image-mapped address
(rodata). The faulting instruction is the arena free-list's intrusive-`next`
store (`str x13, [x8]` with `x8` pointing into `__DATA_CONST`): scope-drop is
`arena_free`-ing a **read-only string constant**.

Only surfaces when the folded string is **bound to (or otherwise owned by) a
local** — the inline `io::print(typeName(math::abs(x)))` form never binds it, so
it never frees it. And only for a **package / effectful call** argument:
`typeName(len(...))`, `typeName(toString(...))`, `typeName(5+5)`, and plain
string literals were all safe.

## Root cause

`typeName(<call>)` is folded to a static `String` constant at codegen by the
builder via `static_type_name_for_fold` (→ `builtins::resolve_call_return_type`,
which resolves *any* call's return type). That constant lives in rodata.

The ownership classifier `value_needs_owning_copy` decides whether a bound value
is a rodata constant that must be **deep-copied into the arena** before a binding
can own (and later free) it. It consults `static_string_value`, whose `typeName`
arm folded through the **coarser** `static_type_name` — whose call arm only
recognizes a hand-written builtin list (`len`, `toString`, `find`, …). A package
call such as `math.abs` / `collections.find` / `io.isBuffered` is not in that
list, so `static_string_value` returned `None`, `value_needs_owning_copy`
returned `false`, and the bind stored the **rodata pointer directly** with no
copy. Scope-drop then `arena_free`d the read-only constant.

The two fold paths disagreed: the *builder* folded the value (rodata constant),
but the *ownership classifier* did not recognize the fold, so it skipped the
mandatory owning copy. `typeName(len(...))` worked only because `len` happens to
be in `static_type_name`'s list. This is the exact fold-mismatch class already
documented in `static_string_value` for the `strings::` package folds
(`caseFold("HELLO")` etc.).

## Fix

`src/codegen/memory/value/builder_value_semantics.rs` — `static_string_value`'s
`typeName` arm now folds through `static_type_name_for_fold` (the same resolver
the builder folds with) instead of `static_type_name`. The classifier's fold now
matches the builder's exactly: whatever the builder materializes as a rodata
constant is recognized as one and deep-copied at the owning store, so scope-drop
frees a real arena block.

## Verification

- Standalone repro: `LET t = typeName(math::abs(0-1))` / `typeName(io::*)` /
  `typeName(collections::*)` bound to a local — all exit 0 after the fix, value
  still correct (`Integer` / `Boolean` / …).
- `cargo test --bin mfb`: 3614 passed.
- Regression fixture gate green; `build.log` `[exit 0]` (was `[exit 138]`).
