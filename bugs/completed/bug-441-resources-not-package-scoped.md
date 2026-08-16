# bug-441: builtin resource type names are a global unqualified reservation (should be package-scoped, `process::Process` not bare `Process`)

Last updated: 2026-08-15
Effort: x-large (1d–3d)
Severity: MEDIUM (footgun today; latent HIGH once a second package registers a same-named resource)
Class: Footgun (+ latent Correctness)

STATUS: FIXED (process eb010b7d8; fs 8a0bd49c2; net/tls/audio b61003c20; docs 36fbb17c3)

All nine builtin resources now carry a package-qualified type identity end to end
(`fs.File`, `net.Socket`, `net.Listener`, `net.UdpSocket`, `tls.TlsSocket`,
`tls.TlsListener`, `audio.AudioInput`, `audio.AudioOutput`, `process.Process`).
Bare resource names no longer resolve (hard cutover, plan-97) — the bare
namespace is free for user types, so a user `TYPE File` / `RESOURCE Socket`
compiles as a distinct user type (the bug-373 shadow ban is inverted; its fixture
is now `tests/syntax/resources/user-resource-shadows-builtin-valid`). Cross-package
collisions (`process.Process` vs `foo.Process`) are impossible.

Regression coverage: `src/syntaxcheck/link.rs` `user_resource_named_like_a_builtin_is_accepted`;
the resource unit tests across `src/builtins/{fs,net,tls,audio}.rs` +
`src/codegen/builtins/process`; the swept rt-behavior/syntax resource fixtures;
and the converted valid shadow fixture. Full `cargo test` green; acceptance
green for every resource fixture.

Deviation: the plan expected a *narrow* Phase-5 sweep, but the earlier fs slice's
`File`-substring seds had corrupted user identifiers (`SoundFile`, `SfFile`,
`FileInfo`) and left several `.mfb`/`.mfp`/thread-package sources half-migrated;
those were repaired here (see the net/tls/audio commit), and `bug221_worker.mfp`
(a sourceless committed copy) was reconstructed as an in-tree package source.
A separate, pre-existing, non-resource regression in source-companion arg-mismatch
diagnostics was surfaced and captured as bug-443 (not fixed here).

Regression Test: user `RESOURCE File`/`TYPE File` accepted (link.rs unit test +
`user-resource-shadows-builtin-valid` fixture); cross-package resolution via the
per-package qualified identities in `resolver::BUILTIN_TYPES`.

Every builtin resource type name — `File`, `Socket`, `Listener`, `UdpSocket`,
`AudioInput`, `AudioOutput`, `TlsSocket`, `TlsListener`, `Process` — is a
**global, unqualified, import-independent bare-name reservation**. A user who
declares `TYPE Process` (or `File`, etc.) has their record silently reclassified
as the builtin resource at type-check time, producing confusing errors and
making the name unusable — *without even importing the owning package*. Every
other builtin surface is package-scoped: functions are `process::spawn`, and
builtin value types (records/unions/enums) are spelled qualified in source
(`net::Url`, `process::Process`). Resource type names are the **only** builtin
type users spell bare (`RES File`, `File STATE Cursor`, `List OF RES File`), and
the resource classifier keys on that bare name globally.

**The single correct behavior a fix produces:** a resource type is addressed
package-qualified (`process::Process`), exactly like a function/record/union/enum;
the bare name `Process` no longer resolves to the builtin resource; a user
`TYPE Process` compiles and runs normally; and resource identity is
package-scoped end-to-end so two packages can each own a same-named resource
without cross-wiring.

References:

- `src/docs/spec/language/15_resource-management.md`, `src/docs/spec/language/04_types.md`
  — the RES resource model and type-name rules this bug implicates (the fix must
  update these).
- Found during the codegen/registry migration review (registry resource
  modeling: `RegistryResource`, `registry::resource_close_function` /
  `builtin_resource`). Related: the registry's bare-name resource lookup is the
  new half of the hazard.

## Failing Reproduction

Two throwaway projects (kept at `/tmp/collide`, `/tmp/nocollide` during
investigation). Minimal form — a single-file executable project with
`project.json` (kind `executable`, entry `main`) and:

```mfb
' src/main.mfb — NO imports at all
IMPORT io
TYPE Process
  x AS Integer
END TYPE
FUNC main AS Integer
  LET p AS Process = Process[42]
  io::print(toString(p.x))
  RETURN 0
END FUNC
```

- Observed: `mfb build` fails —
  `error[2-203-0082 TYPE_RESOURCE_REQUIRES_RES]: resource must be bound with RES`
  — the user's plain record is treated as the builtin `Process` resource, with
  **no `IMPORT process`** anywhere.
- Expected: builds and runs, printing `42` (a user record named `Process` is a
  normal record).

Contrast / bounding cases (all verified):

