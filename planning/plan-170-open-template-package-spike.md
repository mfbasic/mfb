# plan-170: Spike — open user templates across package boundaries

Last updated: 2026-10-04
Effort: medium (1h–2h, guess for focused research; implementation is excluded)
Status: Planned — authoring this plan does not execute the spike

Produce an evidence-backed decision record for allowing an ordinary package to
publish a template that specializes over an importer-defined type. The output
is a detailed investigation and a recommended direction, not a compiler fix or
an implementation plan presented as already approved. The current restriction
is an ecosystem limitation: concrete wrappers cannot anticipate types declared
only by a future importer.

The requested deliverable is `planning/research/plan-170-open-template-packages.md`,
with reproducible probes embedded in that report and a comparison of viable
approaches. A conclusion that more information is needed must name the exact
unresolved question and next experiment; it must not replace evidence with a
promise that source injection will work.

References to read:

- `AGENTS.md`, `.ai/compiler.md`, `.ai/resources-packages.md`,
  `.ai/codegen-invariants.md`, `.ai/spec-content.md`, `.ai/specifications.md`.
- `src/docs/spec/language/03_templates.md` — current restriction and concrete
  wrappers; `13_modules-and-packages.md` — identity, visibility and dependencies.
- `src/docs/spec/architecture/12_monomorphization.md`,
  `03_packages.md`, `05_binary-representation.md`.
- `src/docs/spec/package/01_container-format.md`, `04_type-table.md`,
  `07_functions.md`, `08_ir-section.md`, `12_verifier-rules.md`.
- `src/docs/spec/tooling/01_project-manifest.md` and the package-manager spec
  topics located by `rg --files src/docs/spec/package-manager`.

## Prerequisites

| Must be true | Command | Status at authoring |
|---|---|---|
| Native compiler can be built locally | `cargo build --bin mfb` | MET: dev build succeeded during the spec review; rerun for the spike. |
| A scratch directory is writable | `mktemp -d /tmp/mfb-template-spike.XXXXXX` | Recheck at execution; use the returned path for every probe. |
| Current worktree changes are identified | `git status --short` | Recheck at execution; do not stage another session's files. |

No other plan or bug fix is a prerequisite. In particular, the overload-value
bug is avoided by using single-declaration functions or direct calls in probes.
Re-run every prerequisite command before starting; the command, not this
snapshot, is authoritative. Name all unmet prerequisites if work is blocked.

## 1. Goal

A reader can use the final report to decide whether to pursue cross-package
open templates, which representation to investigate next, and which semantic,
compatibility and security decisions must be settled before implementation.
Every asserted current behavior has a source symbol and command or a recorded
probe. Each proposed behavior is explicitly labeled a proposal.

### Non-goals

- No changes to compiler/runtime code, package format/version, signatures,
  manifest schema, built-in registry, language semantics or existing goldens.
- No implementation of source injection, template serialization, specialization
  caching, or public template-export syntax.
- No automatic promotion of ordinary package code to trusted compiler-owned
  source; no copying an internal sigil or bypassing privacy to make a probe pass.
- No runtime generics, traits, dynamic dispatch or general inference redesign.
- No changes to the open-overload-value bug; file independently reproduced
  compiler defects using write-bug, with their own evidence.
- No claim that the limitation is temporary or scheduled for a particular version.

## 2. Current State and Evidence Seeds

These are starting observations to recheck, not the spike's completed findings.

