# MFBASIC Regex Implementation Plan

Last updated: 2026-06-19

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
- Binary Representation (MFP package) output
- current targets (`macos-aarch64`, `linux-aarch64`)
- future targets

The key constraint is that regex behavior must not depend on host libc, host
OS, or target-specific regex quirks. The compatibility target is Rust
`regex`-style behavior.

## 2. Current State

The spec already defines the package in
`specifications/standard_package.md` section 6:

- invalid patterns fail with `ErrInvalidFormat`
- `match` succeeds when the pattern matches anywhere in the string
- `find` returns a zero-based Unicode scalar index
- `replace` replaces all matches

The compiler/runtime state is that no regex implementation exists yet — nothing
is reserved or stubbed:

- `src/builtins/mod.rs` does not register a `regex` built-in package
  (`is_builtin_import` lists only `fs | io | json | math | strings | thread`).
- there is no `src/builtins/regex.rs` module for regex call signatures and
  lowering, and no `src/regex/` engine module.
- native backend capability lists (the `runtime_calls` arrays in
  `src/target/macos_aarch64/mod.rs` and `src/target/linux_aarch64/mod.rs`) do not
  advertise regex runtime calls.
- shared runtime helper metadata in `src/target/shared/runtime.rs` has no regex
  helper family. The `RuntimeHelper` enum currently contains only
  `Fs | General | Io | Math | Strings | Thread`.
- there are **no** reserved regex opcodes anywhere. The earlier flat
  bytecode/opcode model has been removed: `src/bytecode.rs` no longer exists.
  Packages are now emitted as a structure-preserving **Binary Representation**
  (`src/binary_repr.rs`, `MFBR`/`MFPC` v2), which is a serialization of the IR,
  not an opcode stream. `grep -ri regex src/` returns nothing.

A consequence of the Binary Representation model removes the old "two engines"
risk entirely: a package is **decoded straight back into IR** and lowered through
the same `IR → NIR → native` codegen used for the executable's own code (see
`specifications/architecture.md` §11). There is no separate package interpreter,
so regex semantics cannot diverge between "package" and "native" paths by
construction — both go through one code path.

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
   - Binary Representation (package) generation
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

The current standard-package spec already defines regex as a compiler-owned,
target-stable dialect aligned to Rust `regex` style. Before implementation, the
remaining work is to make the exact supported subset explicit enough for tests
and diagnostics.

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
- lower regex calls into IR (and, for a native helper, runtime-helper calls)

`src/builtins/mod.rs` must be updated so `regex` participates in:

- `is_builtin_import`
- `call_return_type_name`
- `is_builtin_call`
- `call_param_names` (for named-argument resolution)

Two existing built-in packages bracket the design choice, and the regex module
should mirror whichever one matches the implementation strategy chosen in §7.2:

- **Native-helper package** (`fs`, `io`, `math`, `strings`, `thread`): call
  signatures live in a `src/builtins/<name>.rs` module; the operations are
  lowered to native code by `CodeBuilder` and registered as runtime helpers
  (§7.4). There is no linked runtime — the compiler emits the helper bodies.
- **MFBASIC-source package** (`json`): the entire package is written in MFBASIC
  in `src/builtins/json_package.mfb`, parsed via `builtins::json::source_file`,
  and injected through `builtins::json::augmented_project`, which is called from
  both `src/resolver.rs` and `src/typecheck.rs`. Such a package needs no runtime
  helper and no codegen changes — it lowers to ordinary IR like user code.

### 7.2 Shared Regex Core

Whatever the strategy, there must be exactly one definition of regex semantics.
The current architecture has **no linked runtime library**: native helpers are
hand-emitted code generated by `CodeBuilder` (see
`src/target/shared/code/builder_strings_package.rs`), and packages decode back to
IR rather than executing a separate engine. So "one shared engine" must be
realized as one of the following, not as a Rust crate linked into a runtime:

1. **MFBASIC-source engine (recommended).** Write the parser, matcher, and
   replacement logic in MFBASIC as `src/builtins/regex_package.mfb`, following
   the `json` precedent. Because patterns are runtime strings, the engine
   parses and matches at runtime. This gives target stability for free: the
   engine compiles through the single `IR → NIR → native` codegen and is
   serialized into the Binary Representation like any other code. `json` already
   demonstrates that a non-trivial parser is expressible this way.

