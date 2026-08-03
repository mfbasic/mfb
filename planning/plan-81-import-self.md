# IMPORT self — self-referencing thread workers Plan

Last updated: 2026-08-02
Effort: large (3h–1d)

Let a package import its own public interface under the reserved specifier
`self`, so `thread::start(self::worker, …)` can spawn an exported `ISOLATED
FUNC` that lives in the *current* package. Today a thread entry point must be an
exported `ISOLATED FUNC` **from an imported package**; a same-package worker is
rejected. That forces a package author who wants intra-package fan-out (e.g. an
HTTP fetch that spawns 5 parallel document fetches) to split cohesive logic
across two packages purely to satisfy the compiler.

The single behavioral outcome a correct implementation produces: **in a
`kind: "package"` project, `IMPORT self` binds `self` to the current package's
exported API, and `thread::start(self::exportedIsolatedWorker, …)` compiles and
runs a fresh isolated instance of the current package — while the same code in a
`kind: "executable"` project fails to compile with a clear diagnostic, because
an executable has no exported interface to import.**

The design keeps `self` deliberately dumb: it is modelled as an ordinary import
whose target happens to be the current project's own EXPORT declarations. No
`self`-aware branch is added to the thread-entry checker, the visibility rule,
or `package::identifier` resolution — the app exclusion falls out for free
because an executable has no EXPORT symbols and no importable interface.

References:

- `./mfb spec language threads` and `./mfb man thread` — source-level thread model; the entry-point rule ("exported ISOLATED FUNC from an imported package").
- `./mfb spec threading` (source-model → *Entry-point enforcement*) — the compiler-side enforcement contract this plan must not weaken.
- `./mfb spec language modules-and-packages` — import resolution order, visibility (PRIVATE/PUBLIC/EXPORT), `EXPORT_IN_EXECUTABLE`, `package::identifier` rules.
- `.ai/compiler.md`, `.ai/specifications.md`, `.ai/man_*` templates — standing obligations for compiler/spec/man changes.

## Prerequisites

These are preconditions on the whole feature, not dependencies to negotiate.

| Must be true | Command | Status |
|---|---|---|
| Working tree clean / on a work branch | `git status --porcelain` | MET (2026-08-03: empty output) |
| No other plan is mid-flight on the thread checker or import resolver | `ls planning/plan-*import* planning/plan-*thread* 2>/dev/null` | MET (2026-08-03: only plan-81 itself matches import; no thread plans) |

This plan depends on **no other plan**. Everything below is written against the
current `main`.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before continuing and before deciding to stop.

## 1. Goal

- In a `kind: "package"` project, `IMPORT self` (optionally `IMPORT self AS x`)
  binds a name to the current package's **exported** API.
- `thread::start(self::w, …)` where `w` is an exported `ISOLATED FUNC` of the
  correct shape compiles and, at runtime, spawns a fresh independent instance of
  the current package (its top-level `MUT` not shared with the parent thread).
- `self::name` sees **only** EXPORT symbols of the current package — exactly what
  an external importer would see (PUBLIC and PRIVATE symbols are invisible
  through `self`).
- In a `kind: "executable"` project, `IMPORT self` is rejected with a clear,
  dedicated diagnostic (an executable has no exported interface to import).
- The existing rejection of a bare current-package worker
  (`thread::start(localWorker, …)`) is unchanged: only the `self::`-qualified
  path is newly accepted.

### Non-goals (explicit constraints)

- **No new visibility tier.** No `EXPORT PRIVATE` / package-internal-but-
  spawnable concept. Spawnable-through-`self` ⟺ EXPORT, full stop. (Recorded as a
  future idea only.)
- **No special-casing in the thread-entry checker.** `check_thread_builtin_call`
  (`src/syntaxcheck/builtins.rs`) must not learn about `self`. It keeps requiring
  `imported_package_export && Func && isolated`; `self` satisfies it by
  producing signatures that carry `imported_package_export == true`.
- **No change to `package::identifier` rules.** Still exactly two parts; nested
  qualifiers still illegal; dot vs `::` semantics unchanged.
- **No change to the meaning of a bare same-package reference.** `worker` (no
  qualifier) is still a current-package function and still rejected as a thread
  entry.
- **No cross-package transitivity / re-export.** `self` is not exportable and
  does not create re-export chains.
- **No wire/format change** to `.mfp` metadata unless Phase 1 proves it is
  required (see Open Decisions).

