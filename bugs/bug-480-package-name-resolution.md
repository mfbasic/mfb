# bug-480: package name resolution is broken in two directions — a source package's exports resolve to nothing, and an imported package's value types resolve without their prefix

Last updated: 2026-08-31
Effort: x-large (multi-day; four phases, two independently landable)
Severity: HIGH (was MEDIUM; raised when Defect B absorbed bug-473 and bug-481's build failure)
Class: Correctness

Status: Open
Regression Test: `tests/rt-behavior/packages/source-package-dependency-rt/` (new, Phase 1);
`tests/syntax/net/qualified-enum-name-accepted/` (new, Phase 3)

Two defects, one namespace. Names that cross a package boundary are resolved
through a flat, package-blind table, and it fails in both directions:

- **Defect A — names that should resolve, don't.** A dependency declared by
  *source directory* is found, but none of its exported functions acquire a type,
  so every call into it is `Unknown` and the build fails at the caller.
- **Defect B — names that shouldn't resolve, do.** An imported package's record,
  union and enum names resolve **without** their package prefix, which they must
  require; and the correct prefixed spelling of an enum member or union variant
  **fails**, escaping the front end into an unlocated NIR error.

They are filed together because they are the same missing package dimension, and
because the Defect B fix (a package-keyed type namespace) changes the table
Defect A must populate. Fixing either alone means building that table twice.

**The single correct behavior a fix produces:** a name is resolved by
`(package, name)`. A source dependency resolves its exported signatures exactly
as the equivalent `.mfp` does; an imported value type resolves only when written
with its import prefix, and always when written with it.

---

## The Governing Rule

Two lines, covering every kind of name — variables and constants, functions,
records, unions, union variants, enums, enum members, resource types:

| where the name is defined | prefix |
|---|---|
| **locally** — in the current project/package | not needed |
| **imported** | **required** |

`net::PingStatus.Ok` from a consumer, `PingStatus.Ok` inside `net` itself.
`http::Response` as an annotation, `json::JsonBool` as a `CASE` pattern,
`term::LineStyle.Light` as an enum member, `math::pi` as a constant.

Three clarifications the rule's brevity leaves open. Each matches how the
compiler already behaves for the kinds that comply:

- **The prefix is the import *binding*, not the package name.** `IMPORT io AS console`
  makes it `console::flush()` (`tests/syntax/packages/package-import-as/src/lib.mfb`),
  and `IMPORT package_comparable_types AS comparable` makes the type
  `comparable::Box`
  (`tests/syntax/packages/package-comparable-import-invalid/src/main.mfb:5`).
- **It applies to the head of a path, not to members.** `u.host` on a `net::Url`
  needs no prefix on `host`; `net::PingStatus.Ok` prefixes the enum, not the
  member. "Imported" governs name *resolution*, not field selection.
- **`IMPORT self` stays optional.** A package's own members are defined locally,
  so bare is correct inside it; `self::worker` remains the explicit spelling, not
  a required one (`canonical_import_name` already maps `self.X` to bare `X`,
  `src/ir/lower.rs:3047`).

### What already complies, and what does not

Measured at HEAD (744c7c175), `target/release/mfb build` on scratch `mfb init`
projects, macos-aarch64 — bare spelling, used outside the owning package:

| kind | example | today | rule |
|---|---|---|---|
| function | `net::toUrl` | rejected — `SYMBOL_UNKNOWN_IDENTIFIER` | ✅ complies |
| constant | `math::pi` | rejected — `SYMBOL_UNKNOWN_IDENTIFIER` | ✅ complies |
| resource type | `tcp::Socket` | rejected — `SYMBOL_UNKNOWN_TYPE` | ✅ complies |
| record type | `http::Response` | **accepted** | ❌ must reject |
| enum type | `net::PingStatus` | **accepted** | ❌ must reject |
| enum member | `PingStatus.Ok` | **accepted** | ❌ must reject |
| union type | `json::Json` | **accepted** | ❌ must reject |
| union variant | `json::JsonBool` | **accepted** | ❌ must reject |

**The rule is already true for everything except value types.** Functions,
constants and resource types are done; records, unions and enums — and their
members and variants — are the entire remaining scope of Defect B.

### This is finishing bug-441, on bug-441's own premise

The resource row complies because bug-441/plan-97 made it so. The rationale is
still in the source (`src/codegen/builtins/process/mod.rs:57-64`):

> The `Process` resource's **package-qualified type identity** … (bug-441 /
> plan-97: resources are addressed `process::Process`, not bare `Process`, so a
> user `TYPE Process` no longer collides).

