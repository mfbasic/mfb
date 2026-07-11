# bug-120 — runtime-specs LOW cluster: dead strings_specs.rs; clobber lists understate the real set

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, runtime-specs slice). Two
independent LOW findings in `src/target/shared/runtime/`, batched per goal-02.

## 1. strings_specs.rs is entirely dead — 14 specs for helpers never emitted or called (dead-code)

`src/target/shared/runtime/strings_specs.rs:37-189` (all `STRINGS_*_SPEC`) plus
their catalog.rs:98-111 entries. Every `strings.*` call is native-direct
(`is_native_direct_call` usage.rs:73-100, or `native_builtin_target` for
find/mid/replace) and lowers fully inline in builder_strings_package.rs /
builder_strings*.rs. A tree-wide grep (src + tests) finds zero `_mfb_rt_strings_`
outside strings_specs.rs itself: no caller stages such a call, no emitter
produces such a body. The specs, their catalog entries, and effectively
`helper_for_call`'s Strings routing are unreachable except via
`supported_helper_specs()` iteration. Misleads readers into thinking strings
runs through runtime helpers. Fix: delete strings_specs.rs and its catalog
entries (and the dead Strings routing), or document why they're retained.

## 2. `clobbers: abi::IO_PRINT_CLOBBERS` on all 151 specs understates the real clobber set (footgun, latent)

`src/target/shared/runtime/io_specs.rs:74-79` (representative; the constant is
`src/target/shared/abi.rs:3`, used by every spec in all 10 files).
`IO_PRINT_CLOBBERS = ["x0","x1","x2","x9","x16"]`, but every `bl _mfb_rt_*`
helper destroys the full caller-saved file (x0–x17 plus v0–v7 — the helpers
call pthread/libc and stage SCRATCH/FP_SCRATCH; this repo's established rule is
"_mfb_* helper calls destroy ALL x0-x17"). The bug-70 comment claims the value
is "truthful", but it is only *non-empty*: a future per-call clobber reader
would conclude x3–x8, x10–x15, x17 survive the call and keep live values there —
a miscompile. Latent today because the only consumer is the
`!clobbers.is_empty()` gate (validate.rs:214). Fix: set the spec clobber list to
the full caller-saved set (or derive it), so a future reader can't trust the
short list.

## Verified-clean cross-checks (noted for the census)

All 151 spec `symbol` strings mechanically match `symbol_for_call`; no
duplicate catalog entries; catalog == the 151 defined specs; caller-side
default-arg padding matches every spec's declared arity; tls.listen's 5th arg
(backlog) is really read by the openssl listener; usage.rs's IrOp/IrValue
traversal is otherwise exhaustive (the MATCH-guard gap = bug-118 is the one
hole).