| Case | Result |
| --- | --- |
| `TYPE Foo` (non-resource name), no imports | builds, runs, prints `42` ✓ |
| `TYPE File` (classic resource name), no imports | `TYPE_RESOURCE_REQUIRES_RES` ✗ |
| `TYPE Process`, `LET`/`MUT` binding | `TYPE_RESOURCE_REQUIRES_RES` ✗ |
| `TYPE Process`, `RES p AS Process = Process[42]` (no import) | type-checks, then codegen fails `NIR declares unused runtime helper 'process'` ✗ |
| `TYPE Process`, `RES` binding **+ `IMPORT process`** | same codegen error ✗ |

The `RES`-binding path is the important one: it **passes the type checker** (the
record is accepted *and* RES-bound because `Process` is "a resource"), and only
fails later in codegen. The dangerous outcome it gestures at — a user record
RES-bound and then having `process.__drop` (SIGKILL + waitpid) run on its bytes
at scope exit — is currently blocked before a runnable binary exists, so this is
a footgun/diagnostics bug **today**, not a memory-safety hole. See Blast Radius
for the latent path that could reach runtime.

## Root Cause

Resource identity is keyed on the **bare** type-name string, unconditionally and
without a package qualifier, in two layers:

- `src/builtins/resource.rs` — `BUILTIN_RESOURCES` (via
  `ResourceRegistry::with_builtins`) seeds the resource table with every builtin
  resource keyed by its bare name (`"File"`, `"Process"`, …), always, regardless
  of imports. `ResourceRegistry::is_resource` (`resource.rs:104`) /
  `close_function` (`:110`) are plain bare-name `HashMap` lookups. So *any* type
  named `Process` anywhere in a program matches.
- The resolver classifies a bare type name as a resource before honoring a
  same-named user `TYPE`. Value types are immune because they are spelled and
  resolved **qualified** — `resolver/resolution.rs:1450`
  (`resolve_package_qualified_name` → `qualified_builtin_type("net.Url")`) maps a
  `pkg.Type` reference to its bare internal id, so value types never squat the
  bare *source* namespace. Resources have no such qualified spelling: users write
  `RES File`, so the resource name lives in the bare namespace that user `TYPE`s
  also occupy.

The registry adds the second half of the hazard:
`src/codegen/registry/mod.rs::resource_close_function` /
`builtin_resource` resolve a resource by bare name via
`registry().packages().iter().find_map(|pkg| pkg.resources().iter().find(|r| r.name == name))`
— **first package wins, no package scoping**. Today only `process` owns
`Process`, so it is latent; the moment a second migrated package registers a
same-named resource, the close op silently binds to whichever `build()`
registers first.

Why the contrast cases are immune: `Foo` isn't in the resource table, so
`is_resource("Foo")` is false and the user record stands. `net::Url` is a value
type resolved through the qualified path, so it never reserves bare `Url`.

## Goal

- A resource type is referenced package-qualified in source (`process::Process`,
  `fs::File`) — consistent with functions, records, unions, and enums.
- Bare `Process` / `File` no longer resolves to a builtin resource; a user
  `TYPE Process` (or any resource-name) compiles and runs as an ordinary record.
- Resource identity is package-scoped end-to-end: `is_resource`,
  `resource_close_function`, and the registry accessors disambiguate by owning
  package, so `process::Process` and a hypothetical `foo::Process` never
  cross-wire their close ops.
- A clear diagnostic when a name genuinely collides, instead of a downstream
  `TYPE_RESOURCE_REQUIRES_RES` / `unused runtime helper` error.

### Non-goals (must NOT change)

- Resource **ownership/lifetime semantics**: RES binding discipline, scope-drop,
  `STATE T` behavior, thread-plane `RES` elements, sendability, close-may-fail —
  only the *naming/scoping* of resource types changes, not what resources do.
- The set of builtin resources or their close ops.
- **Tempting wrong fixes, explicitly forbidden:**
  - Adding only a "name is reserved" diagnostic and calling the bug fixed — that
    keeps the bare reservation and the cross-package hazard; it's a legitimate
    *interim* (Phase 2a) but not the fix.
  - Renaming the builtin resources to dodge collisions.
  - Rewriting/relaxing the repro test so it no longer exercises the collision.

## Blast Radius

Bare-name resource identity is threaded through the whole stack. Found by search,
not memory:

- `src/builtins/resource.rs` — `BUILTIN_RESOURCES`, `ResourceRegistry`
  (`is_resource`/`close_function`/`is_sendable`/`close_may_fail`),
  `base_resource_name`, `state_type_name` (split `" STATE "` by bare name),
  `is_builtin_resource_type`, `builtin_resource_close_function` — **fixed by this
  bug** (core keying).
- `src/codegen/registry/mod.rs` — `resource_close_function`, `builtin_resource`
  (bare, first-match), `resource_base_eq` (unification) — **fixed by this bug**
  (cross-package scoping).
- `src/resolver/resolution.rs` — `resolve_type_name` /
  `resolve_package_qualified_name` (bare resource classification vs qualified
  value-type path) — **fixed by this bug**.