and bug-441 justified that work by asserting the other kinds were already there:

> Every other builtin surface is package-scoped: functions are `process::spawn`,
> and builtin value types (records/unions/enums) are spelled qualified in source
> (`net::Url`, `process::Process`).

True of the documentation, false of the compiler. `mfb man <pkg> types` renders
every type qualified — `net::Url`, `http::Stream`, `process::Signal`, and a
union's variants as `tcp::Socket | tls::Socket` — but nothing enforces it.
bug-441 hardened the one kind that had already collided and left the rest on that
premise. **bug-481 is where the premise came due:** `IMPORT http` and
`IMPORT process` cannot coexist in one project, because `http::Stream` (a union)
and `process::Stream` (an enum) both claim the bare name `Stream`. This bug's
Phase 4 is that fix.

---

## Defect A — a source-package dependency's exports resolve to nothing

A project may depend on a package by **source directory**
(`"source": "file:packages/<name>"`, resolved at `packages/<name>/project.json`)
instead of by compiled `.mfp`. The resolver finds it — the import does not raise
`IMPORT_PACKAGE_NOT_INSTALLED` — but none of the package's exported functions
acquire a type, so every call into it evaluates to `Unknown` and the build fails
downstream with an error that points at the *caller*, never at the unresolved
package.

The same package, built to a `.mfp` and installed at `packages/<name>.mfp`,
works perfectly. Only the source form is broken.

This matters beyond convenience. A committed `.mfp` silently goes stale whenever
a built-in resource type is re-qualified (a known failure mode: the fixture then
fails native lowering with "native inlined field size not available for type
'<Resource>'" while its `.ir`/`.ast` goldens keep passing, so its `.ncodesum`
stops being verified). The standing remedy for that is "convert the committed
`.mfp` to an in-tree source package so it recompiles every build" — and that
remedy does not currently work. **Phase 4's rule migration will re-qualify type
names tree-wide, which is exactly the trigger for that staleness**, so Defect A
must be fixed before Phase 4 lands or the migration will strand every committed
`.mfp` in the tree.

### Failing Reproduction

Two files and two manifests. Nothing thread- or resource-specific — a plain
`FUNC` returning an `Integer` is enough.

```
app/packages/tiny/project.json
  {"name":"tiny","version":"0.1.0","mfb":"1.0","kind":"package","description":"tiny",
   "sources":[{"root":"src","role":"lib","include":["**/*.mfb"]}],"targets":["native"]}

app/packages/tiny/src/lib.mfb
  EXPORT FUNC answer() AS Integer
    RETURN 42
  END FUNC

app/project.json
  {"name":"spapp","version":"0.1.0","mfb":"1.0","kind":"executable",
   "sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],
   "packages":[{"name":"tiny","version":"=0.1.0","source":"file:packages/tiny"}],
   "entry":"main","targets":["native"]}

app/src/main.mfb
  IMPORT io
  IMPORT tiny

  FUNC main AS Integer
    io::print(toString(tiny::answer()))
    RETURN 0
  END FUNC
```

```
mfb build app
```

- Observed:
  ```
  main.mfb:5 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: function call argument type does not match parameter type
                 Call to `toString` has argument type(s) (Unknown), expected Integer, Float[, Byte], …
  ```
  `tiny::answer()` is `Unknown`. The diagnostic blames `toString`, and nothing
  anywhere says the package failed to load.
- Expected: builds, prints `42`.

Contrast cases:

- **The `.mfp` form works.** `mfb build app/packages/tiny` then copying `tiny.mfp` to `app/packages/` and switching the manifest to `file:packages/tiny.mfp` builds and runs. Verified with the real `connworker` package for `examples/network-server`, whose exports (plain `FUNC`s *and* `EXPORT ISOLATED FUNC` thread entries) all resolve through the `.mfp`.
- **The import itself resolves.** Removing the package directory entirely gives `IMPORT_PACKAGE_NOT_INSTALLED` naming both candidate paths, so the source directory *is* being found and accepted — the failure is later, in loading its interface.
- **Not export-kind-specific.** Measured on the `connworker` package: plain `EXPORT FUNC`s (`tickNanos`, `counterText`) and `EXPORT ISOLATED FUNC` thread entries all came back `Unknown` — 7 errors across 5 call sites.

