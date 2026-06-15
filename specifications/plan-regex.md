# MFBASIC Regex Implementation Plan

Last updated: 2026-06-14

This document plans how to implement the `regex` package specified in
`specifications/standard_package.md` with identical behavior across current and
future targets.

It complements:

- `specifications/standard_package.md`
- `specifications/architecture.md`
- `specifications/package_format.md`
- `specifications/linker.md`

## 1. Goal

Implement:

- `regex::match(value AS String, pattern AS String) AS Boolean`
- `regex::find(value AS String, pattern AS String, start AS Integer = 0) AS Integer`
- `regex::replace(value AS String, pattern AS String, replacement AS String) AS String`

while preserving one semantic contract across:

- native executable backends
- package bytecode output
- current targets (`macos-aarch64`, `linux-aarch64`)
- future targets

The key constraint is that regex behavior must not depend on host libc, host
OS, or target-specific regex quirks. The compatibility target is Rust
`regex`-style behavior.

## 2. Current State

The spec already defines the package in
`specifications/standard_package.md:198`:

- invalid patterns fail with `ErrInvalidFormat`
- `match` succeeds when the pattern matches anywhere in the string
- `find` returns a zero-based Unicode scalar index
- `replace` replaces all matches

The compiler/runtime state is incomplete:

- `src/builtins/mod.rs` does not register a `regex` built-in package.
- `src/builtins/strings.rs` does not expose regex calls.
- native backend capability lists do not advertise regex runtime calls.
- shared runtime helper metadata in `src/target/shared/runtime.rs` has no regex
  helper family.
- bytecode already reserves regex opcodes in `src/bytecode.rs`:
  `OPCODE_STRING_REGEX_MATCH`, `OPCODE_STRING_REGEX_FIND`,
  `OPCODE_STRING_REGEX_REPLACE`.

There is also an important trap in the old backend:

- `src/arch/aarch64_old/mod.rs` routes regex opcodes through existing
  substring-style helpers (`emit_string_predicate`, `emit_general_find`,
  `emit_general_replace`).

That old path is not acceptable as a model because it can make regex appear
implemented while silently behaving like plain substring search or replace.

## 3. Why Host `regcomp()` Alone Is Not Enough

Using the platform's native regex API directly is the wrong default if the goal
is target-stable behavior.

Problems:

- `regcomp()` / `regexec()` semantics vary across libc implementations.
- Linux currently emits both glibc and musl executables; those must not drift.
- macOS uses a different libc and may differ again.
- future targets such as Windows would require a different regex API entirely.
- character class, newline, locale, backtracking, and replacement behavior can
  diverge even when APIs look similar.

Therefore:

- do not define MFBASIC regex semantics as "whatever the host regex library
  does"
- do not make source compatibility depend on the selected target
- do not ship regex by adding per-target behavior branches

## 4. Recommendation

Implement Rust `regex`-style semantics with one parser and one executor shared
across all backends.

Recommended architecture:

1. Define the MFBASIC regex dialect in the spec as Rust `regex` style.
2. Parse patterns into a compiler-owned regex IR over Unicode scalars.
3. Execute that IR with one shared engine used by:
   - bytecode generation/runtime semantics
   - native runtime helpers
   - any future interpreter or JIT
4. Keep target-specific code limited to:
   - string storage access
   - allocation
   - error/result ABI glue

This keeps semantics centralized and makes new targets implement a thin host
adapter rather than a new regex flavor.

## 5. `regcomp()` / `regexec()` / `regfree()` Role

Those APIs can still be useful, but only in a constrained way.

Acceptable uses:

- as an internal development reference for differential testing
- behind a compiler-owned compatibility wrapper when the exact same vendored
  regex implementation is embedded for every target

Not acceptable:

- calling the host libc regex functions directly as the semantic source of truth
- allowing glibc, musl, and Darwin to define different matching behavior

If a vendored engine is preferred, it must be chosen because it can reproduce
Rust `regex`-style behavior across targets. In that design,
`regcomp()`-style functions are implementation detail, not the language
contract.

## 6. Semantic Boundary To Standardize

Before implementation, the spec should stop saying the dialect is merely
"runtime-defined" and instead define a compiler-owned stable dialect aligned to
Rust `regex` style.