- `src/target/shared/**` — `resource_close_function` call sites in
  `validate/mod.rs`, `runtime/usage.rs`, `plan/symbols.rs`,
  `code/builder_resource_cleanup.rs`, `code/builder_exits.rs`,
  `code/module_analysis.rs` (drop wiring / NIR `Bind` by `type_`) — **in scope**:
  they consume the type-name key and must carry the qualifier.
- `src/ir/verify/**` — `link.rs`, `mod.rs` (`is_builtin_resource_type`) —
  **in scope**.
- `src/binary_repr/**` — `builder.rs` + tests key close ops by bare name —
  **in scope**.
- Spec (`language/15_resource-management.md`, `04_types.md`), man pages, and
  **every existing program/test/fixture that spells a resource type**
  (`RES File`, `File STATE Cursor`, `List OF RES File`, thread planes) —
  **in scope** (user-visible syntax + goldens shift): the biggest churn, and why
  this is x-large and breaking.

## Fix Design

Two layers; recommend splitting delivery.

**Phase 2a — interim, non-breaking (this bug):** at `TYPE`/`UNION`/`ENUM`
declaration, if the declared bare name equals a builtin resource type name, emit
a targeted diagnostic ("`Process` is a builtin resource type owned by
`process`; choose another name or qualify your reference") instead of the
downstream `TYPE_RESOURCE_REQUIRES_RES`. This removes the confusing footgun
immediately without changing syntax. It does **not** fix the consistency issue or
the cross-package hazard — it is a stopgap.

**Phase 2b — the real fix, breaking (promote to a `plan-NN`):** make resource
type identity package-qualified everywhere it is currently a bare string —
resolution (`process::Process`), the resource table key, the registry accessors
(package-scoped, no first-match), `base_resource_name`/`state_type_name` parsing
(carry the `pkg::` prefix through `STATE`/thread-plane handling), NIR `Bind` /
drop wiring, `binary_repr`, spec, man pages, and the migration of all existing
resource spellings. This is large and breaking; it deserves its own phased plan
and a compatibility/migration decision (see Open Decisions). **This bug document
tracks the defect and Phase 2a; Phase 2b should be authored as a plan and linked
here.**

Rejected alternatives: (a) diagnostic-only — leaves the hazard (see Non-goals);
(b) auto-qualify bare resource names to the importing package — ambiguous when
multiple packages export same-named resources and still squats the bare
namespace; (c) reserve resource names only when the package is imported — still
inconsistent with value types and still cross-package-unsafe.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a compile-level regression test (per `tests/` conventions, e.g. a
      `cli_*`/rt-behavior fixture) reproducing: user `TYPE Process` (no imports)
      is wrongly rejected as a resource; `TYPE Foo` compiles. Confirm it fails
      today for the documented reason.
- [ ] Add a registry unit test asserting resource resolution is package-scoped
      (currently would document the bare first-match behavior).
- [ ] Finalize the blast-radius verdicts above against a fresh `grep`.

Acceptance: tests fail for the documented reason; audit complete.
Commit: —

### Phase 2a — interim diagnostic (non-breaking)

- [ ] Emit a clear "name collides with builtin resource" diagnostic at type
      declaration; keep the collision an error but make it legible.

Acceptance: `TYPE Process` yields the new diagnostic, not
`TYPE_RESOURCE_REQUIRES_RES`; suite green.
Commit: —

### Phase 2b — package-scoped resources (breaking; author as plan-NN)

- [ ] See Fix Design Phase 2b. Author `plan-NN`, land in its own phases.

Acceptance: `process::Process` resolves; bare `Process` does not; user
`TYPE Process` builds & runs; cross-package same-named resources do not
cross-wire; spec/man/goldens updated; full suite green.
Commit: —

## Validation Plan

- Regression test(s): the Phase 1 compile test + the registry package-scoping
  test.
- Runtime proof: the `/tmp/collide`-style program (user `TYPE Process`) builds
  and prints `42`; a cross-package same-named-resource program closes each with
  its own package's op.
- Doc sync: `language/15_resource-management.md`, `04_types.md`, resource man
  pages.
- Full suite: the project's acceptance/CI command(s).

## Open Decisions

- Delivery split — **recommended:** ship Phase 2a (diagnostic) under this bug
  now; author Phase 2b as a dedicated `plan-NN` (breaking, needs migration).
  vs. do it all here (too large for one bug).
- Migration for existing source — **recommended:** decide whether bare resource
  spellings get a deprecation window or a hard cutover, since it changes every
  program using resources.

## Summary

The engineering risk is almost entirely in Phase 2b: bare resource type-name
identity is load-bearing across resolver, RES ownership, codegen drop-wiring,
`binary_repr`, the registry, and user-facing syntax + goldens — so qualifying it
is a breaking, wide change best run as its own plan. Phase 2a (a legible
diagnostic) is a safe, contained interim. Nothing about resource *semantics*
should change — only how resource types are *named and scoped*. Not a regression
from the registry migration (File/Socket/Process were reserved long before it);
the registry only *added* the cross-package first-match half of the hazard.
