# MFBASIC Regex Implementation Plan

Last updated: 2026-06-21

This document plans how to implement the `regex` package specified in
`specifications/standard_package.md` §6 with identical behavior across current
and future targets.

It complements:

- `specifications/standard_package.md`
- `specifications/regex.md` — **the normative regex dialect spec this plan
  requires creating** (does not exist yet; see §6)
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
- current targets (`macos-aarch64`, `linux-aarch64`, glibc and musl)
- future targets

The key constraint: regex behavior must **not** depend on host libc, host OS, or
any target-specific regex quirk. The contract is a single MFBASIC-owned regex
dialect, defined on its own terms in `specifications/regex.md` (§6) — not "what
some other engine does."

## 2. The Contract Is MFBASIC's Own Dialect

This plan deliberately does **not** define MFBASIC regex as "the Rust `regex`
crate," "POSIX ERE," "PCRE," or "whatever the host `regcomp()` does." Defining
the language by reference to an external engine makes the contract depend on that
engine's version, its bug-for-bug behavior, and whatever subset we happen to
embed — none of which belongs in a portable language spec.

Instead:

- The supported syntax and matching semantics are written down completely and
  self-containedly in `specifications/regex.md`.
- Any external engine that may be used as an *implementation detail* (see §5)
  must reproduce the behavior `regex.md` specifies; it is never the source of
  truth.
- Differential testing against other engines is allowed as a *development aid*,
  but a disagreement is resolved by `regex.md`, not by deferring to the other
  engine.

Practical features chosen for v1 (the dialect, defined fully in `regex.md`):

- matching operates over **Unicode scalar sequences**, with zero-based scalar
  indexes, consistent with the rest of the string package
- literals, concatenation, alternation (`|`), grouping (`(...)`)
- quantifiers `*`, `+`, `?`, and counted `{m}`, `{m,}`, `{m,n}`, greedy by
  default with `?`-suffixed lazy forms
- character classes `[...]`, negation `[^...]`, ranges, and a defined set of
  class shorthands (e.g. `\d`, `\w`, `\s` and their negations) with their exact
  Unicode meaning pinned in `regex.md`
- anchors `^` and `$`, and the dot `.` with its defined newline behavior
- a defined, closed set of escapes
- a defined replacement-string mini-language for `regex::replace` (literal text
  plus a defined capture-reference syntax), specified independently of any other
  engine

Explicitly excluded from v1 (and stated as such in `regex.md`):

- backreferences
- look-around (lookahead/lookbehind)
- locale- or target-sensitive behavior
- any feature whose behavior we cannot pin precisely across targets

## 3. Current State

The spec defines the package in `standard_package.md` §6:

- invalid patterns fail with `ErrInvalidFormat`
- `match` succeeds when the pattern matches anywhere in the string
- `find` returns a zero-based Unicode scalar index, `ErrNotFound` when absent
- `replace` replaces all matches

> Note: `standard_package.md` §6 currently describes the dialect as "the Rust
> `regex` crate style." Part of this work (§6.1) is to replace that
> external-reference wording with a pointer to the self-contained
> `specifications/regex.md`.

The compiler/runtime state is that **no regex implementation exists yet** —
nothing is reserved or stubbed (verified 2026-06-21):

- `src/builtins/mod.rs` does not register a `regex` built-in package.
  `is_builtin_import` currently lists `fs | io | json | math | net | strings |
  thread` — note `net` is now present; `regex` is not.
- there is no `src/builtins/regex.rs` module for regex call signatures and
  lowering, and no `src/regex/` engine module.
- the shared runtime helper enum `RuntimeHelper` in
  `src/target/shared/runtime.rs` currently contains `Fs | General | Io | Math |
  Net | Strings | Thread` — there is no `Regex` family.