At minimum, the dialect must answer:

- that syntax compatibility targets Rust `regex` style, not POSIX ERE/BRE
- whether matching operates on Unicode scalars or UTF-8 bytes
- how `.` behaves on multi-byte Unicode
- supported escapes
- supported quantifiers
- support for alternation and grouping
- support or non-support for capture groups
- backreferences are not supported
- support or non-support for character classes
- whether anchors `^` and `$` are supported
- whether replacement strings support the Rust `regex`-style replacement forms
  chosen for MFBASIC
- whether zero-length matches are allowed and how global replace advances

Recommendation for the first release:

- define matching over Unicode scalar sequences, not bytes
- follow Rust `regex`-style syntax where feasible
- exclude backreferences for v1
- exclude look-around for v1
- exclude target-locale-sensitive features
- define exactly which Rust `regex`-style replacement features are exposed

This matches the rest of the string package, which already specifies scalar
indexes rather than byte offsets.

Practical reading of "Rust `regex` style":

- character classes, alternation, grouping, repetition, anchors, and escapes
  should behave like Rust `regex`
- unsupported Rust `regex` features should be rejected consistently with
  `ErrInvalidFormat`, not implemented differently per target
- host `regcomp()` behavior is irrelevant unless hidden behind a compatibility
  layer that reproduces the same semantics

## 7. Proposed Internal Architecture

Use a layered design:

```text
source regex::*
  -> typecheck builtin package call
  -> compiler-owned regex built-in lowering
  -> shared regex semantic layer
  -> target/package adapters
```

### 7.1 Front-End Layer

Add a new built-in module, for example:

- `src/builtins/regex.rs`

Responsibilities:

- recognize `regex` as a built-in import
- resolve call signatures and arity
- expose return types
- lower regex calls into bytecode operations

`src/builtins/mod.rs` must be updated so `regex` participates in:

- `is_builtin_import`
- `call_return_type_name`
- `is_builtin_call`

### 7.2 Shared Regex Core

Add a target-independent regex core, for example:

- `src/regex/parse.rs`
- `src/regex/ir.rs`
- `src/regex/exec.rs`

Responsibilities:

- pattern parsing
- pattern validation
- compiled regex representation
- matching over Unicode scalar streams
- first-match search
- global replacement logic
- reproducing Rust `regex`-style behavior consistently across targets

The core should define one internal API such as:

```text
compile(pattern) -> Result<CompiledRegex, ErrInvalidFormat>
is_match(compiled, value) -> bool
find(compiled, value, start_scalar) -> Result<MatchRange, ErrNotFound>
replace_all(compiled, value, replacement) -> String
```

`MatchRange` should be scalar-index based, not byte-offset based.

### 7.3 Bytecode Layer

The bytecode path already has opcode numbers reserved. Finish that path instead
of inventing a separate regex lowering design.

Required work:

- teach `src/builtins/regex.rs` to lower source calls to the existing regex
  opcodes
- ensure bytecode serialization/deserialization treats those opcodes as normal
  supported instructions
- ensure any runtime that executes package bytecode uses the shared regex core

Important rule:

- package semantics and native executable semantics must share the same regex
  engine behavior

If package bytecode cannot execute regex yet, that is a blocker, not a reason
to implement regex only for native builds.

### 7.4 Native Layer

For native backends, add a new runtime helper family, for example `Regex`, in
`src/target/shared/runtime.rs`.

Add helper specs for:

- `regex.match`
- `regex.find`
- `regex.replace`

Then update:

- `helper_for_call`
- `supported_helper_specs`
- `is_native_direct_call` if any operation is emitted inline
- backend capability lists in `src/target/macos_aarch64/mod.rs` and
  `src/target/linux_aarch64/mod.rs`
- target-specific import planning only if truly needed

Recommendation:

- do not add libc regex imports to target plans
- emit shared runtime helpers that call the compiler-owned regex core

That keeps `src/target/macos_aarch64/plan.rs` and
`src/target/linux_aarch64/plan.rs` free of regex-library divergence.

## 8. Matching Model

The core engine should operate on Unicode scalar indexes even if strings remain
stored as UTF-8.