| Boundary | Evidence already read; command to reproduce inspection |
|---|---|
| Source dependency compilation | `src/cli/build/source_packages.rs:build_source_dependencies` / `build_source_dependency` compile a source directory into the importer's package cache before its source is resolved (`sed -n '67,132p' src/cli/build/source_packages.rs`; `sed -n '221,249p' src/cli/build/source_packages.rs`). |
| Template collection and erasure | `src/monomorph/lower.rs:Monomorphizer::new` separates template declarations; `into_project` emits concrete declarations and instantiations (`sed -n '44,86p' src/monomorph/lower.rs`; `sed -n '479,590p' src/monomorph/lower.rs`). |
| Specialization | `instantiate_function` binds parameters from call arguments, while `instantiate_type` creates concrete nominal declarations (`rg -n 'fn instantiate_function|fn instantiate_type|template_instantiation_depth|total_instantiations' src/monomorph/lower.rs`). Read the bodies and helpers, not just the matching lines. |
| Ordinary package representation | `src/binary_repr/writer.rs:lower_package_project` / `lower_project_with_external_functions` receive concrete IR; the package type table has no open-template declaration (`sed -n '1,44p' src/binary_repr/writer.rs`; `sed -n '165,195p' src/binary_repr/writer.rs`; `sed -n '65,82p' src/docs/spec/package/04_type-table.md`). |
| Built-in exception | `src/ast/manifest.rs:parse_project` injects collections before monomorphization; `src/codegen/builtins/collections/mod.rs:augmented_project` delegates to registry injection (`sed -n '60,115p' src/ast/manifest.rs`; `sed -n '270,284p' src/codegen/builtins/collections/mod.rs`). Other companions enter through `src/resolver/mod.rs:augment_project` (`sed -n '151,183p' src/resolver/mod.rs`). |
| Foreign nominal identity | `src/manifest/package.rs:package_type_owners` / `qualify_package_type` preserve the declaring owner, including through re-exported signatures (`sed -n '500,570p' src/manifest/package.rs`). New specialization keys must not erase that identity. |

The concrete-wrapper example in `language/03_templates.md` built as a package
with `target/debug/mfb build /tmp/mfb-spec6-contracts/concrete_template_api`
during plan authoring. Recreate it inside the spike's fresh scratch directory;
that temporary path is evidence of the authoring check, not a dependency.

Implementation blast radius, package-size growth and specialization build-time
cost are **UNMEASURED**. No implementation estimate is derived from them. The
spike will inventory affected producers/consumers and identify measurements a
subsequent implementation proposal needs; it will not invent cost numbers for
an implementation that does not exist.

## 3. Investigation Design

Trace the boundary where information disappears before comparing designs. Use
existing language syntax in probes; an unsupported cross-package call is a
result to explain, not something to patch around. Record compiler revision,
host, optimization level, manifest, exact source, command, exit status and
stdout/stderr so a later session can repeat each result without `/tmp` leftovers.

The highest uncertainty is **what a generic body needs from its declaring
package when instantiated elsewhere**, not the binary container field used to
carry it. The highest correctness risk is binding a helper, global or nominal
type to the importer instead of the declaring package, or creating incompatible
specializations that appear identical. Investigate those dependencies first.

Byte identity is not the research acceptance gate. No production behavior or
format changes are authorized; the proof is reproducible findings, a complete
comparison and an explicit recommendation. Any surprising existing artifact
must be localized to one probe and explained before drawing a design conclusion.

## 4. Probe Matrix

Use a library `genericlib`, an application declaring `TYPE Item`, and, where
needed, a sibling package declaring a different `Item`. Use a simple identity
function first so an unrelated collection callback bug cannot decide the result.

| Case | Question and evidence to retain |
|---|---|
| Same-compilation control | Does a local `identity OF T` accept the application's `Item` and return its field unchanged at runtime? Save output and emitted concrete symbol. |
| Source dependency | Export the open function from a source package and attempt `genericlib::identity(item)`. At which pass does it fail? Retain both producer package inspection and consumer diagnostic. |
| Installed artifact | Repeat from the exact `.mfp` produced above, without library source in the importer. Does it expose the same interface? Inspect with `mfb pkg info` and the existing package reader. |
| Concrete wrapper | Export non-template wrappers for Integer/String and call them from an importer. Require correct runtime values, not just successful package writing. |
| Open TYPE / UNION | Compare same-compilation and package forms of a record template and a union template. Separate type-name availability, constructor resolution and signature serialization. |
| Declaration environment | In a local template, call a private helper and read a declaring-package global; inventory how those references would have to survive export. Record default expressions and package-qualified calls separately. |
| Identity and repetition | Use same-leaf-name types from different owners and repeated specialization sites. Trace existing nominal keys, mangling collision defenses and instance ownership. Do not infer cross-package support from local success. |
| Built-in comparator | Run an existing source-generic collections member over an importer-defined record. Trace why injection sees that type and which internal privileges an ordinary package must not inherit. |