- native backend capability/runtime-call wiring does not advertise regex.
- there are **no** reserved regex opcodes anywhere. The flat bytecode/opcode
  model is gone (`src/bytecode.rs` no longer exists). Packages are emitted as a
  structure-preserving **Binary Representation** (`src/binary_repr.rs`,
  `MFBR`/`MFPC` v2), a serialization of the IR. `grep -ri regex src/` returns
  nothing.

A consequence of the Binary Representation model removes the old "two engines"
risk entirely: a package is **decoded straight back into IR** and lowered through
the same `IR → NIR → native` codegen used for the executable's own code (see
`architecture.md` §11). There is no separate package interpreter, so regex
semantics cannot diverge between "package" and "native" paths by construction —
both go through one code path.

## 4. Why Host `regcomp()` Alone Is Not Enough

Using the platform's native regex API directly as the semantic source of truth is
the wrong default when the goal is target-stable behavior.

Problems:

- `regcomp()` / `regexec()` semantics vary across libc implementations.
- Linux emits both glibc and musl executables; those must not drift.
- macOS uses a different libc and may differ again.
- future targets (e.g. Windows) would need a different regex API entirely.
- character class, newline, locale, and replacement behavior can diverge even
  when the APIs look similar.

Therefore:

- do not define MFBASIC regex semantics as "whatever the host regex library does"
- do not make source compatibility depend on the selected target
- do not ship regex by adding per-target behavior branches

`regcomp()` / `regexec()` / `regfree()` may be used only as an internal
development reference for differential testing, or behind a compiler-owned
compatibility wrapper that reproduces the `regex.md` contract on every target.
They are never the contract.

## 5. Recommendation

Implement the `regex.md` dialect with one parser and one matcher shared across
all backends.

Recommended architecture:

1. Write the dialect spec (`specifications/regex.md`, §6) first.
2. Parse patterns into a compiler-owned regex IR over Unicode scalars.
3. Execute that IR with one shared engine used by package generation, native
   runtime, and any future interpreter/JIT.
4. Keep target-specific code limited to string storage access, allocation, and
   error/result ABI glue.

This centralizes semantics and makes a new target a thin host adapter rather than
a new regex flavor.

## 6. Deliverable: `specifications/regex.md`

**Before implementation, author `specifications/regex.md`** as the normative,
self-contained definition of the MFBASIC regex dialect. It must define the
behavior directly — with prose, grammar, and worked examples — and must **not**
define semantics by reference to Rust, PCRE, POSIX, or any host library. Other
engines may appear only as a non-normative "informal lineage" footnote, never as
the definition of a feature.

`regex.md` must specify, completely:

- **Matching domain.** Matching is over Unicode scalar sequences; all
  user-visible indexes are zero-based Unicode scalar indexes, not byte offsets
  and not grapheme clusters (consistent with `standard_package.md` §3.1).
- **Grammar.** A complete grammar for the supported pattern syntax (EBNF or
  equivalent), covering literals, concatenation, alternation, grouping,
  quantifiers (greedy and lazy, including counted `{m,n}` forms), character
  classes and negation/ranges, class shorthands, anchors, and the dot.
- **Escapes.** The exact, closed set of supported escape sequences and what each
  one matches. Anything outside the set is an invalid pattern.
- **Class shorthands.** The precise Unicode definition of `\d`, `\w`, `\s` and
  their negations (which scalar values each includes) — pinned, not "whatever the
  platform thinks."
- **Dot and anchors.** What `.` matches with respect to line terminators, and the
  exact meaning of `^` and `$` (start/end of input vs. line), with no multiline
  mode in v1 unless explicitly defined.
- **Match selection.** Leftmost match rule, greedy-vs-lazy resolution, and how
  ties are broken, so `find` returns a single well-defined index.
- **Zero-length matches.** Whether they are allowed, and exactly how
  `regex::replace` advances past a zero-length match to guarantee termination and
  a deterministic result.
- **Replacement mini-language.** The full syntax of the `replacement` argument to
  `regex::replace`: literal text, the capture-reference syntax and numbering, how
  an unmatched/out-of-range reference behaves, and how to write a literal that
  would otherwise be a reference. Defined on its own terms.