| Environment | config | Result |
| --- | --- | --- |
| macOS aarch64, release `mfb` at `9782bc60b` | `file:packages/tiny` (source) | fails ✗ |
| macOS aarch64, release `mfb` at `9782bc60b` | `file:packages/tiny.mfp` (compiled) | works ✓ |

Not expected to be platform-dependent — this is front-end resolution, before any
target selection — but that is an assumption, not a measurement.

### Root Cause (A)

Not yet established. Hypotheses, most likely first:

1. **The source package is located but never compiled into an interface.**
   `src/resolver/packages.rs` finds `packages/<name>/project.json` (the `:410`
   comment and the two-path diagnostic prove the lookup exists) but the path that
   builds a source dependency's exported-signature table is missing or is only
   wired for the `.mfp` decode path. Confirm by instrumenting where an `.mfp`'s
   public API metadata is loaded and checking whether the source path reaches an
   equivalent.
2. **The interface is built but registered under the wrong key**, so lookups of
   `tiny::answer` miss and fall back to `Unknown`. Confirm by dumping the
   resolved symbol table for both dependency forms and diffing.
3. **Ordering**: the dependency is compiled *after* the importer is type-checked.
   Confirm from the build driver's phase order.

`Unknown` propagating silently rather than raising at the unresolved call is a
second, separable defect — see Goal.

---

## Defect B — imported value types resolve bare, and fail when qualified

Absorbed from bug-473 (retired 2026-08-31,
`bugs/skipped/bug-473-qualified-enum-name-escapes-to-nir-unlocated.md`). Two
halves, both measured at 744c7c175:

### B1 — the qualified spelling fails

Writing an imported enum member or union variant with its prefix — the spelling
the rule requires — escapes the front end and dies in NIR:

```basic
IMPORT net
IMPORT io

FUNC main AS Integer
  LET result = net::ping("127.0.0.1", 1000)
  MATCH result.status
    CASE net::PingStatus.Ok
      io::print("up in " & toString(result.rttMs) & " ms")
    CASE ELSE
      io::print("unreachable")
  END MATCH
  RETURN 0
END FUNC
```

```
Building p108_ping (executable) for macos-aarch64
error: NIR local reference 'net.PingStatus' does not resolve
```

No file, no line, no error code, no caret. The same for a union variant:
`CASE json::JsonBool` → `error: NIR local reference 'json.JsonBool' does not resolve`.
Same class as bug-466: an unresolved name is passed downward instead of refused,
and the back end reports it in its own vocabulary.

Position matters, and the inconsistency is itself diagnostic:

- As a **type**, the qualified form works: `LET s AS net::PingStatus = r.status` builds.
- In a **`LET` initialiser**, `LET s = net::PingStatus.Ok` *is* located, but says
  `TYPE_UNKNOWN_VALUE` — "Initializer for binding `s` does not have a known type",
  naming neither the enum nor the qualifier.
- In a **`CASE`**, it escapes entirely.

### B2 — the bare spelling resolves

Everything in the ❌ rows of the compliance table above: `AS Response`, `AS Url`,
`AS Json`, `CASE JsonBool`, `PingStatus.Ok` all compile from a consumer today.

### Root Cause (B)

The qualifier is canonicalized on the **type** path and nowhere else.

`canonical_import_name` (`src/ir/lower.rs:3033`) rewrites `net::X` to `net.X`,
and `canonical_import_type` (`src/ir/lower.rs:3020`) applies it to every
`ParameterType` — which is why the annotation `AS net::PingStatus` resolves.

The enum-member expression path never calls it:

- `src/ir/verify/values.rs:637` — `check_member_access` looks up
  `self.enums.get(&ParameterType::declared(name))` with the **raw** target name.
  For `net.PingStatus` that misses, `infer_type` then returns `None`, and the
  function returns without emitting anything. This is the front-end gate that
  should have fired.
- `src/ir/lower.rs:2710` — the `MemberAccess` type-inference arm makes the same
  raw-name `enums` lookup, so the expression types as nothing.

The name then survives to `src/target/shared/validate/body.rs:560`, which reports
it in NIR's vocabulary with no source location.

