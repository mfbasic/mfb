# plan-128-A: CLI argument-source contract

Last updated: 2026-09-09
Overall Effort: large (3h–1d) — the whole plan-128 CLI package feature
Effort: medium (1h–2h)
Depends on: nothing

This sub-plan supplies the two host-argument guarantees that a source package
cannot create for itself: an invocation-program name and rejection of malformed
UTF-8 argument bytes. The resulting `os` surface gives `packages/cli` a truthful
input boundary before it parses switches; the package itself is deliberately
out of scope for this letter.

References:

- `mfb spec stdlib os` — current `os::args` and `os::executablePath` contract.
- `mfb spec language error-model` — entry argument vector includes the program
  name, whereas `os::args` does not.
- `.ai/compiler.md`, `.ai/arch-abi.md`, and `.ai/testing-gates.md` — required
  runtime/codegen and artifact verification rules.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| The caller has accepted the API decisions in `plan-128-B` § Open Decisions (the source package consumes these semantics). | `test -f planning/plan-128-B-cli-package.md` | MET |

Everything below is written against the current registry-based `os` package.

## 1. Goal

- `os::prog() AS String` returns the program name exactly as passed in
  `argv[0]`, and `os::args()` either returns a list of valid UTF-8 arguments
  after it or raises `errorCode::ErrEncoding`; no malformed host bytes reach a
  MFBASIC `String`.

### Non-goals

- Do not change `os::args()` to include `argv[0]`; its existing public contract
  remains arguments-after-program-name.
- Do not redefine `os::executablePath()` as an invocation name; it remains the
  absolute executable path.
- Do not add CLI switch parsing or package state in this letter.
- Do not change the entry-function `args AS List OF String` contract without a
  separately approved language-surface plan.

## 2. Current State

- `src/codegen/builtins/os/func_args.rs:lower_args` starts at index `1`, copies
  raw NUL-terminated `argv` bytes directly into a `String`, and performs no
  UTF-8 validation. `src/docs/spec/stdlib/14_os.md` consequently promises the
  list after the program name.
- `src/codegen/builtins/os/func_executable_path.rs:lower_executable_path`
  obtains an absolute host path, which is observably different from `argv[0]`.
- `src/codegen/engine/function/entry.rs:lower_program_entry` captures the raw
  argc/argv pair into `OS_ARGC_GLOBAL_SYMBOL` and `OS_ARGV_GLOBAL_SYMBOL` before
  user code; `src/codegen/builtins/os/gen_shared.rs` exposes those symbols.
- `errorCode::ErrEncoding` is the established invalid-UTF-8 error
  (`src/codegen/builtins/mod.rs:inline_builtin_fallibility_depends_on_args`; the
  registry defines byte-list-to-String decoding as fallible).

### Measured populations

| What | Count | Command |
|---|---:|---|
| Existing source-package directories | 6 | `find packages -mindepth 1 -maxdepth 1 -type d -print \| sort \| wc -l` |
| Existing package source files | 50 | `find packages -path '*/src/*.mfb' -type f \| wc -l` |
| Existing package TESTING files | 22 | `rg -l '^TESTING$' packages -g '*.mfb' \| wc -l` |
| Existing `os` registry functions | 19 | `rg -n 'assert_eq!\\(pkg.functions\\(\\).len\\(\\), 19\\)' src/codegen/builtins/os/mod.rs \| wc -l` |

### Verified properties

- An entry function with `args AS List OF String` has a different source-level
  vector: element zero is the host program name. Verified by reading `mfb spec
  language error-model` and `src/docs/spec/language/08_error-model.md` § entry
  arguments; this is not `os::args()`.
- A pure source package cannot inspect raw argument bytes after `os::args()`
  returns. Verified by reading `func_args.rs:lower_args`: the raw pointer walk
  and String materialization occur entirely in generated native code.

## 3. Design Overview

Add a shared UTF-8-copy helper in `src/codegen/builtins/os/` that scans each
NUL-terminated argument before materializing it. `args` uses that helper for
indices `1..argc`; `prog` uses it for index `0`. Both calls use the existing
entry-captured globals, so neither re-reads a process-global argv nor changes
startup ordering. A malformed sequence returns `ErrEncoding` with an `os`
error result rather than constructing an invalid String.

Correctness risk concentrates in the byte scanner and its register lifetime
across arena allocation, then in preserving the same behavior on all five
native targets. This is behavior-changing work, so byte identity is not its
correctness gate: the `os` byte-identity fixture is expected to differ, while
unrelated fixtures must remain unchanged.

Rejected alternative: implement `cli::prog` over `os::executablePath`. It loses
the invocation spelling (relative path, symlink, shell spelling) and cannot
meet the requested program-name contract. Rejected alternative: validate in
`packages/cli`; by then invalid bytes have already been represented as a
String, violating String's Unicode invariant.

## 4. Detailed Design

### 4.1 UTF-8 boundary

The helper validates each complete byte sequence while calculating its length;
it rejects truncated sequences, bad continuation bytes, overlong encodings,
surrogates, and values above U+10FFFF. It must allocate/copy only after a
successful scan. `os::args()` fails if any user argument is malformed;
`os::prog()` fails if `argv[0]` is malformed. The first discovered malformed
entry terminates the operation with `ErrEncoding`.

