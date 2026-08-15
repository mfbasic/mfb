# plan-97: package-scoped resource types (`process::Process`, not bare `Process`)

Last updated: 2026-08-15
Effort: x-large (1d–3d)

<!-- x-large → splits by effort into small/medium sub-plans. The Phases below are
     each independently landable and map 1:1 to the intended split
     plan-97-A … plan-97-E; kept in one file here as the design of record.
     Promote to per-letter files when execution starts. -->

Make builtin resource types package-scoped, exactly like functions, records,
unions, and enums. Today the 9 builtin resource type names (`File`, `Socket`,
`Listener`, `UdpSocket`, `AudioInput`, `AudioOutput`, `TlsSocket`,
`TlsListener`, `Process`) are a global, import-independent **bare-name**
reservation, so a user `TYPE Process` is silently reclassified as the builtin
resource (bug-441). This plan makes resource types referenced as `pkg::Name`
(`process::Process`, `fs::File`), keys resource identity by owning package end
to end, and frees the bare namespace for user types.

**The single behavioral outcome:** a program may declare `TYPE Process` (or any
resource name) and use it as an ordinary record; a builtin resource is reached
only via its qualified name; and two packages may each own a same-named resource
without their close ops cross-wiring.

References:

- `bugs/bug-441-resources-not-package-scoped.md` — the defect, repro, and root
  cause this plan fixes. (This plan is bug-441 "Phase 2b".)
- `src/docs/spec/language/15_resource-management.md`,
  `src/docs/spec/language/04_types.md` — the RES model and type-naming rules the
  fix must update.
- `planning/todo.md` — the registry-migration roadmap this plan sequences
  against (must land before the resource-owning packages migrate in its Phase 1).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| bug-441 is filed (defines the target behavior) | `ls bugs/bug-441-*.md → 1 match` | MET |
| The resource-owning packages `fs`/`net`/`tls`/`audio` have NOT yet migrated onto the registry (else their bare-name `add_resource` sites must be re-qualified as part of this plan) | `ls src/builtins/fs.rs src/builtins/net.rs src/builtins/tls.rs src/builtins/audio.rs → 4 matches (still old-branch)` | MET (2026-08-15) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before continuing and before deciding to stop. The second
> row is *why this plan is scheduled early*: each of those packages that migrates
> first adds another bare-name resource to re-qualify. If any have migrated, add
> their `add_resource` sites to Phase 2's scope.

Everything below is written against the world where these hold.

## 1. Goal

- `process::Process` (and `fs::File`, etc.) is the way a resource type is named in
  source; the resolver accepts it and rejects a bare `Process` as a resource.
- A user `TYPE Process` / `TYPE File` compiles and runs as a plain record —
  bug-441's `/tmp/collide` repro builds and prints `42`.
- Resource identity (`is_resource`, close-op lookup, sendability, close-may-fail,
  `STATE`/thread-plane parsing) is keyed by **owning package + name**, not a bare
  string; the registry accessor is package-scoped, not first-match.
- A clear diagnostic when a user name collides with a builtin resource, instead
  of the downstream `TYPE_RESOURCE_REQUIRES_RES` / `unused runtime helper`.

### Non-goals (explicit constraints)

- **Resource semantics do not change**: RES binding discipline, scope-drop, the
  meaning of `STATE T`, thread-plane `RES` elements, sendability, close-may-fail.
  Only how resource types are *named and scoped* changes.
- The set of builtin resources and their close ops (`process.__drop`, `fs.close`,
  …) is unchanged — note these op names are *already* qualified; it is the *type*
  name that is bare.
- No change to non-resource type resolution (records/unions/enums keep resolving
  as they do via `qualified_builtin_type`).
- **Forbidden wrong fixes** (from bug-441): diagnostic-only and calling it done;
  renaming the builtin resources; relaxing the repro test.

## 2. Current State