Underneath both halves is the flat namespace: `TypeIndex` (`src/ir/lower.rs:4486`)
keys records, enums and union variants by **bare declared name**, and folds
imported packages' types into the same map (`src/ir/lower.rs:4587`+) with
`entry(..).or_insert_with(..)` — first wins. There is no package dimension to key
on. That is why the bare consumer spelling works at all, and why two packages
exporting the same value-type name collide (bug-481).

---

## Goal

- Defect A's reproduction builds and prints `42`.
- A source dependency and the `.mfp` built from the same sources produce the same program (same `.ir`, modulo the dependency-form metadata).
- `CASE net::PingStatus.Ok` and `CASE json::JsonBool` compile, in every expression position — `CASE`, `LET` initialiser, comparison, argument.
- A bare imported record/union/enum name from outside its package is refused with a located, coded diagnostic that **prints the qualified spelling**.
- Nothing unresolved reaches NIR: `NIR local reference '…' does not resolve` is unreachable from source.
- An export that genuinely cannot be resolved raises a located diagnostic naming *the package and the member*, instead of leaking `Unknown` into an unrelated argument-type error at the call site.
- `IMPORT http` and `IMPORT process` coexist (bug-481).

### Non-goals (must NOT change)

- The `.mfp` dependency path, which works today.
- Package signing/verification: a source dependency is unsigned by construction and `mfb audit`'s treatment of it must not become laxer for `.mfp` dependencies as a side effect.
- Do not "fix" Defect A by deleting the source-package lookup and making the form an error. It is specified in two places (see References) and is the standing remedy for stale committed `.mfp`s; removing it would need a spec change and would leave that remedy with nothing behind it.
- Do not change `examples/network-server` to the source form until Phase 2 lands — it is deliberately on the `.mfp` + `prepare_network_server()` path, mirroring `examples/browser`.
- Do not require `self::` inside a package. Own members are defined locally; the explicit form stays optional.
- Do not make member/field selection prefixed. The rule governs the head of a path only.

## Blast Radius

**Defect A:**

- `src/resolver/packages.rs` — the lookup and the local-package manifest read (`:57`, `:133-138`, `:410`). Primary suspect.
- The build driver's dependency-compilation ordering — **audit required**; hypothesis 3.
- `tests/**` — **audit required**: `grep -rn '"source": "file:' --include=project.json tests/ examples/` returns **only `.mfp` paths**, in every fixture in the tree. So no existing test covers the source form, which is why this is invisible. That absence is itself the finding: the specified form has zero coverage.
- `mfb.lock` / `mfb audit` — `07_cli-reference.md:506` says source-package dependencies get no lockfile state. **Audit required**: confirm the fix does not start writing one.
- `examples/browser` — unaffected; it is on the `.mfp` path and stays there.

**Defect B — the namespace:**

- `src/ir/lower.rs:4486` — `TypeIndex`, the flat bare-name type namespace. The load-bearing change.
- `src/ir/lower.rs:3020`/`:3033` — `canonical_import_type`/`canonical_import_name`.
- `src/ir/verify/values.rs:637` — `check_member_access`, the gate that declines to fire.
- `src/ir/lower.rs:2710` — the `MemberAccess` type-inference arm.
- `src/target/shared/validate/body.rs:560` — where the unlocated message is produced.
- The builtin-source injection path — `SYMBOL_DUPLICATE_TOP_LEVEL` (`src/rules/table.rs`) currently fires across two packages' injected sources (bug-481).

**Defect B — the corpus migration.** Measured, not estimated:

| surface | bare refs to an imported builtin type | scope |
|---|---|---|
| `examples/`, `tests/`, `src/docs/` `.mfb` | **1139** | 128 of 1373 files |
| rendered `mfb man` example blocks | **339** | 174 of 890 blocks |

Census method: the 93 exported type names from `mfb man <pkg> types` across all
28 builtin packages, matched word-boundary in each file, counting a hit only
where the file `IMPORT`s the owning package and does not itself declare that
name. Heaviest names: `Certificate` 217 and `Hash` 199 (both `crypto` enums, used
as `Hash.Sha256`), then `AsymmetricCipher` 76, `Float3` 59, `Json` 42,
`LineStyle` 35, `DateTime` 34.

The migration is mechanical and self-verifying: at the point of rejection the
compiler knows which package owns the name, so the diagnostic carries its own
fix and "everything still builds" is the completeness check. Man examples are
compiled now (bug-472 / plan-108-C), so that surface is gated rather than
trusted.