- **Global replacement semantics.** Non-overlapping, left-to-right replacement of
  all matches, with the zero-length advance rule above.
- **Errors.** Invalid pattern → `ErrInvalidFormat`; `regex::find` no match →
  `ErrNotFound`; invalid `start` → the same out-of-range behavior other scalar
  index APIs use. Enumerate which constructs are rejected as invalid.
- **Determinism guarantee.** The same `(pattern, value)` (and `replacement`)
  must produce identical observable results on every target and on both the
  package and native code paths.
- **Non-goals for v1.** Backreferences, look-around, locale sensitivity, and any
  target-specific extensions are out of scope and rejected consistently.

### 6.1 Update `standard_package.md` §6

Once `regex.md` exists, update `standard_package.md` §6 to (a) drop the "Rust
`regex` crate style" wording and (b) point to `specifications/regex.md` as the
normative dialect definition, while keeping the high-level table of the three
functions and their error behavior.

## 7. Proposed Internal Architecture

Use a layered design:

```text
source regex::*
  -> typecheck builtin package call
  -> compiler-owned regex built-in lowering
  -> shared regex semantic layer (implements regex.md)
  -> target/package adapters
```

### 7.1 Front-End Layer

Add a new built-in module, `src/builtins/regex.rs`. Responsibilities:

- recognize `regex` as a built-in import
- resolve call signatures and arity
- expose return types
- lower regex calls into IR (and, for a native helper, runtime-helper calls)

`src/builtins/mod.rs` must be updated so `regex` participates in:

- `is_builtin_import`
- `call_return_type_name`
- `is_builtin_call`
- `call_param_names` (for named-argument resolution)

Two existing built-in packages bracket the design choice; the regex module should
mirror whichever matches the strategy chosen in §7.2:

- **Native-helper package** (`fs`, `io`, `math`, `net`, `strings`, `thread`):
  call signatures live in a `src/builtins/<name>.rs` module; operations are
  lowered to native code by `CodeBuilder` and registered as runtime helpers
  (§7.4). There is no linked runtime — the compiler emits the helper bodies.
- **MFBASIC-source package** (`json`): the entire package is written in MFBASIC
  in `src/builtins/json_package.mfb`, parsed via `builtins::json::source_file`,
  and injected through `builtins::json::augmented_project`, called from both
  `src/resolver.rs` and `src/typecheck.rs`. Such a package needs no runtime
  helper and no codegen changes — it lowers to ordinary IR like user code.

### 7.2 Shared Regex Core

Whatever the strategy, there must be exactly one implementation of the
`regex.md` semantics. The architecture has **no linked runtime library**: native
helpers are hand-emitted code generated by `CodeBuilder` (see
`src/target/shared/code/builder_strings_package.rs`), and packages decode back to
IR rather than executing a separate engine. So "one shared engine" must be one of
the following, not a Rust crate linked into a runtime:

1. **MFBASIC-source engine (recommended).** Write the parser, matcher, and
   replacement logic in MFBASIC as `src/builtins/regex_package.mfb`, following
   the `json` precedent. Because patterns are runtime strings, the engine parses
   and matches at runtime. This gives target stability for free: the engine
   compiles through the single `IR → NIR → native` codegen and is serialized into
   the Binary Representation like any other code. `json` already demonstrates a
   non-trivial parser is expressible this way.

2. **Compiler-owned Rust core driving native lowering.** Add
   `src/regex/{parse,ir,exec}.rs`. At compile time this can validate patterns and
   fold constant patterns, but because patterns may be runtime values, the
   *matching* engine must still be emitted as native code by `CodeBuilder` (a
   regex NFA/program interpreter generated into the program). This is
   substantially more work than option 1 and is only justified if regex becomes
   performance-critical.

If a Rust core is introduced, it should expose one internal API, all
implementing `regex.md`:

```text
compile(pattern) -> Result<CompiledRegex, ErrInvalidFormat>
is_match(compiled, value) -> bool
find(compiled, value, start_scalar) -> Result<MatchRange, ErrNotFound>
replace_all(compiled, value, replacement) -> String
```

`MatchRange` is scalar-index based, not byte-offset based.

Responsibilities of the core, regardless of strategy — all governed by
`regex.md`: pattern parsing and validation, compiled representation, matching
over Unicode scalar streams, first-match search, global replacement, and
producing identical behavior across targets.

### 7.3 Binary Representation (Package) Layer

There is no opcode path to "finish" — the flat bytecode model is gone. The Binary
Representation in `src/binary_repr.rs` is a structure-preserving serialization of
the IR (`MFBR`/`MFPC` v2); packages are decoded back into IR and lowered through
the same codegen as the executable's own code.

What this means for regex:

- If regex is an **MFBASIC-source package** (§7.2 option 1), no Binary
  Representation work is required. The engine lowers to ordinary IR
  (`Call`/`CallResult` nodes), which the existing encoder/decoder already
  handles. Nothing regex-specific touches `src/binary_repr.rs`.
- If regex is a **native runtime helper** (§7.2 option 2), the only Binary
  Representation surface is the `runtime_helpers: Vec<RuntimeHelper>` list a
  package records (see `src/target/shared/nir.rs`). Adding a `Regex` variant to
  `RuntimeHelper` makes it serialize like the existing `Net`/`Strings`/`Fs`
  helpers; confirm the enum's encode/decode in `src/binary_repr.rs` covers the
  new variant.

Important rule (structural, not aspirational): package and native semantics share
one regex implementation automatically, because a package is decoded to IR and
lowered through the single `IR → NIR → native` path. There is no separate package
interpreter that could drift.

### 7.4 Native Layer

This layer applies only if regex is a native runtime helper (§7.2 option 2); an
MFBASIC-source package needs none of it.

For native backends, add a `Regex` family to `RuntimeHelper` in
`src/target/shared/runtime.rs` (alongside `Net`), and emit the operation bodies
from a new `src/target/shared/code/builder_regex_package.rs` (mirroring
`builder_strings_package.rs`), since helper bodies are generated inline rather
than linked.

Add helper specs for `regex.match`, `regex.find`, `regex.replace`, then update:

- `helper_for_call`
- `supported_helper_specs`
- `is_native_direct_call` if any operation is emitted inline
- backend capability/runtime-call wiring in `src/target/macos_aarch64/` and
  `src/target/linux_aarch64/`
- target-specific import planning only if truly needed

Recommendation: do **not** add libc regex imports to target plans; emit shared
runtime helpers that implement the `regex.md` engine. That keeps
`src/target/macos_aarch64/plan.rs` and `src/target/linux_aarch64/plan.rs` free of
regex-library divergence.

## 8. Matching Model

The core engine operates on Unicode scalar indexes even though strings are stored
as UTF-8:

1. Validate `start` for `regex::find` as a scalar index.
2. Convert the UTF-8 string to a scalar cursor view (not necessarily a copied
   scalar array).
3. Run the engine on scalar positions.
4. Return scalar indexes to user code.
5. Convert scalar ranges back to byte slices only when building output strings.

This is more work than byte-oriented regex, but it is the only design that
matches the standard-package contract.

## 9. Compilation And Caching

`regex::match`, `regex::find`, and `regex::replace` accept patterns as runtime
strings, so patterns cannot be assumed constant.

Recommended execution strategy:

- compile the pattern on each call initially
- optionally add constant-pattern folding later (if the pattern is a compile-time
  constant, pre-parse/fold during lowering; otherwise parse at runtime with the
  same shared parser)
- optionally add per-call-site caching later if profiling justifies it

Caching must never be part of semantic correctness, and both paths must produce
identical results. `builder_strings_package.rs` already does this kind of
constant folding for `strings` (e.g. `static_strings_package_string` and the
constant-string `graphemes` path), so a constant-pattern fast path follows an
established pattern.