## 2. Current State

**Thread-entry enforcement.** `src/syntaxcheck/builtins.rs:check_thread_builtin_call`
(the `callee == "thread.start"` arm) accepts the first argument only when it
resolves to a visible signature that is simultaneously
`sig.imported_package_export && matches!(sig.kind, FunctionKind::Func) &&
sig.isolated`; otherwise it reports `TYPE_CALL_ARGUMENT_MISMATCH` (`2-203-0021`)
with *"thread.start entry point must be an exported ISOLATED FUNC from an
imported package."* It matches only `Expression::Identifier(name)` and maps
package-qualified spellings via `canonical_import_name`. Confirmed by fixtures:
`tests/syntax/threads/func_thread_start_valid/src/main.mfb` uses
`thread::start(thread_workers::echoText, …)` (qualified), and
`func_thread_start_invalid` asserts the rejection for a bare `localWorker`.

**The distinguishing field.** `FunctionSig.imported_package_export: bool`
(`src/syntaxcheck/mod.rs:96`) is `true` only for signatures loaded from an
imported package's exports — set at `src/syntaxcheck/mod.rs:774` inside the
export-loading loop (registered with `Visibility::Export`, `owner_file_path =
package_file`). In-project functions are registered with the flag `false`
(`src/syntaxcheck/mod.rs:1040`, `src/syntaxcheck/link.rs:854`). This field — not
`isolated` — is the reason a current-package `ISOLATED FUNC` is rejected.

**Import resolution.** `src/resolver/packages.rs:resolve_imported_package`
implements the resolution order: `is_builtin_import` → `dependency_packages`
map (else `IMPORT_PACKAGE_NOT_DECLARED`) → `local://` → `packages/{name}.mfp` →
`packages/{name}/project.json` → `IMPORT_PACKAGE_NOT_INSTALLED`. Driven from
`src/resolver/resolution.rs:resolve_file` (loops `file.imports`, computes
`binding_name()`/`package_name()`, dedup/alias diagnostics, then resolves).
`src/resolver/packages.rs:validate_source_package_manifest` checks the *imported*
package's `name`/`kind` (`IMPORT_PACKAGE_NAME_MISMATCH`,
`IMPORT_PACKAGE_KIND_INVALID`) — there is **no** check today that a project
imports its own name.

**Import AST.** `src/ast/types.rs:Import { module, alias, line }`;
`binding_name()` = alias or `package_name()`; `import_bindings()` maps binding →
package name. `self` is **not** currently a lexer keyword or reserved parser
token (`rg` in `src/lexer*` → no matches).

**package::identifier resolution & visibility.**
`src/resolver/resolution.rs:resolve_package_qualified_name` splits the root
binding, looks it up in `imports` (else `SYMBOL_UNKNOWN_IMPORT`, `2-201-0014`).
Visibility is centralized in `src/syntaxcheck/mod.rs:visible_from`
(`Export | Public => true`, `Private => same file`); `visible_function_sigs`
filters through it. Imported sigs are registered `Visibility::Export`, so an
importer sees only exports.

**Package/executable kind.** `EXPORT_IN_EXECUTABLE` (`2-203-0103`) is emitted by
`src/syntaxcheck/mod.rs:export_in_executable_diagnostics`, which takes an
explicit `is_package: bool` (does not thread through `SyntaxChecker`). The kind
comes from `src/manifest/mod.rs:project_kind` (`kind == "package"`), passed in by
`src/cli/build/mod.rs` (~line 430). This same `is_package` boolean is the natural
gate for `IMPORT self`.

**Runtime isolation (the load-bearing unknown).** The spec states: *"Starting
isolated functions from the same package multiple times creates multiple
independent instances; their top-level MUT bindings are not shared."* So the
runtime already supports N independent instances of one imported package. The
open question is whether the **current** package participates in that same
per-instance state scheme, or whether the current/main package's top-level `MUT`
lives in process-global storage that a self-spawned worker would share. This is
where correctness risk concentrates — see Phase 1.

### Measured populations

| What | Count | Command |
|---|---|---|
| Golden files asserting the thread.start rejection message | 7 | `rg -rl "thread.start entry point must be" tests/ \| wc -l → 7` |
| `self` as lexer/parser keyword today | 0 | `rg -n '"self"' src/lexer* → no matches` |
| Thread syntax fixtures (dir) | 26 | `ls tests/syntax/threads/ → 26 entries` |