- Resource identity is a **bare** type-name string. `src/builtins/resource.rs`:
  `BUILTIN_RESOURCES` seeds the table keyed by bare name (`FILE_TYPE = "File"`,
  `PROCESS_TYPE = "Process"`, …); `ResourceRegistry::is_resource`
  (`resource.rs:104`) and `close_function` (`:110`) are bare-name `HashMap`
  lookups; `base_resource_name` (`:283`) / `state_type_name` (`:291`) parse
  `"<Name> STATE <T>"` by bare name.
- The registry accessor `src/codegen/registry/mod.rs:1497 resource_close_function`
  searches **all packages, first-match, by bare name**
  (`packages().iter().find_map(|p| p.resources().iter().find(|r| r.name == name))`)
  — no package scoping. `RegistryResource` already knows its owning package (it
  lives on a `RegistryPackage`), so the data to scope is present but unused.
- The resolver treats a bare type name as a resource before honoring a same-named
  user `TYPE`. Value types are immune because they resolve **qualified**:
  `src/resolver/resolution.rs:1450` maps `pkg.Type` → its id via
  `qualified_builtin_type`; resource type names have no such qualified spelling
  (users write `RES File`).
- Close **op** names are already qualified (`process.__drop`, `fs.close`) — the
  downstream drop-wiring speaks qualified op names; only the resource *type* key
  is bare.

### Measured populations

| What | Count | Command |
|---|---|---|
| Builtin resource types | 9 | read `src/builtins/resource.rs` `BUILTIN_RESOURCES` |
| Owning packages | 5 | fs, net, tls, audio, process (same source) |
| Bare-name `resource_close_function` consumer sites | 57 | `grep -rn "resource_close_function" src --include='*.rs' \| grep -v "fn resource_close_function" \| grep -v test \| wc -l → 57` |
| `is_resource` / `is_builtin_resource_type` consumers | 21 | `grep -rn "\.is_resource(\|is_builtin_resource_type(" src --include='*.rs' \| wc -l → 21` |
| `RES ` spellings in test `.mfb` sources | 733 | `grep -rn "RES " tests --include='*.mfb' \| wc -l → 733` |
| ` STATE ` clauses in test `.mfb` sources | 192 | `grep -rn " STATE " tests --include='*.mfb' \| wc -l → 192` |

Note on the 733/192: most `RES` bindings are **inferred** (`RES p = fs::open(...)`)
and do NOT spell the type name — they are unaffected. The syntax churn is only
where a resource type name is written explicitly: function params
(`f AS RES File`), `STATE` clauses, `List OF RES File`, thread planes. Phase 5
must measure that narrower set before touching sources (see its first task).

### Verified properties

- **A user `TYPE Process` is wrongly rejected today** — VERIFIED by building
  bug-441's repro (`TYPE Process` → `TYPE_RESOURCE_REQUIRES_RES` with no import;
  `TYPE Foo` builds & prints `42`).
- **Close ops are already qualified** — VERIFIED: `process.__drop`, `fs.close`
  in `BUILTIN_RESOURCES` / per-package `resource_close_function`.
- **`RegistryResource` carries its owning package** — VERIFIED: resources live on
  `RegistryPackage`; `registry::resource_close_function` already iterates
  `packages()` (it just discards the package identity).
- UNVERIFIED (Phase 1 task): that the resolver can cleanly distinguish "bare name
  = user type" from "`pkg::Name` = builtin resource" without disturbing existing
  qualified value-type resolution. This is the design's core premise — falsify it
  first.

## 3. Design Overview

The canonical resource identity becomes **package-qualified** (`process::Process`).
Because close *op* names are already qualified, the change concentrates in three
layers, scheduled uncertainty-first, blast-radius-last:

1. **Resolution premise (uncertainty, Phase 1):** prove the resolver can route
   `pkg::Name` → builtin resource while a bare `Name` falls through to the user
   type set — the cheapest experiment that could kill the design.
2. **Identity keying (core, Phases 2–3):** scope the registry accessor and the
   `ResourceRegistry` table by owning package; make the resolver require the
   qualified spelling and emit the collision diagnostic.