Man **prose** is not covered by that census and must be swept by hand — e.g.
`src/codegen/builtins/net/func_ping.rs` uses bare `PingStatus.Ok` in its
description text at `:24`, `:34-39`, `:96`, `:155` as well as in the examples at
`:81`, `:83`, `:127`. plan-108-C rewrote those from the qualified form to the
bare form; under the rule that edit was backwards and must be reverted.

## Phases

Phase 1–2 are Defect A and are independently landable. Phase 3 is a strict
widening. Phase 4 is the breaking change and depends on all three.

### Phase 1 — Defect A: failing test + root cause (no behavior change)

- [ ] Add `tests/rt-behavior/packages/source-package-dependency-rt/` — the minimal reproduction above, asserting it prints `42`. Confirm it fails today.
- [ ] Add the paired `.mfp` fixture so the two forms are compared by the suite, not by hand.
- [ ] Decide between hypotheses 1–3 with a measurement; record which, and the evidence, here.
- [ ] Complete the Defect A blast-radius audit, especially the lockfile/audit question.

Acceptance: the new fixture fails for the documented reason; the root cause is named with evidence.
Commit: —

### Phase 2 — Defect A: the fix

- [ ] Fix the identified layer.
- [ ] Make an unresolved package member raise a located diagnostic rather than yielding `Unknown`.
- [ ] Re-measure the user-package half of the compliance table, which Phase 1 could not: bare vs qualified for a *user* package's exported record/enum/union. Today even the qualified `comparable::Box` fails with `TYPE_UNKNOWN_VALUE`, and `tests/syntax/packages/package-comparable-import-invalid/golden/build.log` **pins that failure as its expected output** — a golden pinning a build failure, which must be re-examined here rather than carried forward.

Acceptance: Phase 1 fixtures pass; the `.mfp` path is unchanged.
Commit: —

### Phase 3 — Defect B1: make the qualified form resolve (strict widening)

- [ ] Run `canonical_import_name` on a `MemberAccess` target before the `enums` lookup, in both sites (`src/ir/verify/values.rs:637`, `src/ir/lower.rs:2710`).
- [ ] Do the equivalent for a union variant in a `CASE` pattern.
- [ ] Add `tests/syntax/net/qualified-enum-name-accepted/` and a union-variant twin.
- [ ] Assert the NIR escape is closed: a qualified name that genuinely cannot resolve gets a located, coded diagnostic naming the enum, not `NIR local reference … does not resolve`.

Every program that compiles today still compiles. This must land before Phase 4:
a corpus cannot be migrated to a spelling the compiler rejects.

Acceptance: `CASE net::PingStatus.Ok` and `CASE json::JsonBool` build and run; no source program can reach the NIR message.
Commit: —

### Phase 4 — Defect B2: require the prefix (breaking; needs its own plan)

Scope is exactly the five non-compliant rows — records, unions, union variants,
enums, enum members. Functions, constants and resource types already comply.

- [ ] Key `TypeIndex` by `(package, name)` (`src/ir/lower.rs:4486`) and stop injecting builtin package sources into one flat top-level namespace.
- [ ] Reject a bare imported record/union/enum name from outside its package, with a located diagnostic that prints the qualified spelling — the shape `SYMBOL_UNKNOWN_TYPE` already produces for bare `Socket`.
- [ ] Decide explicitly, and record here: the requirement applies to type **annotations** as well as expressions. It has to — `AS Response` is exactly as ambiguous as `CASE JsonBool`, and leaving annotations bare keeps the collision that breaks bug-481.
- [ ] Migrate the corpus: 1139 `.mfb` refs across 128 files, 339 man-example refs across 174 blocks, plus the hand-swept man prose.
- [ ] Revert plan-108-C's `net::PingStatus` → `PingStatus` edit in `src/codegen/builtins/net/func_ping.rs`.
- [ ] Close bug-481: `IMPORT http` + `IMPORT process` builds. Re-report a builtin-source name collision against the developer's `IMPORT` lines, never against a line in `builtins/<pkg>.mfb`.
- [ ] Sync the spec: `13_modules-and-packages.md` must state the two-line rule.

Acceptance: the compliance table is all ✅; bug-481's repro builds; full suite green.
Commit: —

### Phase 5 — validation