### Verified properties

- **The thread checker is gated on `imported_package_export`, not on a distinct
  compiled artifact.** Verified by reading `check_thread_builtin_call`
  (`src/syntaxcheck/builtins.rs:425-448`) — it only consults the `FunctionSig`
  flags (`sig.imported_package_export && Func && sig.isolated`), so producing a
  self-import signature with the flag set is sufficient at the syntaxcheck layer.
- **VERIFIED (Phase 1) — the current package rides the existing per-instance
  mechanism; §4.4 is wiring-only, no net-new runtime work.** Worker isolation is
  arena-based and package-agnostic: `lower_thread_start_helper`
  (`src/target/shared/code/runtime_helpers.rs:562`) arena-allocates a fresh,
  zeroed worker arena sized `ENTRY_GLOBALS_OFFSET + arena_global_slots * 8`
  covering the *entire* merged program's writable-globals region, and the worker
  trampoline (`runtime_helpers.rs:1023-1064`, bug-369) re-runs the program's
  single `global_init` initializer in that fresh arena "so a worker sees the
  region the main thread sees instead of a region of zeros." That one initializer
  and one globals region hold the current package's own top-level `MUT` too (a
  package project's own globals are ordinary program globals), so a self-worker
  gets a fresh, declared-value-initialized, isolated copy — its `MUT` writes never
  touch the parent's. The mechanism does not key on which package the worker came
  from. (Spec: `./mfb spec threading isolation` — "the writable globals region
  lives in that arena, so each worker gets its own copy.")
- **VERIFIED (Phase 1) — the current package's functions are already addressable
  as worker entry points.** `thread.start`'s first argument lowers via
  `Expression::Identifier` → `IrValue::FunctionRef { name: canonical_value, .. }`
  (`src/ir/lower.rs:2543-2551`) whenever the name is in `context.function_types`,
  producing an ordinary `_mfb_fn_<name>` code address (the trampoline loads it at
  `THREAD_OFFSET_ENTRY`, `runtime_helpers.rs:1066-1071`). A package project's own
  exported ISOLATED FUNC is already in `function_types` and already emitted as
  such a symbol (per `./mfb spec threading worker-and-package-functions`: every
  own/merged function routes through the single `_mfb_fn_` namespace), so a
  self-worker entry is the same code-address relocation used for imported workers.
  The only work is front-end wiring: make `self`/its alias a recognized import
  binding so `canonical_import_name` maps `self.worker` to the current package's
  real function key and a self-import sig with `imported_package_export = true` is
  registered (Phases 2–4). Effort estimate holds; no re-split.

## 3. Design Overview

Model `IMPORT self` as an **ordinary import whose source is the current
project's own EXPORT declarations**. The one and only special case lives in
import resolution: when the import specifier is the reserved word `self`, instead
of probing the package store, the resolver binds `self` (or its alias) to the
current package and registers the project's EXPORT top-level declarations as
imported-package signatures — i.e. with `imported_package_export == true`,
`Visibility::Export`, mirroring the loop at `src/syntaxcheck/mod.rs:774`.

Everything downstream is untouched and works by construction:

- `self::worker` resolves through the existing `resolve_package_qualified_name`
  path and `canonical_import_name`, finding the self-import signature.
- The thread-entry checker sees `imported_package_export == true` and accepts —
  no `self` awareness.
- `visible_from` already hides non-EXPORT symbols, so `self::` exposes only the
  public API — same as any external importer.

**Where correctness risk concentrates (schedule LAST, behind Phase 1's proof):**
the runtime/codegen wiring that instantiates a fresh instance of the current
package for a self-spawned worker.

**Where design uncertainty concentrates (schedule FIRST):** whether that runtime
instantiation already exists for the current package or is net-new work. Phase 1
is the cheap experiment that resolves the whole plan's size.

**Rejected alternatives:**

- *Teach the thread checker to accept a same-package ISOLATED FUNC directly.*
  Rejected: reintroduces a same-package special case, breaks the "spawnable ⟺
  exported" contract, and loses the automatic app exclusion.
- *Let `self` see PRIVATE/PUBLIC (non-exported) symbols.* Rejected as a non-goal:
  it would make `self` mean something different from a real import and require a
  new visibility concept.
- *Import the current package by its own real name (`IMPORT mypkg`).* Rejected:
  it would route through the package store, fail (not a declared dependency of
  itself), and could produce a second compiled copy. `self` as a reserved
  specifier is clearer and copy-free.

## 4. Detailed Design

### 4.1 Reserved specifier `self`

`self` becomes a reserved import specifier recognized in `resolve_file` /
`resolve_imported_package` before the builtin/dependency lookup. It is *not* made
a general reserved identifier — only the import root position treats it
specially. `IMPORT self AS x` is permitted (alias binds `x` to the current
package); the bare `self` name is also usable unless shadowed by the normal
alias-conflict rules (which already reject collisions with top-level decls and
builtins — `self` is not a builtin, so `self::` is available by default).

### 4.2 Self-import signature synthesis (resolver/syntaxcheck)

When `self` is imported, register the current project's EXPORT top-level FUNC/
SUB/TYPE/etc. under the `self` binding as imported-package signatures
(`imported_package_export = true`, `Visibility::Export`), reusing the same
registration shape as `src/syntaxcheck/mod.rs:774`. These are *additional*
signatures keyed under the `self`/alias binding; the existing in-project
registrations (flag `false`, keyed bare) are unchanged, so ordinary unqualified
in-project calls are unaffected.

### 4.3 Executable rejection

In a `kind: "executable"` project, `IMPORT self` is rejected with a dedicated
diagnostic (new rule, e.g. `IMPORT_SELF_IN_EXECUTABLE`) explaining that an
executable has no exported interface to import; suggest that self-referencing
threads require a `kind: "package"` project. The gate reuses the same
`is_package` boolean already computed at `src/manifest/mod.rs:project_kind` and
passed into `export_in_executable_diagnostics`. Even without the dedicated
diagnostic, an executable has zero EXPORT symbols, so `self::worker` would fail
resolution — but an explicit, self-explaining error is required (no silent
"unknown identifier").

### 4.4 Runtime/codegen instantiation

`thread::start(self::w, …)` must lower to a worker that runs a fresh instance of
the current package. Phase 1 determines whether this is already covered by the
existing "multiple independent instances of the same package" machinery (in
which case this is wiring only) or requires new per-instance state setup for the
current package. Any new work here is the highest-blast-radius part and lands
last, behind runtime tests.

## Compatibility / Format Impact

- **Source surface:** adds one accepted form (`IMPORT self`). No existing valid
  program changes meaning; no existing diagnostic is removed or weakened.
- **`.mfp` format:** unchanged unless Phase 1 proves the current package needs
  new per-instance metadata to be self-spawnable (see Open Decisions). Default
  expectation: no format change.
- **Unchanged:** `package::identifier` grammar, visibility semantics, the
  bare-same-package-worker rejection, `EXPORT_IN_EXECUTABLE`.

## Phases

> Order: uncertainty first (Phase 1 spike), then the localized front-end work,
> then the highest-blast-radius runtime wiring behind tests, then docs.

### Phase 1 — Runtime-instance spike (resolve the load-bearing unknown)

Determine, before touching the parser, whether a fresh instance of the **current**
package can be spawned as a worker with isolated top-level `MUT`.

- [x] Read the threading runtime/codegen path (`./mfb spec threading`
      source-model → *worker-and-package-functions*, *function-ids-and-package-calls*,
      *isolation*; and the corresponding `src/` codegen/runtime for thread-start
      worker entry) and determine how per-package instance state is allocated, and
      whether the current package participates in the same scheme as a
      multiply-started imported package. **Finding: isolation is arena-based
      (`lower_thread_start_helper` fresh per-worker globals region +
      trampoline-re-run `global_init`, `runtime_helpers.rs:562,1023-1064`) and
      package-agnostic; the worker entry is an ordinary `FunctionRef` `_mfb_fn_`
      code address (`ir/lower.rs:2543-2551`). The current package participates by
      construction.**
- [x] Record the answer as a **Verified property** in §2 (replaced the two
      UNVERIFIED entries): "current package rides the existing per-instance
      mechanism → §4.4 is wiring only," with file:symbol evidence.
- [x] ~~If net-new runtime work is required, re-estimate the plan and split~~ —
      moot: net-new runtime work is NOT required (finding above). Effort estimate
      (large) holds; no split. No Corrections/Open-Decisions change needed beyond
      resolving the "Runtime instance mechanism" open decision to the Recommended
      option (reuse existing per-instance state), recorded below.

Acceptance: §2 contains a cited verified statement that a self-spawned worker gets
isolated top-level `MUT` (arena-based, package-agnostic), and the plan's
effort/split is re-confirmed (large, no split). MET.
Commit: 7a3b5663a

### Phase 2 — Parse & resolve `IMPORT self`

Front-end recognition of the reserved specifier, with no thread involvement yet.

- [x] Recognize `self` as a reserved import specifier in the import-resolution
      entry (`src/resolver/packages.rs:resolve_imported_package`), short-circuiting
      before builtin/dependency probing. Support `IMPORT self` and `IMPORT self AS x`
      (both route through `resolve_imported_package(name="self")` since
      `import.package_name()` is the module). Reserved const `SELF_IMPORT`
      (`src/ast/types.rs`).
- [x] Bind `self` (or alias) to the current package in the import bindings — falls
      out for free: `resolve_file` already inserts `binding_name() → package_name()`
      into the `imports` map (`imports["self"] = "self"`, or `imports["x"] = "self"`
      for the alias), so `resolve_package_qualified_name` finds the root and never
      emits `SYMBOL_UNKNOWN_IMPORT`. No AST change needed.
- [x] Emit `IMPORT_SELF_IN_EXECUTABLE` (new rule `2-201-0019` in
      `src/rules/table.rs`) when the project is not `kind: "package"`, gated on a
      new `Resolver.is_package` field (non-panicking kind read in `Resolver::new`,
      since that ctor also runs from manifest-less doc/test paths).
- [x] Preserve existing alias-conflict diagnostics: `IMPORT self AS <builtin>` and
      `IMPORT self AS <top-level>` are still caught by the existing checks; added a
      reserved-binding guard in `resolve_file` so `IMPORT other AS self` (aliasing
      another import onto the reserved `self` binding) reports `SYMBOL_DUPLICATE_IMPORT`.
- [x] Tests: `tests/syntax/project/import-self-*` fixtures — (i)
      `import-self-package-valid` resolves clean; (ii) `import-self-in-executable`
      emits `IMPORT_SELF_IN_EXECUTABLE` (not `SYMBOL_UNKNOWN_IMPORT`); (iii)
      `import-self-alias` (`IMPORT self AS me`) resolves clean; plus (iv)
      `import-self-alias-conflict` (`IMPORT io AS self`) rejected. Inline unit
      tests `self_import_in_package_is_ok` / `self_import_in_executable_is_reported`
      in `src/resolver/packages.rs` mirroring `undeclared_package_is_reported`.

Acceptance: the four fixtures produce the expected `golden/build.log` (test-accept
green); the executable case shows the dedicated diagnostic (not
`SYMBOL_UNKNOWN_IMPORT`). MET.
Commit: fdcbf62e2

### Phase 3 — Self-import signatures carry `imported_package_export`

Make `self::name` resolve to signatures the thread checker accepts, with zero
change to the checker.

- [x] When `self` is imported (package project), register the project's EXPORT
      top-level declarations under the `self`/alias binding as imported-package
      signatures (`imported_package_export = true`, `Visibility::Export`) via new
      `SyntaxChecker::collect_self_exports` (called from `collect_package_functions`
      before the `.mfp` probe), reusing the registration shape at
      `src/syntaxcheck/mod.rs:774`. The existing bare in-project (`false`)
      registrations from `collect_functions` are untouched.
- [x] Confirm `visible_from` hides non-EXPORT through `self`: only EXPORT decls are
      registered under `self.`, so a `self::hiddenWorker` (PUBLIC) reference finds
      no `self`-keyed sig and is rejected — shown by `func_thread_start_self_invalid`
      and the `thread_start_self_non_exported_rejected` unit test.
- [x] Verified `check_thread_builtin_call` is **unchanged** (`git diff --stat
      src/syntaxcheck/builtins.rs` = empty apart from the added tests; grep for
      `SELF_IMPORT`/`self::` in the checker → none) and now accepts
      `thread::start(self::echoText, …)` purely because the looked-up sig has
      `imported_package_export == true`.
- [x] Tests: `tests/syntax/threads/func_thread_start_self_valid` (`kind:
      "package"`, `thread::start(self::echoText, …)` accepted, exit 0 with correct
      `functionRef` IR) and `func_thread_start_self_invalid` (`self::` of a
      non-exported PUBLIC func **and** an EXPORT non-ISOLATED func both rejected
      with the existing `TYPE_CALL_ARGUMENT_MISMATCH` message). Inline unit tests
      `thread_start_self_entry_accepted` / `thread_start_self_non_exported_rejected`
      alongside `thread_start_bad_entry_rejected`.

Acceptance: the self-valid fixture compiles clean (test-accept green); self-invalid
reproduces the existing `TYPE_CALL_ARGUMENT_MISMATCH` errors; the thread-checker
source has no `self` reference (grep proof; empty non-test diff). MET.
Commit: 1394b01df

### Phase 4 — Runtime/codegen wiring & end-to-end proof (largest blast radius last)

Make a self-spawned worker actually run as an isolated instance.

- [x] Wire `thread::start(self::w, …)` lowering so the worker entry references
      the current package's already-compiled ISOLATED FUNC. **Landed early (with
      Phase 3)** because the valid fixture's `.ir` golden depends on it: a one-line
      special case in `src/ir/lower.rs:canonical_import_name` maps `self.worker`
      to the bare current-package key `worker`, so the `Identifier` lowering finds
      it in `function_types` and emits `functionRef name="worker"` (→
      `_mfb_fn_worker`) instead of a dangling `Local`. Confirmed in
      `func_thread_start_self_valid.ir` (`"kind": "functionRef", "name":
      "echoText"`). Fresh-instance instantiation is the existing arena mechanism
      (Phase 1), reached unchanged. Blast radius nil: the branch only fires for a
      `self`-bound qualifier, which did not exist before this plan.
- [x] Runtime test: `tests/rt-behavior/threads/thread-self-fanout-rt` — an
      executable imports the package `self_fanout_workers` (source in
      `tools/thread-package-sources/self_fanout_workers`), whose `runFanout` uses
      `IMPORT self` to spawn **two** self-workers (`thread::start(self::bump, …)`)
      that each mutate the package-level `MUT COUNTER` (declared 7). Golden output
      `a=107 b=207 parent=7` / `main_counter=7` proves (a) parallel results are
      correct and independent (107≠207 → the two workers do not share state) and
      (b) top-level `MUT` is NOT shared with the parent (parent stays 7). Release-
      seeded `.mfp` + goldens (perf-goldens caveat); `.mfp` verified byte-identical
      to a fresh release rebuild.
- [x] The HTTP fan-out shape as a worked example:
      `tests/syntax/threads/func_thread_start_self_http_fanout` — an exported
      ISOLATED FUNC `fetchStatus` doing `http::read(net::toUrl(url))`, started three
      times via `self::fetchStatus` from within the same package. Compile-only
      (a live fetch is network-flaky per the acceptance baseline); the fixture
      proves the motivating intra-package fan-out shape type-checks and lowers to
      three `functionRef fetchStatus` worker entries.

Acceptance: the runtime test passes showing correct parallel results **and**
isolated top-level `MUT` (`a=107 b=207 parent=7`); `cargo test --bin mfb` +
artifact-gate green (verified at finalization, Phase 5). MET.
Commit: eef4d4cbe

### Phase 5 — Spec, man, goldens, gate

- [x] Updated `./mfb spec language modules-and-packages` (added the reserved
      `self` specifier + executable exclusion after the import resolution order),
      `./mfb spec language threads` (entry-point rule now "…or via `IMPORT self`"),
      and `./mfb spec threading` source-model *Entry-point enforcement* (a new
      paragraph: `IMPORT self` synthesizes `imported_package_export` sigs, so the
      contract is not weakened — the checker stays `self`-unaware). Citations
      resolve (`spec_citations_resolve` green).
- [x] Updated `./mfb man thread start` (the entry-point sentence: exported
      ISOLATED FUNC reached through an import — imported package **or `self::` in a
      package project**; bare current-package still rejected). `man_citations_resolve`
      green.
- [x] Added the rule row `2-201-0019 IMPORT_SELF_IN_EXECUTABLE` to
      `src/docs/spec/diagnostics/01_rule-codes.md`; the enforced
      `every_rule_is_documented_in_the_spec` test is green.
- [x] No existing golden modified (`git status` shows only new fixtures + doc
      edits). The bare-worker rejection message is byte-identical: the 7 existing
      `thread.start entry point must be…` occurrences (6 in `func_thread_start_invalid`,
      1 in `lambda-mut-capture-invalid`) are untouched; the new
      `func_thread_start_self_invalid` legitimately adds 2 more.
- [ ] Run the full gate: `cargo test --bin mfb`, acceptance/test-accept for the
      new fixtures, and one artifact-gate at finalization.

Acceptance: spec/man/rule-table updated and in sync; full `cargo test --bin mfb`
+ artifact-gate green; new fixtures pass; no unexplained golden churn.
Commit: —

## Validation Plan

- **Tests:** syntax fixtures (self resolve OK; executable rejection; self-worker
  accept/reject) + runtime isolation test + inline unit tests in `src/resolver/*`
  and `src/syntaxcheck/builtins.rs`. Include negative cases (executable,
  non-exported target, non-isolated target, alias conflict).
- **Coverage check:** confirm the new self-path is exercised by
  `cargo test --bin mfb` (compiler tests live in the bin target, not `--lib`) —
  a green run must include the new fixtures in its denominator.
- **Runtime proof:** the Phase 4 program prints correct fan-out results and
  demonstrates non-shared top-level `MUT` across parent and self-workers.
- **Doc sync:** `mfb spec language modules-and-packages`, `mfb spec language
  threads`, `mfb spec threading`, `mfb man thread`, the diagnostics/rule table.
- **Acceptance:** `cargo test --bin mfb` + one finalization artifact-gate (do not
  run the full gate per phase; check `pgrep -f artifact-gate` first — no
  concurrent gate).

## Open Decisions

- **Runtime instance mechanism** — **RESOLVED (Phase 1): reuse the existing
  multiply-started-package per-instance state for the current package.** Isolation
  is arena-based and package-agnostic (`lower_thread_start_helper` fresh worker
  arena/globals region + trampoline-re-run `global_init`,
  `src/target/shared/code/runtime_helpers.rs:562,1023-1064`); the worker entry is
  an ordinary `FunctionRef` `_mfb_fn_` code address (`src/ir/lower.rs:2543-2551`).
  §4.4 is wiring-only; the x-large/split alternative does not apply. (§4.4, Phase 1)
- **Alias support** — *Recommended:* allow `IMPORT self AS x`. *Alternative:*
  bare `self` only. Allowing the alias costs nothing (it rides the existing
  alias machinery) and is consistent with every other import. (§4.1)
- **Diagnostic granularity** — *Recommended:* a dedicated
  `IMPORT_SELF_IN_EXECUTABLE`. *Alternative:* let it fall through to
  `SYMBOL_UNKNOWN_IMPORT`. The dedicated error is required by the "no silent
  fallthrough" bar. (§4.3)

## Corrections

- **Phase 1 open decision resolved / no re-split.** The "Runtime instance
  mechanism" open decision resolved to the Recommended option (reuse existing
  per-instance state); §4.4 is wiring-only. Effort (large) unchanged. Evidence in
  §2 Verified properties and the Phase 1 boxes.