### 4.2 Program name

Register `prog` beside `args` in `src/codegen/builtins/os/mod.rs`, give it the
nullary `String` registry descriptor and documentation, and lower it from
`argv[0]`. An empty argument vector is an impossible normal host condition; it
must use the existing runtime error discipline rather than return a fabricated
empty String. The implementation must document and test that `prog` is the
invocation name, not the resolved executable location.

## Phases

### Phase 1 — Specify and validate raw argv materialization

Land the shared scanner/copy primitive and test it before exposing `prog`.

- [ ] Add the UTF-8 validation-and-copy emitter in
  `src/codegen/builtins/os/gen_shared.rs` (or a focused sibling), returning the
  existing `ErrEncoding` result without allocating a malformed String.
- [ ] Refactor `src/codegen/builtins/os/func_args.rs:lower_args` to call the
  shared primitive for every index after zero while preserving its empty-list
  and argument-order behavior.
- [ ] Add valid and error runtime fixtures under `tests/rt-behavior/os/` and
  `tests/rt-error/os/`; use a host-side executable/runner fixture capable of
  passing crafted non-UTF-8 argv bytes on Unix, and a Windows wide-argument
  case proving valid non-ASCII input remains accepted.
- [ ] Update the `os::args` registry prose and `src/docs/spec/stdlib/14_os.md`
  with the `ErrEncoding` boundary.

Acceptance: valid ASCII and multibyte arguments round-trip in order; every
crafted malformed UTF-8 category raises `ErrEncoding`; `os::args()` with no
arguments still returns an empty list.
Commit: —

### Phase 2 — Add and prove `os::prog`

Expose the program-name accessor once the shared boundary is reliable.

- [ ] Add `src/codegen/builtins/os/func_prog.rs` with registry descriptor,
  fallibility declaration, documentation, and a lowering built on the shared
  index-zero path.
- [ ] Register `func_prog` in `src/codegen/builtins/os/mod.rs`; update the
  registry-count/parity tests, `BUILTIN_IMPORTS`-adjacent metadata only if the
  actual registry seam requires it, and the `os` package/spec/man prose.
- [ ] Add `tests/rt-behavior/os/func_os_prog_valid/` proving an invocation name
  chosen by the runner is returned verbatim and differs from
  `os::executablePath()`; add `tests/syntax/os/func_os_prog_invalid/` for
  arity/type diagnostics.

Acceptance: a compiled program prints the supplied argv[0] spelling, not its
absolute executable path; invalid calls report the standard builtin argument
diagnostic; generated binaries execute correctly on each locally buildable
target.
Commit: —

### Phase 3 — Cross-target and full compiler gates

Finish the codegen change with all required checks and precise golden handling.

- [ ] Run `cargo test --bin mfb` and the targeted `os` artifact gate; inspect
  one expected `os` `.ncode` diff with objdump before regenerating only the
  affected target goldens.
- [ ] Run `scripts/test-accept.sh target/debug/mfb target/accept-actual` and
  `scripts/artifact-gate.sh target/debug/mfb all`; investigate every unexpected
  artifact or runtime difference rather than rebasing it.
- [ ] Run the native `os` argument fixtures on available Linux/macOS/Windows
  machines, recording any platform that cannot pass malformed raw argv as a
  harness limitation rather than silently omitting it.

Acceptance: full acceptance and artifact gates pass, runtime evidence covers
the raw-argv path, and only expected `os` target artifacts have changed.
Commit: —

## Validation Plan

- Tests: valid and malformed argv runtime fixtures, `prog` syntax fixtures,
  registry tests, and `os` man/spec rendering.
- Coverage check: confirm the new `prog` fixture is in the `os` corpus using
  `rg -n 'func_os_prog' src/codegen/builtins/tests tests` before treating a
  green suite as coverage.
- Runtime proof: invoke an emitted binary with a relative/symlink argv[0],
  valid Unicode args, and crafted malformed Unix bytes; verify stdout/error and
  exit code.
- Doc sync: `src/docs/spec/stdlib/14_os.md`, package registry prose, and
  rendered `mfb man os`/`mfb man os prog`.
- Acceptance: `cargo test --bin mfb`, `scripts/test-accept.sh target/debug/mfb
  target/accept-actual`, and `scripts/artifact-gate.sh target/debug/mfb all`.

## Open Decisions

- Empty host argv behavior — recommend `ErrInvalidArgument` rather than `""`.
  It preserves the guarantee that a successful `prog` is a real argv element;
  confirm the project’s existing entry-error convention before coding.
- Windows malformed Unicode test mechanism — recommend a platform-specific
  runner that documents Windows' UTF-16 command-line boundary; Windows cannot
  receive arbitrary Unix-style raw bytes.

## Corrections

None yet.

## Summary

This letter is the small compiler/runtime foundation needed for an honest CLI
package. It preserves existing `os::args` indexing and confines the expected
codegen movement to programs using `os` argument access.
