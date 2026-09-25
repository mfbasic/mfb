# plan-151-A: Builtin cost schema and man rendering

Last updated: 2026-09-23
Overall Effort: huge (>3d); planning estimate, not a measured duration.
Effort: large (3h–1d); planning estimate, including constructor migration.
Depends on: nothing

Add reviewed, overload-specific Cost sections to every public builtin function man
page. Cost records distinguish computational growth, space, external waiting and
callback behavior. They describe the current compiler; they are neither measured
nanosecond timings nor static proofs of a deadline. This file owns the final schema
and all shared authoring/validation rules. Execute A → B → C → D → E → F → G → H →
I → J → K → L, without interleaving. B–K fill the whole public registry, not just
collections. L closes the coverage gate and runs the full suite once.

## Execution order

| Letter | Work |
|---|---|
| A | [Builtin cost schema and man rendering](plan-151-A-cost-schema-and-rendering.md) |
| B | [Collection access, mutation, copying and callbacks](plan-151-B-collections.md) |
| C | [Unicode and attributed text](plan-151-C-text.md) |
| D | [Parsing, codecs, compression and patterns](plan-151-D-parsing-and-codecs.md) |
| E | [Fixed-width numeric and vector operations](plan-151-E-fixed-numerics.md) |
| F | [Arbitrary precision and cryptographic operations](plan-151-F-big-and-crypto.md) |
| G | [Time, operating-system queries and intrinsic functions](plan-151-G-general-time-and-os.md) |
| H | [Files, standard streams and child processes](plan-151-H-files-streams-processes.md) |
| I | [Resolution and transport waiting](plan-151-I-network-transports.md) |
| J | [Protocols, worker lifecycle and retained callbacks](plan-151-J-http-and-threads.md) |
| K | [Presentation, drawing, terminal and audio costs](plan-151-K-presentation-and-audio.md) |
| L | [Whole-registry completeness and final gates](plan-151-L-completeness-and-final-gates.md) |

References:

- `src/codegen/registry/mod.rs:Implementation`, `RegistryFunction`, `registry`.
- `src/cli/man.rs:render_function_markdown`, `function_errors_by_overload`.
- `src/docs/spec/tooling/07_cli-reference.md`; `.ai/man-content.md`;
  `.ai/specifications.md`; `.ai/spec-content.md`; `.ai/testing-gates.md`.
- `benchmark/README.md` and `benchmark/mfb/src/`: corroborating workloads, not proofs.

## Prerequisites

| Must be true | Command | Status at authoring |
|---|---|---|
| Existing per-overload registry and registry-driven man renderer exist | `rg -n 'struct Implementation|struct RegistryFunction|fn render_function_markdown' src/codegen/registry/mod.rs src/cli/man.rs` | MET; definitions read |

There is no dependency on an unfinished optimization plan. Describe the code present
when each package letter executes. Do not implement planned optimizations to make a
cost claim true. Re-run prerequisites before implementation; statuses are snapshots.
If a genuine prerequisite is discovered, add its command and current result here.
No implementation was performed while authoring this plan.

## 1. Goal

Every non-internal `RegistryFunction` has reviewed cost data for each
`Implementation`, and its single-function and package-all man output exposes the
applicable costs without leaking compiler internals. An executable registry census
fails on missing public overloads after L. The renderer distinguishes worst-case,
expected, and amortized claims and never turns a timeout into a wall-clock guarantee.

### Non-goals

No optimization, scheduler, runtime, resource, language, ABI, `.mfp`, DOC-block,
`mfb doc`, `mfb audit`, or benchmark-format change. No static real-time checker,
expression parser for Big-O, measured timings in registry fields, blanket constant
cost defaults, or external-provider complexity guesses. No changes to existing
builtin signatures, errors, helper source strings, or overload order. Constants,
types, package overviews and narrative guide pages do not each get a Cost section.

## 2. Current State

Read with `sed -n '440,520p' src/codegen/registry/mod.rs` and
`sed -n '776,846p' src/cli/man.rs`: `Implementation` owns params, return type,
errors and body; `RegistryFunction` owns shared prose and implementation order.
`render_function_markdown` numbers overloads and derives declaration, parameter and
error sections. Its existing error grouping is the numbering precedent, not a new
parallel signature scheme. `RegistryPackage::add_function` only asserts nonempty
implementations; shape tests walk the built registry rather than source literals
(`sed -n '1390,1435p;4690,4765p' src/codegen/registry/mod.rs`).