The report must distinguish a measured pass/failure from a source-trace-only
answer. It may state that an unimplemented transport cannot be runtime-tested;
then identify the exact invariant a future prototype must demonstrate.

## 5. Candidate Approaches to Compare

Do not choose a representation merely because the built-in precedent is handy.
Compare these alternatives on the same semantic and distribution criteria:

| Candidate | Questions the report must answer |
|---|---|
| Source carried in a signed package | Which declarations/private helpers must ship? How are names bound to the declaration environment, source locations preserved, and parser/language versions constrained? Can source be included without making all package internals public? |
| Serialized AST/HIR template bodies | At what phase is the representation stable enough to export but still generic? What provenance, imports, privacy and unresolved type information must remain? How does a reader reject malformed/deep bodies? |
| Generic IR/package extension | Which current concrete-IR assumptions would need redesign? Can generics exist before layout/ownership decisions without partially duplicating HIR? Inventory actual consumers, not guessed counts. |
| Source dependency compiled with importer | How would separate identity, globals, visibility and manifests be retained? What behavior diverges from installed `.mfp` dependencies, and is that divergence acceptable? |
| Explicit concrete exports only | What remains impossible for importer-defined types? Is this an adequate long-term policy or only a compatibility baseline? Wrappers do not count as solving arbitrary importer-defined specialization. |

For every candidate, cover:

- Binding time and lookup: declaration-site helpers/imports, overloads, defaults,
  private fields, diagnostics at definition and instantiation sites.
- Type identity and specialization ownership: package identity plus type arguments,
  collision-safe symbols, recursion budgets, repeated uses across packages,
  deduplication, and placement of generated bodies/global references.
- Export graph: template references to other templates and dependency versions,
  missing bodies, dependency cycles and closed versus open signatures.
- Distribution: source and compiled packages, signed bytes, content hashes,
  format/language versions, compatibility gates, lockfiles and reproducibility.
- Resource and ownership rules: constraints checked after concrete substitution,
  imported record layouts, RES types, closure callbacks and isolation.
- Caching and costs: what invalidates instantiations after private helper/body
  changes, cold/warm build measurement design, package size and code-size growth.
- Trust boundary: untrusted package payload parsing, bounded instantiation and
  rejection of compiler-only identifiers; installation must not execute code.

Each cell must carry evidence, a concrete design obligation, or an explicitly
unresolved question with a recommended experiment. “Works like collections” is
not an answer to declaration environment, signing or package identity.

## Compatibility / Format Impact

This spike changes documentation only. Its report must name the prospective
format/API impact of each candidate, including compatibility for existing
concrete packages. It cannot reserve a format tag, change a version, promise a
migration strategy, or silently turn a proposal into the current specification.

## Phases

### Phase 1 — reproduce the boundary and map the information flow

- [ ] Create `planning/research/plan-170-open-template-packages.md` with `Scope`,
      `Baseline`, `Probe Results`, `Information Flow`, `Candidate Comparison`,
      `Risks`, `Recommendation` and `Follow-up` headings.
- [ ] Recreate the probe matrix in `/tmp`; embed a complete Python replay script
      and its build/run commands and sources in the report. Extract the script
      to `/tmp/mfb-template-spike-replay.py` for execution; it must create fresh
      temporary projects and assert outcomes. Populate manifest `direct` and
      `requiredBy` fields correctly so dependency-closure errors do not mask
      template behavior. Capture runtime controls and negative diagnostics
      separately.
- [ ] Trace declarations from parsing through monomorphization, package writing,
      import resolution and native merge. In `Information Flow`, list the
      symbol, information entering/leaving it, and the earliest loss point.