## 10. Error Behavior

Per `regex.md` and `standard_package.md`:

- invalid pattern → `ErrInvalidFormat`
- `regex::find` no match → `ErrNotFound`
- invalid `start` index → the out-of-range behavior other scalar index APIs use

Do **not** map malformed patterns to `FALSE`, `-1`, an unchanged source string,
or target-specific errno-style errors. The contract stays explicit and portable.

## 11. Validation Plan

Validation must prove identical behavior across build paths and that the
implementation matches `regex.md`.

### 11.1 Function Tests

Add mandatory valid and invalid directories per function:

- `tests/func_regex_match_valid/**`, `tests/func_regex_match_invalid/**`
- `tests/func_regex_find_valid/**`, `tests/func_regex_find_invalid/**`
- `tests/func_regex_replace_valid/**`, `tests/func_regex_replace_invalid/**`

Coverage: literal matches; alternation/grouping/quantifier cases from the
dialect; class shorthands with their pinned Unicode meaning; Unicode scalar
indexing; zero-length and boundary cases; the replacement mini-language including
capture references and literal-escape cases; invalid patterns; wrong arity; wrong
argument types; out-of-range `start`; `ErrNotFound` for absent matches. Every
worked example in `regex.md` should have a corresponding test.

### 11.2 Runtime Proofs

Regex is a runtime feature; passing compilation is not enough. Add runtime tests
that execute generated programs and prove:

- native executable regex behavior
- Binary Representation package regex behavior when an MFP package is imported
- identical observable results on current targets

For Linux, ensure both emitted flavors agree: glibc and musl.

### 11.3 Acceptance Suite

After implementation, run
`scripts/test-accept.sh target/debug/mfb target/accept-actual`. Regex is not
complete until acceptance passes and runtime behavior is demonstrated
end-to-end (native executable and imported MFP package), matching `regex.md`.

## 12. Recommended Implementation Sequence

1. **Author `specifications/regex.md`** (§6): the complete, self-contained
   dialect, defined without reference to any external engine.
2. Update `standard_package.md` §6 to point at `regex.md` (§6.1).
3. Choose the implementation strategy (§7.2): MFBASIC-source package
   (recommended) or native runtime helper.
4. Register the package in `src/builtins/mod.rs` (and, for a source package, add
   `regex_package.mfb` plus an `augmented_project` injection mirroring `json`).
5. Implement the parser/matcher/replacement to `regex.md` — in MFBASIC for a
   source package, or as `src/regex/*` plus `CodeBuilder` emission for a native
   helper.
6. (Native-helper strategy only) add the `Regex` `RuntimeHelper` variant, helper
   specs, `helper_for_call`/`supported_helper_specs`/`is_native_direct_call`
   wiring, and target capability wiring. Confirm the new variant round-trips
   through `src/binary_repr.rs`.
7. Keep platform plan/code changes minimal and free of regex-library imports; do
   not add host `regcomp` imports to target plans.
8. Add valid/invalid function tests plus runtime proofs (native + imported MFP),
   including a test per `regex.md` worked example.
9. Run acceptance and compare results across targets.

## 13. Non-Goals For V1

The first implementation does not need: PCRE/POSIX/other-engine compatibility,
locale-sensitive behavior, target-specific extensions, backreferences,
look-around, JIT, or any platform-dependent behavior differences. These may be
evaluated later, but only if they remain compatible with the single
`regex.md` contract.

## 14. Bottom Line

The contract is **MFBASIC's own regex dialect**, written down completely in
`specifications/regex.md` and implemented once:

- one self-contained dialect spec (`regex.md`), not defined by any external engine
- one compiler-owned parser/executor implementing it
- one semantic contract used by both Binary Representation packages and native
  backends
- thin target adapters only for allocation, ABI, and string storage

If `regcomp()` / `regexec()` / `regfree()` — or any other engine — appears
anywhere, it is an implementation detail behind that contract, never the contract
itself.