Read `thread::overload/function`, `math::overload`, `vector::imp/implementations`,
`general::member` and `testing` descriptor builders in their package `mod.rs`
files: helper construction expands descriptor rows. Source literal counts are
editing scope, NOT the runtime public-function or overload denominator.

Read `collections/func_append.rs:DESC_APPEND`, `func_get.rs:DESC_GET`, and
`func_any.rs:DESC`: call shape, types and callbacks already qualify performance
prose. Read `gen_list.rs:lower_list_get_common`, `gen_map.rs:lower_map_get`,
`gen_mutate.rs:lower_collection_end_insert`: access, payload materialization and
update paths must be considered separately. Read
`func_sort_by.rs:sort_by_fast_path`: concrete types and expression shapes select
paths, so a shared signature does not prove a shared cost.

Read `http/func_route.rs:register` and its BODY: a callable can be retained rather
than invoked. Read `thread/lowering.rs:lower_send` and `os/func_sleep.rs:DESC`:
queue waits and deliberate delays require different waiting descriptions.
`src/codegen/registry/mod.rs:every_registry_function_parameter_callback_is_synchronous`
actually checks synchronous and retained parameter classifications; use the body,
not its historical name, when extending tests.

`rg -n 'stub|todo!|unimplemented!|placeholder' src/codegen/registry/mod.rs src/cli/man.rs`
found type-placeholder rendering references, not a cost implementation. Cost
metadata is new; those unrelated placeholders are not an implementation blocker.

### Measured populations

Reproduce source counts with:

```sh
python3 - <<'COUNT'
from pathlib import Path
for root in ['src', 'src/codegen/builtins']:
    for needle in ['Implementation {', 'RegistryFunction {']:
        rows = [(p, p.read_text().count(needle)) for p in Path(root).rglob('*.rs')]
        print(root, needle, sum(n for _, n in rows), sum(n > 0 for _, n in rows))
COUNT
```