Implementation shape:

1. Validate `start` for `regex::find` as a scalar index.
2. Convert the UTF-8 string to a scalar cursor view, not necessarily a copied
   scalar array.
3. Run the regex engine on scalar positions.
4. Return scalar indexes to user code.
5. Convert scalar ranges back to byte slices only when building output strings.

This is more work than byte-oriented regex, but it is the only design that
naturally matches the current standard package contract.

## 9. Compilation And Caching

`regex::match`, `regex::find`, and `regex::replace` accept patterns as runtime
strings, so patterns cannot be assumed constant.

Recommended execution strategy:

- compile the pattern on each call initially
- optionally add constant-pattern folding later
- optionally add per-call-site caching later if profiling justifies it

Do not make caching part of semantic correctness.

A safe optimization path is:

- if pattern is a compile-time constant, pre-parse it in native lowering or
  bytecode generation
- otherwise parse at runtime using the same shared parser

Both paths must produce identical compiled regex IR.

## 10. Error Behavior

Required error behavior:

- invalid pattern -> `ErrInvalidFormat`
- `regex::find` no match -> `ErrNotFound`
- invalid `start` index -> same out-of-range behavior used by other scalar index
  APIs

Do not map malformed patterns to:

- `FALSE`
- `-1`
- unchanged source string
- target-specific errno-style errors

The language contract must stay explicit and portable.

## 11. Validation Plan

Validation must prove identical behavior across build paths.

### 11.1 Function Tests

Add mandatory valid and invalid function test directories for every new
function:

- `tests/func_regex_match_valid/**`
- `tests/func_regex_match_invalid/**`
- `tests/func_regex_find_valid/**`
- `tests/func_regex_find_invalid/**`
- `tests/func_regex_replace_valid/**`
- `tests/func_regex_replace_invalid/**`

Coverage should include:

- successful literal matches
- alternation/grouping/quantifier cases in the chosen dialect
- Unicode scalar indexing behavior
- zero-length and boundary cases
- invalid patterns
- wrong arity
- wrong argument types
- out-of-range `start`
- `ErrNotFound` for absent matches

### 11.2 Runtime Proofs

Because regex is a runtime feature, passing compilation is not enough.

Add runtime tests that execute generated programs and prove:

- native executable regex behavior
- package bytecode regex behavior when imported into an executable
- identical observable results on current targets

For Linux specifically, ensure both emitted flavors agree:

- glibc output
- musl output

### 11.3 Acceptance Suite

After implementation:

- run `scripts/test-accept.sh target/debug/mfb target/accept-actual`

Regex work should not be considered complete until acceptance passes and regex
runtime behavior is demonstrated end-to-end.

## 12. Recommended Implementation Sequence

1. Tighten the spec from "runtime-defined" to Rust `regex`-style,
   compiler-owned regex semantics.
2. Add `src/builtins/regex.rs` and register the package in
   `src/builtins/mod.rs`.
3. Finish bytecode lowering for regex using the existing reserved opcode space.
4. Implement the shared regex parser/IR/executor.
5. Wire bytecode/package execution to that shared core.
6. Add native runtime helper metadata and lowering for regex calls.
7. Keep platform plan/code changes minimal and free of regex-library-specific
   imports unless a vendored engine absolutely requires them.
8. Add valid and invalid function tests plus runtime proofs.
9. Run acceptance and compare results across targets.

## 13. Non-Goals For V1

The first implementation does not need:

- PCRE compatibility
- locale-sensitive regex behavior
- target-specific regex extensions
- backreferences
- lookahead/lookbehind
- JIT compilation
- heuristic behavior differences by platform

These can be evaluated later, but only if they remain compatible with the
single shared semantic contract.

## 14. Bottom Line

If consistency across targets is the priority, the implementation should not
be based on each target's native `regcomp()` behavior.

The correct plan is:

- one MFBASIC-defined regex dialect aligned to Rust `regex` style
- one compiler-owned regex parser/executor
- one semantic contract used by both package bytecode and native backends
- thin target adapters only for allocation, ABI, and string storage

If `regcomp()` / `regexec()` / `regfree()` appear anywhere, they should be
behind that shared contract, never the contract itself.
