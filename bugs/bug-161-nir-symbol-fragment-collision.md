# bug-161 — `symbol_fragment` collapses `$`/space/`_` alike, so distinct monomorphized names can collide on one linker symbol

Last updated: 2026-07-12
Severity: MEDIUM — link-time symbol aliasing → wrong function/global silently shadows another.
Class: Correctness.
Status: Open

## Finding

`src/target/shared/nir/symbols.rs:23` (`symbol_fragment`, used by
`function_symbol:1` and `global_symbol:11`). The monomorphizer mangles overloaded
names as `name$Type$Type` and sanitizes type-name spaces to `$`
(`src/monomorph/helpers.rs:364,449`). `symbol_fragment` then maps every
non-alphanumeric char (including `$`) to `_` while passing a literal `_`
through. So the *distinct* IR names `f$List$OF$Integer` (from an overloaded
`f(List OF Integer)`) and a separate user function `f_List_OF_Integer()` both
become `_mfb_fn_f_List_OF_Integer`. Two different IR functions resolve to the
same defined symbol → the linker silently keeps one → the wrong function
executes. This is the exact collision class already fixed for
`link_thunk_symbol` (bug-139.6) via `_XX` hex-escaping; `function_symbol`/
`global_symbol` were left on the lossy mapping.

## Trigger

A program defining both an overloaded `f(List OF Integer)` (mangled
`f$List$OF$Integer`) and a plain function/global literally named
`f_List_OF_Integer`. Both mangle to the same `_mfb_fn_f_List_OF_Integer`; one
shadows the other at link time with no diagnostic (the IR names are genuinely
distinct, so no earlier stage catches it). Confidence medium: requires a user to
pick the underscore-form name that matches a mangled overload.

## Fix

Escape non-alphanumeric bytes (including `_`) to an unambiguous `_XX` hex form in
`symbol_fragment`, matching `link_thunk_symbol`'s `escape`. Add a test with the
colliding pair asserting two distinct emitted symbols.