- **`is_package` source (Phase 2).** §4.3/Phase 2 said to thread `is_package` from
  `src/cli/build/mod.rs` into the resolver. Instead it is computed inside
  `Resolver::new` from the manifest it already receives (a **non-panicking** kind
  read, because `Resolver::new` also runs from doc-validation/unit-test paths with
  an empty manifest, where `manifest::project_kind`'s `expect` would panic). Same
  gate, fewer plumbing edits; no behavior change on the real build path (validated
  manifest always carries `kind`).
- **`self::name` binding falls out for free (Phase 2).** The plan anticipated a
  possible AST/import-binding-map change so `resolve_package_qualified_name` sees
  `self`. None was needed: `resolve_file` already populates the per-file `imports`
  map from `binding_name() → package_name()`, so `self`/its alias is a known root
  with no new code.
- **Added reserved-`self`-binding guard + a 4th fixture (Phase 2).** To satisfy
  "aliasing another import to `self` must still be caught," `resolve_file` now
  rejects `IMPORT other AS self` (there was no pre-existing diagnostic for it,
  since `self` was not previously a reserved binding). Locked by the added
  `import-self-alias-conflict` fixture (beyond the plan's three).

## Summary

The engineering risk is concentrated in one place: whether the runtime can spawn
a fresh instance of the **current** package with isolated top-level `MUT`
(Phase 1). Everything else is a deliberately small, localized front-end change —
`IMPORT self` is modelled as an ordinary import of the project's own EXPORT
declarations, so the thread-entry checker, visibility rule, and qualified-name
resolution are all left untouched, and the application exclusion is free. Future
work (a package-internal-but-spawnable visibility, e.g. `EXPORT PRIVATE`) is
explicitly out of scope; today's floor is spawnable ⟺ exported.