| Root / literal | Occurrences | Files |
|---|---:|---:|
| src / Implementation { | 633 | 529 |
| src / RegistryFunction { | 589 | 550 |
| src/codegen/builtins / Implementation { | 620 | 528 |
| src/codegen/builtins / RegistryFunction { | 583 | 549 |

These include definitions, helpers, comments and test construction; do not blindly
rewrite every match. The command above is the evidence for all four rows.

`MFB=./target/debug/mfb scripts/man-census.sh --fill` reported 588 function pages
in the existing binary. This is only a binary snapshot: source and that binary's
package surfaces differ, so it is NOT the final denominator or rollout schedule.
The new source-built registry census in A is authoritative and must include
`general` and `testing`, which top-level `man --all` omits. Internal-only functions
are excluded. `errorCode` has constants rather than callable entries.

Next number was measured with:

```sh
python3 - <<'COUNT'
from pathlib import Path
import re
print(max(int(m.group(1)) for p in Path('planning').rglob('plan-*.md')
          if (m := re.match(r'plan-(\d+)', p.name))))
COUNT
```

Output before authoring: `150`.

### Verified properties / limits

Registry/man data ownership and helper expansion are verified by the code reads
above. Actual cost bounds for the whole library are UNVERIFIED; B–K audit and
populate them. Do not confuse a complete schema with established complexity.
No runtime benchmarks or compiler test suites were run for this plan-only task.

## 3. Design Overview

Use structured categories around authored mathematical expressions and prose.
Attach `cost: Option<Cost>` to `Implementation`. FINAL meaning: `None` is permitted
only for an internal-only function; public overloads require `Some`. During A–K,
missing public data is an explicitly measured migration state, omitted from man
output and never described as constant/cheap. The temporary package coverage gate
ratchets after each letter; L removes its allow-list and enforces all public rows.

This is a documentation-output feature: man output is EXPECTED to change for
annotated pages. No generated program artifact is expected to change. Observable
CLI output and coverage checks are the feature gates; existing golden tests guard
accidental codegen drift. A surprising generated diff is investigated on one fixture,
not bulk-rebaselined. No new whole-corpus before/after byte-identity harness is needed.

Correctness risk is false costs, especially copying, cleanup, polymorphic payloads,
provider behavior and optimized expression shapes. Design risk is handled first by
A's renderer tests exercising conditional cases, mixed overloads and callbacks.
Rejected: a single blurb (no enforced dimensions), a single complexity enum (cannot
express payload/callback costs), `bounded: bool` (ambiguous), and a Big-O AST
(no calculating consumer exists).

## 4. Final schema

Place these types in `src/codegen/registry/cost.rs`, re-export from `registry/mod.rs`.
All types and fields below are `pub(crate)`; derive `Clone, Debug, PartialEq, Eq` on all types.
No blanket `Default` implementation. All text/slices are static, matching authored
registry documentation. This is the final shape, not an illustrative alternative.

```rust
struct Cost {
    variables: &'static [CostVariable],
    cases: &'static [CostCase],
}
struct CostVariable {
    name: &'static str,
    meaning: &'static str,
}
struct CostCase {
    when: &'static str,
    work: WorkCost,
    space: SpaceCost,
    waits: &'static [WaitCost],
    callbacks: &'static [CallbackCost],
    notes: &'static str,
    evidence: &'static [CostEvidence],
}
struct WorkCost {
    worst_case: WorkBound,
    expected: Option<ConditionalBound>,
    amortized: Option<ConditionalBound>,
}
enum WorkBound {
    Asymptotic { expression: &'static str, explanation: &'static str },
    ProviderDependent { explanation: &'static str },
    NoFiniteBound { explanation: &'static str },
}
struct ConditionalBound {
    expression: &'static str,
    assumptions: &'static str,
}
struct SpaceCost {
    temporary: &'static str,
    retained: &'static str,
}
struct WaitCost {
    when: &'static str,
    reason: &'static str,
    limit: WaitLimit,
}
enum WaitLimit {
    NoDeadline,
    Parameter { parameter: &'static str, semantics: &'static str },
    ProviderDefined { explanation: &'static str },
}
struct CallbackCost {
    parameter: &'static str,
    behavior: CallbackBehavior,
    detail: &'static str,
}
enum CallbackBehavior {
    InvokedDuringCall { calls: &'static str },
    RetainedForLater,
    StartedWorker,
}
struct CostEvidence {
    path: &'static str,
    symbol: &'static str,
}
// Add to the existing Implementation:
// pub(crate) cost: Option<Cost>,
```

### Field contracts

- Variables define units and meaning, e.g. element count, UTF-8 bytes, total nested
  payload, pattern length, output bytes, comparison cost. Names are unique within
  `Cost`. Expressions are authored text, not parsed or used by compilation.
- Cases are an exhaustive, non-overlapping partition under their stated conditions.
  Use `when: "All calls"` for an unconditional case. Conditional cases must include
  the general remaining case where necessary, not only a favorable fast path.
  Their conditions may name parameter values, types, targets, optimization levels
  and source forms. The reviewer establishes coverage; tests cannot prove prose.
- `worst_case` is mandatory. `Asymptotic` is a conservative computational upper
  bound under the stated model, not a wall-clock deadline. `ProviderDependent`
  means a named external provider owns the work contract; it is NOT an escape hatch
  for unreviewed MFB code. `NoFiniteBound` names an actual potentially nonterminating
  computation/protocol loop, not simply an arbitrary input size.
- Expected bounds require distribution/hash assumptions. Amortized bounds require
  the sequence, growth policy and type conditions. Neither substitutes for the
  mandatory worst-case entry. Do not call measured averages amortized complexity.
- Work includes the operation's mandatory validation, materialization/copies,
  synchronous cleanup and failure handling. Exclude evaluation of the caller's
  argument expressions and later caller-scope cleanup. Callback execution and
  external waiting are listed separately; local work must account for dispatch
  and preparing callback arguments. Include error-path costs in the upper bound;
  early success/failure refinements belong in notes or additional cases.
- Space is peak additional temporary space during the call versus additional space
  retained after return (result, resource state, queues or background work).
  Each field states an expression/size relationship and defines whether input
  reuse, output and pending work are included. Temporary excludes space that remains
  live after return; peak total additional space is bounded by their sum. For
  updates, distinguish logical result size from newly required space. Include
  type-dependent payload sizes; do not count an arbitrary record as one byte-sized
  unit. Provider-dependent space is permitted only with a named reason.
- An empty `waits` means no intrinsic wait for external progress is introduced by
  this operation. It does not mean lock-free, wait-free, constant time, or immune to
  scheduling/runtime-service delay. Those general exclusions are in the cost guide.
  Callback waiting is independent and explicitly explained by callback entries.
- A `WaitCost` names a wait phase and its condition. `Parameter` names a canonical
  parameter of this overload and explains omitted/zero/positive/negative behavior,
  units, timeout-versus-duration meaning, and whether one deadline covers retries
  or is restarted. `NoDeadline` means no API deadline, not infinite CPU work.
  `ProviderDefined` names the external policy rather than guessing one. Multiple
  phases coexist, so a network timeout never silently covers preceding DNS work.
  Timed waits remain subject to scheduling; no upper wall-clock guarantee is implied.
- Callback entries name the root parameter, including a record/container parameter
  holding callables; `detail` names the member path when indirect. Invocations
  state counts, ordering/short-circuit conditions and that user work/space/waiting
  is additional. Retaining a callback is not invoking it. Starting a worker lists
  launch/input costs here; worker execution is not charged to start's synchronous
  work. All callable paths are manually audited, not inferred from a `Func` token.
- Notes qualify assumptions and cross-reference relevant man pages; empty is allowed.
  No fixed timings, allocator/IR vocabulary or promises about unspecified optimizer
  behavior in rendered text. Existing man vocabulary bans remain in effect.
- Evidence is nonempty for every case, names repository-relative files and exact
  supporting symbols, and NEVER renders into man. Use several symbols when a cost
  depends on a helper/optimization/copy path. Source existence checks aid navigation;
  they do not prove mathematics. No commit hashes or plan/bug IDs in metadata.
  Benchmark references go in the review ledger, not the public descriptor schema.

### Worked schema applications (review requirements, not assigned bounds)

`collections::get`: separate list and map overloads; variable-sized selected values
must appear in copy costs. Map cases distinguish eligible key types and scan paths;
expected hashing claims need worst-case collisions plus hashing/comparison work.
`append`: distinguish element/list overloads, assignment-back/growth and general
new-result paths; capacity growth must not disappear behind an amortized bound.
`any`: count at most input-length predicate calls, note early exit and failures;
include argument materialization independently of predicate work.
`thread::send`: copy/message size, queue-retained space and deadline semantics;
`os::sleep`: duration is a requested delay, not a completion deadline.
`http::route`: handler is retained; `http::handleRequest` accounts for indirect
handler invocation. These cases exercise every major schema axis.

## 5. Rendering and validation

Add `render_function_cost` and helpers beside `render_function_errors_table` in
`src/cli/man.rs`; call it after Description and before Errors. Group equal PUBLIC
cost content across overloads in first-seen order. Ignore `evidence` when comparing
for presentation; otherwise identical costs with distinct proofs render once.
A mixed migration page identifies only the overload numbers actually documented.
Use the same one-based `implementations` order as Declaration/Overloads/Errors.
Render compact paragraphs/bullets, not a wide table:

- Cost heading and one line linking `mfb man tooling cost`.
- Overload numbers when needed, variable definitions, then each case condition.
- Worst-case work, optional expected/amortized work with assumptions.
- Temporary space and retained space.
- Waiting (explicit no-intrinsic-wait text if empty), callback entries and notes.

Escape authored table delimiters where any reusable rendering helper uses tables;
prefer bullets so mathematical `|` and `<` remain legible. Do not use double-backtick
spans (the current terminal renderer does not support them).

Add `src/docs/man/tooling/cost.md` and link it from `tooling/package.md`. It defines
Big-O units, worst/expected/amortized, work versus waiting, result/temporary space,
callback exclusions, and absence of hard deadlines. Add
`src/docs/spec/tooling/10_builtin-costs.md`, update `tooling/spec.md` and link from
`07_cli-reference.md`; describe the schema, validation, rendering and cost boundary
with provenance. Per-function bounds remain in descriptors, not duplicated spec
bodies. Implementation evidence stays in `CostEvidence` and the ledger.

Add tests under `registry/cost.rs` that validate all populated data: nonempty cases,
required text, unique variable names, existing canonical wait/callback parameter
names, nonempty assumptions and evidence, legal relative evidence paths and
resolving source symbols. Exact tokens, not substring matches, for symbol checks;
MFB source-body symbols inside Rust strings are allowed. Evidence checks run only
in repository tests, never while a installed compiler starts. Add negative cases
for missing fields, duplicate variables, wrong parameter, empty assumptions and
missing evidence; a schema test does not certify the bound itself.

Expose a test-only `cost_census` with `MFB_COST_PACKAGES` (comma-separated filter)
and `MFB_COST_REQUIRE_COMPLETE=1` to fail on any missing public overload in scope.
Print deterministic rows keyed by package/function/one-based overload, with full
rendered signature, case count and documented/missing status, plus totals.
Reject unknown filter packages; never succeed over an empty selection. It walks
`registry().packages()` including general/testing, filters `internal_only`, and
has an unconditional final all-public coverage test activated in L. No new user CLI
or machine-readable public format is needed. Coverage list during migration is a
named temporary test constant, removed in L.

## Compatibility / Format Impact

Only builtin man text gains Cost sections and a guide page. No package wire change,
DOC grammar change or generated program change. `Implementation.cost` is not
serialized and must not influence selection, error classification, source injection
or optimization. Default CLI commands and flags remain unchanged.

## Phases

Keep checkboxes and Commit lines current in the same commits as implementation.
An unchecked box is unfinished. Add discovered tasks and corrections explicitly.

### Phase A1 — Metadata and a truthful inventory

- [ ] Add `registry/cost.rs`, schema re-exports and `Implementation.cost`;
      migrate direct constructors and package helper builders to explicit `None`
      for the measured migration state. Preserve every body string and signature.
      Helper APIs must accept per-member/per-overload cost data rather than assign
      a blanket cost. Update test-only constructors in `registry/mod.rs` too.
- [ ] Add positive/negative schema tests, populated-row validator, evidence checks
      and `cost_census`. Create `planning/plan-151-cost-review.md` with the ledger columns specified in B;
      record freshly compiled all-public counts grouped by package and reconcile
      source helper expansion. Record baseline man vocabulary findings before editing
      cost prose so later checks can distinguish newly introduced findings.
- [ ] Tests: direct and indirect callbacks, timeout parameter aliases rejected in
      favor of canonical names, internal-only exclusion, general/testing inclusion,
      unknown package filter rejection, empty-scope rejection.

Acceptance: the census visits real expanded registry rows and missing metadata is
reported rather than treated as a cheap operation.
Check: `cargo test --bin mfb cost_ -- --nocapture` → schema tests pass and census
prints nonempty public totals (estimate 5–10 min including incremental build).
Commit: —

### Phase A2 — Cost output and authoring contract

- [ ] Implement rendering/grouping in `src/cli/man.rs`; add tests named with
      `cost_` for equal/different overloads, evidence-only differences, conditional
      cases, missing migration data, callback waits, and no leaked evidence.
      Use test descriptors only for formatting mechanics; B onward verifies real rows.
- [ ] Add the man guide and spec topic described in §5; update `.ai/man-content.md`
      editable-fields table and Cost authoring rules without weakening vocabulary bans.
      This is the repository's documentation standard, not an auto-memory status write.
- [ ] Add tests that the new guide resolves through `show_man` and that single-page,
      package-all and complete-registry rendering share the Cost renderer.

Acceptance: valid metadata renders with precise scope, and schema/evidence internals
never appear in the reader-facing section.
Check: `cargo test --bin mfb cost_` → all schema/render/guide tests pass
(estimate 5–10 min).
Commit: —

## Validation Plan

- `cargo build --bin mfb` → CLI ready (estimate 5–10 min).
- `target/debug/mfb man tooling cost` → readable guide with work/wait distinction
  (estimate <1 min; actual CLI output is the runtime proof for this feature).
- `cargo test --bin mfb spec` → new topic embedded and links discoverable
  (estimate 5 min).
- Full compiler/runtime suite and acceptance run once in L, not here. Backend
  execution is unnecessary for pure renderer tests. A cost claim requiring actual
  runtime investigation gets a bounded probe in its package letter.

## Open Decisions

None. The schema, whole-registry scope, transient migration policy, rendering order,
coverage gate and work boundary are settled above. Numerical cost bounds are review
work in B–K, not unresolved schema decisions.

## Corrections

None at authoring. Record changed premises, measurements and affected letters here.

## Summary

Structure makes omissions and distinctions testable; reviewed prose and source
provenance establish the actual cost claims. No cost annotation becomes executable
compiler policy. Finish all package letters before calling the feature complete.