- [ ] Record separately any confirmed compiler defects with reproducible bug
      documents; do not edit compiler code or acceptance goldens.

Acceptance: each matrix row has a measured result or a justified source-only
answer, and another reader can rerun the primary control/negative pair.
Check: `python3 /tmp/mfb-template-spike-replay.py target/debug/mfb` → exit 0
with every recorded exit status and runtime value asserted; unsupported cases
assert their diagnostic rather than pretending to succeed (guess: under
5 minutes on the local macOS AArch64 host). The report must contain the script
verbatim so this command is reproducible after temporary files are removed.
Commit: —

### Phase 2 — compare representations and declaration environments

- [ ] Fill `Candidate Comparison` against every criterion in §5, using concrete
      source symbols and package-format fields rather than generic pros/cons.
- [ ] In `Risks`, work through importer-defined `Item`, a private helper, an
      imported foreign type and repeated specialization across intermediaries
      for each viable candidate; explain how identity and visibility would hold.
- [ ] Record why each rejected candidate fails a requirement, or what additional
      experiment is needed. Inventory candidate-specific affected consumers
      using scoped `rg` commands and retain the command output in the report.

Acceptance: every candidate and criterion has a disposition supported by the
Phase 1 boundary trace; no unsupported candidate is labeled implemented.
Check: `cat planning/research/plan-170-open-template-packages.md` → manual
matrix audit finds no empty candidate/criterion cells and each factual claim
has its inspection/probe command (guess: 10 minutes, local documentation review).
Commit: —

### Phase 3 — decision record and bounded next step

- [ ] Write `Recommendation`: preferred direction, rejected alternatives,
      confidence, unresolved decisions and specific experiments needed next.
- [ ] Write `Follow-up`: separate prerequisite fixes from future feature work,
      list runtime/diagnostic/package compatibility tests an implementation must
      satisfy, and state what remains UNMEASURED. Do not write or start the fix.
- [ ] Reconcile all plan checkboxes and Corrections against evidence, then
      archive this plan only when the full research deliverable is verified.

Acceptance: a reader can accept/reject the recommended direction without
reconstructing the compiler boundary or mistaking the report for shipped support.
Check: `cat planning/research/plan-170-open-template-packages.md` → recommendation
names a direction, evidence, risks and next experiments; no implementation is
claimed complete (guess: 5 minutes, local documentation review).
Commit: —

## Validation Plan

- Reproduction: the report retains all source/manifest/command inputs and exact
  observed outputs; temporary files alone are not a durable result.
- Supported controls execute and assert values. Negative probes assert the
  failure location/category. Compilation alone cannot prove template semantics.
- Final gate, once: `git diff --check` → no whitespace errors; review
  `git diff --name-only` → only the research report, this plan and any separately
  filed bug documents changed (guess: under 1 minute on the local host).
- No whole compiler/acceptance suite is required for the research-only change:
  it modifies neither compiler code nor embedded spec. If an independently
  authorized implementation follows, it needs its own plan and the compiler's
  runtime/function-fixture/full-acceptance gates; this spike cannot waive them.
- If the research discovers stale embedded-spec prose, record the exact claim
  and evidence for a separate documentation correction rather than changing
  language behavior under this plan.

## Open Decisions

- Preferred payload: unresolved; compare source, HIR and generic IR before
  recommending one. Do not assume the least code initially means the least
  compatibility risk.
- Definition-site versus instantiation-site lookup: recommend preserving the
  declaring environment for non-template names; explicitly identify any intended
  importer-dependent operations and their diagnostics.
- Source-only generics: evaluate as an alternative, but recommend parity between
  source and compiled dependency forms unless a deliberate product decision
  permits divergence.
- Future implementation authorization: separate from accepting this research
  plan. Completing the spike produces a recommendation, not permission to fix.

## Corrections

None yet; this plan has not been executed. Record changed premises and the
commands that disproved them here during the spike.