2. **Compiler-owned Rust core driving native lowering.** Add
   `src/regex/{parse,ir,exec}.rs`. At compile time this can validate patterns and
   fold constant patterns, but because patterns may be runtime values, the
   *matching* engine still has to be emitted as native code by `CodeBuilder` (a
   regex NFA/program interpreter generated into the program). This is
   substantially more work than option 1 and is only justified if regex becomes
   performance-critical.

If a Rust core is introduced, it should expose one internal API such as:

```text
compile(pattern) -> Result<CompiledRegex, ErrInvalidFormat>
is_match(compiled, value) -> bool
find(compiled, value, start_scalar) -> Result<MatchRange, ErrNotFound>
replace_all(compiled, value, replacement) -> String
```

`MatchRange` should be scalar-index based, not byte-offset based.

Responsibilities of the core, regardless of strategy:

- pattern parsing and validation
- compiled regex representation
- matching over Unicode scalar streams
- first-match search
- global replacement logic
- reproducing Rust `regex`-style behavior consistently across targets

### 7.3 Binary Representation (Package) Layer

There is no opcode path to "finish" — the flat bytecode model is gone. The Binary
Representation in `src/binary_repr.rs` is a structure-preserving serialization of
the IR (`MFBR`/`MFPC` v2), and packages are decoded back into IR and lowered
through the same codegen as the executable's own code.

What this means for regex:

- If regex is an **MFBASIC-source package** (§7.2 option 1), no Binary
  Representation work is required at all. The engine lowers to ordinary IR
  (`Call` / `CallResult` nodes over user-level constructs), which the existing
  encoder/decoder already handles. Nothing regex-specific touches
  `src/binary_repr.rs`.
- If regex is a **native runtime helper** (§7.2 option 2), the only Binary
  Representation surface is the `runtime_helpers: Vec<RuntimeHelper>` list a
  package records (see `src/target/shared/nir.rs`). Adding a `Regex` variant to
  the `RuntimeHelper` enum makes it serialize like the existing `Strings`/`Fs`
  helpers; confirm the enum's encode/decode in `src/binary_repr.rs` covers the
  new variant.

Important rule (now structural, not aspirational):

- package semantics and native executable semantics share one regex
  implementation automatically, because a package is decoded to IR and lowered
  through the single `IR → NIR → native` path. There is no separate package
  interpreter that could drift.

### 7.4 Native Layer

This layer applies only if regex is implemented as a native runtime helper
(§7.2 option 2); an MFBASIC-source package needs none of it.

For native backends, add a new runtime helper family, for example `Regex`, to the
`RuntimeHelper` enum in `src/target/shared/runtime.rs`, and emit the operation
bodies from a new `src/target/shared/code/builder_regex_package.rs` (mirroring
`builder_strings_package.rs`), since helper bodies are generated inline rather
than linked from a runtime library.

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

- if the pattern is a compile-time constant, pre-parse/fold it during lowering
- otherwise parse at runtime using the same shared parser

Both paths must produce identical results. `builder_strings_package.rs` already
does this kind of constant folding for the `strings` package (e.g.
`static_strings_package_string` and the constant-string `graphemes` path), so a
constant-pattern fast path would follow an established pattern rather than a new
mechanism.

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
- Binary Representation package regex behavior when an MFP package is imported
  into an executable
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

1. Confirm the standard-package regex subset is specific enough for tests,
   diagnostics, and implementation.
2. Choose the implementation strategy (§7.2): MFBASIC-source package
   (recommended) or native runtime helper.
3. Register the package in `src/builtins/mod.rs` (and, for a source package,
   add `regex_package.mfb` plus an `augmented_project` injection mirroring
   `json`).
4. Implement the regex parser/matcher/replacement — in MFBASIC for a source
   package, or as `src/regex/*` plus `CodeBuilder` emission for a native helper.
5. (Native-helper strategy only) add the `Regex` `RuntimeHelper` variant, helper
   specs, `helper_for_call`/`supported_helper_specs`/`is_native_direct_call`
   wiring, and the target `runtime_calls` capability arrays. Confirm the new
   `RuntimeHelper` variant round-trips through `src/binary_repr.rs`.
6. Keep platform plan/code changes minimal and free of regex-library-specific
   imports; do not add host `regcomp` imports to target plans.
7. Add valid and invalid function tests plus runtime proofs (native executable
   and imported MFP package).
8. Run acceptance and compare results across targets.

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
- one semantic contract used by both Binary Representation packages and native
  backends
- thin target adapters only for allocation, ABI, and string storage

If `regcomp()` / `regexec()` / `regfree()` appear anywhere, they should be
behind that shared contract, never the contract itself.