3. **Threading + migration (blast radius, Phases 4–5):** carry the qualifier
   through `STATE`/thread parsing, the 57 close-op consumers and 21 `is_resource`
   consumers, `binary_repr`, and then the user-facing syntax + spec + man +
   goldens.

**Byte-identity is NOT this plan's gate.** Behavior legitimately changes (a name
that failed now compiles; a resource is spelled differently). Use rt-behavior +
resolver tests. The `.ncode`/objdump for programs that use resources may shift
where the resource type *string* is embedded (e.g. `binary_repr` close-op
tables); that diff is the plan working. State per fixture which diffs are
expected before regenerating goldens (Phase 5).

Rejected alternatives:
- **Keep bare internal ids, only add a resolver diagnostic** — that is bug-441
  Phase 2a (the interim), not this plan; leaves the cross-package hazard and the
  inconsistency. Land it separately/earlier if desired, but it is not a
  substitute.
- **Auto-qualify a bare resource name to the importing package** — ambiguous with
  multiple exporters and still squats the bare namespace.
- **Reserve resource names only when the owning package is imported** — still
  inconsistent with value types and still cross-package-unsafe.

## 4. Detailed Design

- **Canonical key = `"pkg::Name"`** for resource identity in `ResourceRegistry`
  and the registry accessor. `base_resource_name`/`state_type_name` gain the
  qualifier: `"process::Process STATE Foo"` splits into base `"process::Process"`
  and state `"Foo"` (the ` STATE ` split is unchanged; the base now carries
  `pkg::`).
- **Registry accessor** becomes package-scoped: resolve `(pkg, name)` (or a
  qualified string) against the owning package's resources only — no first-match.
- **Resolver:** in `resolve_type_name`/`resolve_package_qualified_name`, a
  `pkg::Name` where the package owns a resource `Name` resolves to the resource;
  a bare `Name` that a user `TYPE` declares resolves to that record; a bare name
  matching only a builtin resource emits the new collision diagnostic
  (subsuming bug-441 Phase 2a).
- **Downstream consumers** (57 close-op + 21 is_resource sites, `binary_repr`,
  drop-wiring) receive qualified type strings; close-**op** lookup is unchanged
  (already qualified).

## Compatibility / Format Impact

- **Source syntax (breaking):** a resource type spelled explicitly must be
  written `pkg::Name` (`f AS RES fs::File`, `fs::File STATE Cursor`). Inferred
  `RES` bindings are unaffected. Requires a migration decision (Open Decisions).
- **Spec + man pages:** resource-type spellings updated.
- **`binary_repr` / any embedded resource type strings:** the stored key changes
  from bare to qualified — an on-disk/emitted-format shift; goldens regenerate.
- **Unchanged:** close-op names, resource semantics, value-type resolution.

## Phases

> Keep checkboxes current in the same commit as the work. An unticked box means
> NOT DONE.

### Phase 1 — falsify the resolution premise + failing tests (no behavior change)

Prove `pkg::Name`→resource / bare→user-type routing is feasible before touching
identity keying.

- [ ] Add failing tests: (a) user `TYPE Process` (no imports) compiles & runs
      [rt-behavior fixture, mirrors bug-441 repro]; (b) `process::Process`
      resolves as the resource; (c) a two-package same-named-resource unit test
      showing today's first-match cross-wiring (registry unit test).
- [ ] Spike the resolver routing on one resource (`Process`) behind the tests;
      confirm no regression to qualified value-type resolution. Record the result
      in Verified properties.
- [ ] Tests: resolver + `codegen::registry` + one rt-behavior compile fixture.

Acceptance: the new tests fail for the documented reason; the spike confirms the
routing premise (or this plan stops here with the premise recorded false).
Commit: —

### Phase 2 — package-scoped resource identity (registry + resource table)

- [ ] Make `src/codegen/registry/mod.rs resource_close_function` (and any
      resource accessor) package-scoped — resolve within the owning package, not
      first-match.
