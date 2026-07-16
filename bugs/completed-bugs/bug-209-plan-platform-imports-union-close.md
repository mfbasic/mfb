# bug-209: NativePlan.platform_imports omits resource-union variant close imports → link failure on valid source

Last updated: 2026-07-14
Effort: small (<1h)
Severity: MEDIUM
Class: correctness

Status: Fixed (2026-07-15) — platform_imports now mirrors the runtime_symbols resource-union block: for each bound all-resource union type it collects platform_imports_for_runtime_call for every variant's resource_close_function. The NirOp::Bind scan only resolves resource_close_function(type_), which is None for the union itself (its variants are the resources), so a scope-dropped resource union whose variant close needed a unique import previously failed the emitter's platform_imports lookup on valid source. Regression Test: verified on HW (VM 2228) — a `UNION Stream / AudioInput / AudioOutput` bound via audio::openOutput and scope-dropped builds, links, and runs (exit 0).
Regression Test: tests/ (a scope-dropped resource-union whose close needs a platform import links)

`platform_imports` has no resource-union-variant close handling, so a union drop
emits each variant's close runtime helper (via `runtime_symbols`) but never
collects that close's platform imports. The close helper code unit is emitted,
then the emitter does `platform_imports.get("_X").ok_or_else(...)` and fails the
build with "…requires _X import" on valid source (or leaves a dangling external
symbol).

The bare-resource `Bind` path handles this exact case explicitly
(`symbols.rs:262-269`, "e.g. audio's `_munmap`… links with it missing"); the
union path was left out.

## Failing Reproduction

A program that binds a resource-union (all variants resource types, e.g. a union
containing a `TlsConnection`/audio device) and relies on scope-drop (never an
explicit close), where a variant's close helper needs a libc/system import
(`_munmap`, tls teardown, etc.) that no other operation in the module pulls in.
Observed: build fails with "…requires _X import". Expected: the import is
collected and the build links.

## Root Cause

`src/target/shared/plan/symbols.rs:94-186` `platform_imports` — asymmetric with
`runtime_symbols` (`:16-40`), which has a resource-union block; `platform_imports`
does not.

## Non-goals

- Do not change the bare-resource path (already correct).

## Blast Radius

- `platform_imports` in `plan/symbols.rs`.

## Fix Design

In `platform_imports`, mirror the `runtime_symbols` union block: for each bound
all-resource union type, call `platform_imports_for_runtime_call(platform,
close)` for every variant's `resource_close_function` and `push_platform_import`
the results.