- [ ] Full `cargo test --no-fail-fast`; `cargo check --all-targets`; `scripts/artifact-gate.sh all`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh <pkg> --run` across every package.
- [ ] Consider converting a committed-`.mfp` fixture to the source form now that it works, closing the stale-`.mfp` failure mode at its root. Scope that separately — it is a tree-wide change, not part of this fix.

Acceptance: full suite green.
Commit: —

## Validation Plan

- Regression tests: the paired source/`.mfp` fixtures (A); the qualified enum-member and union-variant fixtures (B1); a bare-imported-type rejection fixture per kind (B2); a `http` + `process` coexistence fixture (bug-481).
- Runtime proof: Defect A's reproduction prints `42`; Defect B1's `CASE net::PingStatus.Ok` program runs.
- Doc sync: `13_modules-and-packages.md:144` for the source form; the two-line rule for the prefix; `mfb man variable`/`link` if either describes name scoping.
- Full suite: `cargo test --no-fail-fast`, `scripts/artifact-gate.sh all`, `scripts/test-accept.sh`, `scripts/man-run-examples.sh`.

## Open Decisions

- **Is the source form meant to support `EXPORT ISOLATED FUNC` thread entries?** A source dependency is recompiled into the importer's build, and `13_modules-and-packages.md:148` says an isolated function starts "in a fresh instance of its package" — which is well-defined for a source package too. Recommended: yes, same semantics. Worth settling in Phase 1, because it decides whether `examples/network-server` could ever drop its two-step build.
- **Does Phase 4 need a deprecation window, or is it a hard cutover?** plan-97 did the resource migration as a hard cutover and it held. Recommended: hard cutover, for the same reason — a warning period doubles the resolution paths and the corpus has to be swept either way.
- **bug-481 interim.** Phase 4 is multi-day and `IMPORT http` + `IMPORT process` is broken now. Renaming `process::Stream` (to `StdStream` or `Fd`) unblocks it in an afternoon but is itself a breaking surface change and leaves the next collision to whoever hits it. Decide whether to take the interim rename or wait for Phase 4.

## References

- `bugs/skipped/bug-473-qualified-enum-name-escapes-to-nir-unlocated.md` — retired into Defect B; keeps the original reproduction record.
- `bugs/bug-481-http-and-process-cannot-be-imported-together.md` — the flat namespace as a hard build failure. Closed by Phase 4.
- `bugs/completed/bug-441-resources-not-package-scoped.md` + `planning/completed/plan-97-resources-package-scoped.md` — the same migration, applied to resources only. This bug is the unfinished remainder.
- `bugs/bug-466-unknown-field-type-escapes-to-codegen.md` — the same class as B1 (an unresolved name passed downward instead of refused).
- `mfb spec language modules-and-packages` (`src/docs/spec/language/13_modules-and-packages.md:144`) — "A package may import a **source package** or an `.mfp` package". The source form is specified, not accidental.
- `mfb spec tooling cli-reference` (`src/docs/spec/tooling/07_cli-reference.md:493,506`) — "a dependency missing both a `packages/<name>.mfp` and a source-package …", "source-package dependencies get no state". The tooling models it as a first-class dependency kind.
- `src/resolver/packages.rs:410` — the resolver comment "No packages/shape.mfp and **no packages/shape/project.json**", and the `IMPORT_PACKAGE_NOT_INSTALLED` message that names both locations. The lookup is deliberate.
- `tests/syntax/packages/package-import-as/src/lib.mfb` — `IMPORT io AS console`, the aliasing the rule composes with.
- Found: Defect A while adding `--thread` to `examples/network-server` (the source form was tried first to avoid a two-step build); Defect B during plan-108-C, compiling `net`'s man examples for the first time.

## Summary

Defect A may be a *missing* path rather than a broken one — hypothesis 1 would
mean source-package dependencies have never worked, which the total absence of
tree coverage quietly supports. That would make it a feature gap wearing a bug's
clothes. Phase 1 exists to answer that before any code moves.

Defect B is not in doubt: it is measured, it is finishing a migration the tree
already started and documented, and it is already costing a shipped capability
(bug-481). Its risk is entirely in the size of the corpus sweep, which is
mechanical and compiler-checked.

The ordering constraint is the thing to hold onto: **Defect A must be fixed
before Phase 4's migration**, because re-qualifying type names tree-wide is
precisely the trigger that makes committed `.mfp`s go stale, and converting them
to source packages is the remedy that Defect A currently breaks.