- [ ] Key `ResourceRegistry` / `BUILTIN_RESOURCES` by `"pkg::Name"`; update the
      seed sites (9 entries) and `is_resource`/`close_function`/`is_sendable`/
      `close_may_fail`.

Acceptance: registry + resource unit tests show `process::Process` resolves and a
same-named resource in another package does not cross-wire; suite green.
Commit: —

### Phase 3 — resolver: qualified references + collision diagnostic

- [ ] `resolve_type_name`/`resolve_package_qualified_name`: route `pkg::Name` to
      the resource; let a bare user `TYPE Name` win; emit the "collides with
      builtin resource `pkg::Name`" diagnostic for a bare resource name.
- [ ] This subsumes bug-441 Phase 2a.

Acceptance: bug-441 repro builds & prints `42`; a bare resource-name TYPE with the
intent to use the builtin yields the clear diagnostic; suite green.
Commit: —

### Phase 4 — thread the qualifier through parsing, consumers, binary_repr (blast radius)

- [ ] `base_resource_name`/`state_type_name` carry `pkg::` through `STATE`/thread
      parsing (`src/builtins/resource.rs`, thread helpers).
- [ ] Update the 57 `resource_close_function` consumers and 21 `is_resource`
      consumers to pass qualified type strings (`target/shared/**`, `ir/verify/**`,
      `binary_repr/**`). Close-op lookup unchanged.

Acceptance: full `cargo test` green; NIR/drop-wiring for resource programs
correct; no bare-name resource lookup remains (`grep` audit).
Commit: —

### Phase 5 — migrate user syntax + spec + man + goldens (largest blast radius, last)

- [ ] Measure the *narrow* set of explicit resource-type spellings (params,
      `STATE`, `List OF RES`, thread planes) — `grep` the actual sites, don't
      touch inferred `RES` bindings.
- [ ] Update those `.mfb` sources, `language/15_resource-management.md`,
      `04_types.md`, and resource man pages to `pkg::Name`.
- [ ] Regenerate affected goldens/`.ncode`/binary_repr snapshots; diff and
      confirm the delta is ONLY the type-string qualification.

Acceptance: full suite green; golden deltas are exactly the qualification;
migrated example programs build and run.
Commit: —

## Validation Plan

- Tests: resolver routing (qualified vs bare), registry package-scoping,
  rt-behavior compile fixture for user `TYPE Process`, negative case for the
  collision diagnostic.
- Coverage check: confirm the resolver/resource changes are in the bin unit-test
  denominator (per memory: `--bin mfb`).
- Runtime proof: bug-441 `/tmp/collide` (user `TYPE Process`) builds & prints
  `42`; a resource program still opens/closes correctly (e.g. an fs/process
  rt-behavior fixture) under the qualified spelling.
- Doc sync: `language/15_resource-management.md`, `04_types.md`, resource man
  pages.
- Acceptance: the project's full test/CI command(s).

## Open Decisions

- **Migration of existing source** — recommended: hard cutover (update all
  in-tree spellings in Phase 5, one release) vs. a deprecation window that accepts
  bare resource names with a warning. Cutover is simpler; a window is friendlier to
  out-of-tree code. (§Compatibility)
- **Interim diagnostic (bug-441 Phase 2a)** — recommended: land it independently
  *before* this plan as a cheap non-breaking stopgap; Phase 3 here supersedes it.

## Corrections

<Filled in during execution.>

## Summary

The engineering risk is concentrated in Phases 4–5: bare resource type-name
identity is load-bearing across 57 close-op consumers, 21 `is_resource`
consumers, `STATE`/thread parsing, `binary_repr`, and user-facing syntax + spec +
goldens. Phases 1–3 are the small, high-value core (resolver routing + package
scoping) that fixes bug-441's observable footgun; Phases 4–5 are the wide,
mechanical threading + migration. Resource *semantics* are untouched — only how
resource types are named and scoped. Sequence this **before** the resource-owning
packages (`fs`/`net`/`tls`/`audio`) migrate onto the registry, so each adopts
qualified resources from the start.
