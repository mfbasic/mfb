# bug-390: a package `.mfp` cannot serialize an imported package's type in its own public API (`truncated binary representation`)

Last updated: 2026-07-26
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/... (to be added — a package-imports-package fixture; see Phase 1)

A package build (`kind: "package"`) fails to produce a valid `.mfp` when one of
its **exported** functions or types names a type that was **imported from a
dependency package**. The build aborts with `error: truncated binary
representation`. A package may freely *call* a dependency's functions; it may
not *surface* a dependency's type in its own compiled interface.

The single correct behavior a fix produces: a package whose exported API
references a type owned by a declared dependency builds to a valid `.mfp`, and an
executable that installs both packages compiles, links, and runs — with the
imported type resolving to the dependency's definition. (At minimum, if the
feature is deliberately unsupported, the exporter must fail with a precise,
actionable diagnostic naming the type and its owning package, never emit a
corrupt `.mfp` that fails later with an opaque `truncated binary
representation`.)

This is dangerous because the failure is *opaque*: the `bug` package build aborts
with a bare `truncated binary representation` that names neither the offending
type (`Box`) nor the exported function (`describe`) nor a source line — the type
check of `describe` has already passed, so nothing points the author at the real
problem. (Reproduced 2026-07-26 on macos-aarch64 — see below. The earlier draft
of this section claimed the exporter *writes* a silently-corrupt `.mfp` that
fails only on a later read-back; that is wrong. The reproduction confirms the
build fails during the package's **own** ABI serialization, before any `.mfp` is
written — the corrupt entry never leaves the process. The danger is the useless
diagnostic, not a shipped corrupt artifact.)

References:

- `src/docs/spec/architecture/05_binary-representation.md` §"Decode-and-Merge of
  Package Dependencies" — defines dependency linking for the **executable** build
  path (decode each `.mfp` and merge its IR); does not cover a package build
  re-serializing a reference to a dependency's type.
- `src/docs/spec/architecture/03_packages.md` — package dependency install/verify.
- `src/docs/spec/package/01_container-format.md` — records a dependency list in
  package metadata (`packages/<name>.mfp`), implying package→package deps are an
  intended shape.
- Found while building the `examples/browser` example: a separate `display`
  package was to expose `render(dom AS Node, width) AS String`, where `Node` is a
  union defined in the `backend` package — blocked by this bug.

## Failing Reproduction

Two packages: `pkga` defines a type `Box`; `pkgb` imports `pkga`. `pkgb` builds
fine when it only *uses* a `pkga` function, and fails when it *names* `pkga`'s
`Box` in an exported signature.

```sh
MFB=./target/release/mfb          # a release build of this repo
root=$(mktemp -d)

# --- pkga: defines and exports a type ---
$MFB init-pkg "$root/pkga" >/dev/null
cat > "$root/pkga/src/lib.mfb" <<'MFB'
EXPORT TYPE Box
  n AS Integer
END TYPE
EXPORT FUNC make(v AS Integer) AS Box
  RETURN Box[v]
END FUNC
EXPORT FUNC plainValue() AS Integer
  RETURN 99
END FUNC
MFB
( cd "$root/pkga" && $MFB build >/dev/null )   # writes pkga.mfp

# --- CONTROL (works): pkgb uses a pkga function, no pkga type in its API ---
$MFB init-pkg "$root/ok" >/dev/null
mkdir -p "$root/ok/packages"; cp "$root/pkga/pkga.mfp" "$root/ok/packages/"
cat > "$root/ok/project.json" <<'JSON'
{ "name":"ok","version":"0.1.0","mfb":"1.0","kind":"package","description":"uses pkga plain",
  "sources":[{"root":"src","role":"package","include":["**/*.mfb"]}],
  "packages":[{"name":"pkga","version":"=0.1.0","source":"file:packages/pkga.mfp"}] }
JSON
cat > "$root/ok/src/lib.mfb" <<'MFB'
IMPORT pkga
EXPORT FUNC doubled() AS Integer
  RETURN pkga::plainValue() * 2
END FUNC
MFB
( cd "$root/ok" && $MFB build )                # -> "Wrote package to ./ok.mfp"

# --- BUG (fails): pkgb names pkga's Box type in an exported signature ---
$MFB init-pkg "$root/bug" >/dev/null
mkdir -p "$root/bug/packages"; cp "$root/pkga/pkga.mfp" "$root/bug/packages/"
cat > "$root/bug/project.json" <<'JSON'
{ "name":"bug","version":"0.1.0","mfb":"1.0","kind":"package","description":"uses pkga type",
  "sources":[{"root":"src","role":"package","include":["**/*.mfb"]}],
  "packages":[{"name":"pkga","version":"=0.1.0","source":"file:packages/pkga.mfp"}] }
JSON
cat > "$root/bug/src/lib.mfb" <<'MFB'
IMPORT pkga
EXPORT FUNC describe(b AS Box) AS String
  RETURN "box=" & toString(b.n)
END FUNC
MFB
( cd "$root/bug" && $MFB build )               # -> error
```

- Observed (the `bug` package):

  ```
  uses pkga - [Unsigned]
  Building bug (package) for macos-aarch64
  error: truncated binary representation
  ```

  No `.mfp` is written; the type-check of `describe` *passes* (the exporter
  resolves `Box` from `pkga`'s ABI exports), so the failure is purely in
  serialization, and the diagnostic names neither `Box` nor `describe`.

- Expected: either a valid `bug.mfp` that an executable installing both `pkga`
  and `bug` can consume, or a precise export-time error such as
  `a package's public API cannot reference type 'Box' imported from 'pkga'
  (src/lib.mfb:2)`.

Contrast cases that work today (regression guards):

- `ok` above — a package that *calls* `pkga::plainValue()` but keeps only
  built-in types in its own exported API — builds and writes `ok.mfp`.
- An **executable** that imports both `pkga` and `bug` and passes a `Box` between
  them builds and runs (the executable decode-and-merges both `.mfp`s into one
  IR, so `Box` becomes a merged local type — there is no cross-package type
  *reference* left to serialize). This is exactly why `examples/browser/app`
  (executable) can consume `backend`'s types while a sibling package cannot.
- `--unsigned` does not change the outcome; this is not a signing issue.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | macos-aarch64, release `mfb` | fails ✗ (package build); executable-consumer path works ✓ |

(The mechanism is target-independent — it is in the shared binary-representation
writer, not a backend — so Linux/other targets are expected to fail identically;
confirm during the fix.)

## Root Cause

`src/binary_repr/sections.rs` — `TypeTable::type_id`, the final `_` arm:

```rust
_ => {
    if let Some(id) = self.ids.get(name) {
        *id
    } else {
        self.add_entry(strings, "", name, 1, Vec::new())  // kind 1 = RECORD, zero fields
    }
}
```

When a package build serializes its own type table, every type it exposes is
run through `type_id`. A type **defined in the building unit** is present in
`self.ids` (its fields were registered), so it resolves to a real entry. A type
**imported from a dependency package** — `pkga`'s `Box`, used in `bug`'s exported
`describe` signature — is *not* in `self.ids`: the dependency's definitions are
read only to type-check the importer (via
`syntaxcheck::collect_package_functions` → `binary_repr::read_package_exports`),
never merged into the importer's IR type table. So `Box` misses `self.ids` and
falls to `add_entry(strings, "", name, 1, Vec::new())`, which interns it as an
**empty kind-1 RECORD** — a record type "that does not exist, with no fields."

The failure fires **within the same build**, not on a later read-back: after the
type table is populated, the package's ABI section is serialized by walking each
exported signature's type graph (`AbiSerializer` /
`TypeTable::serialize_type` → `serialize_record_type`,
`src/binary_repr/sections.rs:1048`). For the empty-record `Box` entry the payload
is zero-length, so reading the record's field-count `u32` off it
(`cursor_u32` → `checked_u32_at`, `src/binary_repr/util.rs:211`) fails with
`truncated binary representation`. The error propagates up through
`lower_package_project` / `build_package_binary_repr_bytes`
(`src/binary_repr/mod.rs:583`) → `target::write_package`
(`src/target/package_mfp/mod.rs:64`) → the build's `error: {err}` printer, so no
`.mfp` is ever written. This is the exact chain the STATE-type comment at
`src/binary_repr/sections.rs:type_id` already documents for a different trigger —
a `STATE` composite that used to fall to this same `_` fallback and was fixed by
giving it a real kind-11 encoding.

Confirmed reproduction (macos-aarch64, release `mfb` at main `4610f15e2`): the
`ok` control builds and writes `ok.mfp`; the `bug` package aborts with
`error: truncated binary representation` and writes no `bug.mfp` — exactly the
mechanism above.

Why the contrast cases are immune:
- The `ok` package never puts a foreign type in its exported API, so `type_id`
  is never called with `Box` during its serialization — no empty-record entry.
- The executable consumer never *writes* a `.mfp`; it decode-and-merges each
  dependency's IR (§05 spec), so `Box` is a fully-defined merged type at codegen
  time — there is no foreign reference to encode.

In short: **the BR type table has no encoding for "a type owned by another
package."** `type_id` silently degrades such a reference to a zero-field record
instead of either encoding a resolvable foreign reference or refusing at export.

## Goal

**Scope decided (2026-07-26, with the maintainer): the full foreign-reference
encoding, not the interim diagnostic-only descope.** The acceptance model is the
pA/pB/pC/app scenario below.

- A package whose exported API names a type imported from a declared dependency
  builds to a valid `.mfp`; an executable installing both packages compiles,
  links, and runs, with the imported type resolving to the dependency's
  definition (fields, unions, and nested `List`/`Map` of it all intact).
- The imported type is **re-exported by original ABI identity, never
  re-mangled**: the foreign reference a package writes carries the owning
  dependency's name, the exported type's original name, **and that type's ABI
  hash as computed by the owning package**. So the same underlying type surfaced
  through two different intermediary packages resolves to one identity and
  unifies (an `A` returned by pC's function can be passed to pB's function).
- **Only surfaced types are re-exported.** A dependency type that no exported
  function/type of the building package actually names is not written into the
  `.mfp` at all (it is never reached during ABI serialization).
- **ABI-incompatibility is a compile error.** If two intermediary packages were
  built against ABI-incompatible versions of the shared dependency (or the
  consumer resolves a dependency version incompatible with what an intermediary
  was built against), the consumer build must reject at dependency-verify /
  ABI-index validation time, not miscompile.
- No `.mfp` ever encodes an imported type as a zero-field record.

### Acceptance model (the fixture to build)

```
pA : EXPORT TYPE A ; TYPE B (private) ; EXPORT TYPE C
pB : IMPORT pA ; EXPORT FUNC takesA(a AS A) ...        (A in an exported ARGUMENT)
pC : IMPORT pA ; EXPORT FUNC makesA() AS A ...         (A in an exported RESULT)
app: IMPORT pB, pC ; wires pC::makesA() -> pB::takesA(...)
```

Required outcomes:
- pB and pC each build to a valid `.mfp` carrying a foreign reference to `pA::A`
  (original name + pA's ABI hash for A).
- pB does **not** re-export `A` unless it is surfaced (it is here); pB never
  re-exports `B` (never exported by pA) nor `C` (pA exports it, but pB names no
  `C` in its own API).
- app builds/links/runs: the `A` from `pC::makesA()` is the *same identity* as
  the `A` `pB::takesA()` expects (both resolve to pA's merged `A`), so the wiring
  type-checks and the value round-trips at runtime.
- app has access to `A` (pA's exported type) and **no** access to `B` (pA's
  private type — it is in no ABI surface).
- A deliberately ABI-incompatible `pA` under pB vs pC fails app's build.

**Open naming decision (blocks the resolver design):** whether app must
`IMPORT pA` to *name* `A` in its own source (identity-only re-export — values
flow/unify without importing pA, but naming requires the import), or whether
`IMPORT pB` alone brings `A` into scope under pA's original identity (true
namespace re-export, idempotent when pB and pC both surface it). Value flow
(wiring pC→pB) works in either model; this only governs whether app can write
`DIM x AS A` without importing pA. See Open Decisions.

### Non-goals (must NOT change)

- **Byte-identity of existing `.mfp` outputs** for packages that do *not*
  reference a foreign type. The current type-table encoding for built-ins and
  own-package types must be untouched; a fix adds a new case, it does not
  re-encode the working ones.
- **The executable decode-and-merge path** (§05) and its symbol-prefixing /
  identity model — packages consumed by executables already work and must keep
  working byte-for-byte where unaffected.
- **Silent degradation of any kind.** The tempting wrong fix — leave the `_`
  fallback emitting a kind-1 empty record but make the *reader* tolerate it (e.g.
  treat a zero-field record as opaque) — is forbidden: it would ship a `.mfp`
  whose imported type has no fields, silently degrading every consumer exactly as
  the STATE comment warns. Equally forbidden: "fixing" the reproduction by
  rewriting the repro to avoid the foreign type, or by moving the shared type
  into the consumer — those are the *workarounds*, not the fix.

## Blast Radius

Searched: `grep -rln '"kind"[: ]*"package"'` across `tests/` cross-referenced
with a `"packages"` dependency array — **no in-tree package depends on another
package**, so nothing in the repo exercises (or regresses from) this path today.
It is a latent capability gap, not an active corruption of shipping artifacts.

- `src/binary_repr/sections.rs:TypeTable::type_id` (`_` arm) — the defect; fixed
  by this bug.
- `src/binary_repr/sections.rs` type-table **decoder** / `util.rs` readers — must
  learn to decode whatever new foreign-reference kind the writer emits; in scope.
- `bindings/libsnd` and other native-resource packages — a *related* package
  boundary the STATE fix (kind-11) already addressed; **unaffected** here because
  they expose built-in resource types, not a sibling package's user type.
- `examples/browser` (this worktree, not committed to `src/`) — the concrete
  consumer that wants a `display` package taking `backend`'s `Node`; **out of
  scope for the fix**, it is the motivating use case and will adopt whichever
  structure lands (today it works around the gap by keeping rendering in the
  `backend` package or the executable).

## Fix Design

Add a type-table entry kind for an **imported/foreign type reference**. Unlike an
inline definition it records three things: the owning package's dependency name
(as declared in `packages[]`), the exported type's **original name**, and **that
type's ABI hash as computed by the owning package**. The hash is the load-bearing
field — the name alone cannot detect a version mismatch.

**Writer.** `type_id`'s `_` arm, when it misses `self.ids`, consults the imported
packages' exported types (the tables `binary_repr::read_package_exports` /
`read_package_type_exports` already surface for type-checking) and, on a hit,
emits the new foreign-reference kind instead of a zero-field record. Crucially,
the export's `sig_hash` (`src/binary_repr/mod.rs`, ABI_INDEX) must be computed so
the foreign type contributes **pA's identity for A**, not a locally re-walked
structure — this is the "original ABI hash, no re-mangling" requirement. Two
different intermediary packages surfacing the same `pA::A` therefore produce
identical hash contributions for it, which is what lets a consumer unify them.

**Selective.** Because the foreign-reference entry is only ever created when
`type_id` is reached while serializing the building package's *own* exported
signatures, a dependency type that no exported func/type names is never written —
no extra gate needed (see acceptance model: `C` is not re-exported by pB).

**Reader / merge (executable consumer).** A foreign reference resolves to the
already-merged, identity-prefixed definition of that dependency's type. Both (all)
dependencies must be present — including a **transitive** dependency: in the
acceptance model app declares only pB and pC, but pA is pulled in transitively via
the container-format dependency list so there is a single merged `pA::A` for both
foreign references to resolve to. *(This upgrades the earlier "direct-only deps
first" Open Decision to "transitive is required" — the pA/pB/pC/app model cannot
work without it.)*

**Dependency verification (the compat gate).** `validate_abi_index` already
recomputes each export's `sig_hash` from the function table and rejects a
per-symbol mismatch (see the ABI_FORMAT_VERSION comment at
`src/binary_repr/mod.rs:89`). Wire the foreign reference through this recompute so
that, at consume time, a foreign ref is checked against the consumer's *resolved*
`pA::A`: if an intermediary was built against an ABI-incompatible `pA` than the
consumer resolves (or than a sibling intermediary was), the recomputed hash won't
match the stored `sig_hash` and the build fails — this is how "pB and pC
disagreeing on pA's version does not compile" is enforced, reusing the existing
mechanism rather than inventing a new one.

Rejected alternative — **inline the full foreign definition** into the package's
own `.mfp`: rejected because when the executable later merges *both* the
dependency and this package, the type would be defined twice under one identity
(a definition clash / duplicated data), it breaks the single-definition invariant
the merge relies on, and it defeats the ABI-hash compat check (an inlined copy
carries no link back to the owning package's version).

Fallback (NOT the chosen scope; recorded only in case the encoding proves
infeasible mid-implementation) — **a precise export-time diagnostic**: detect a
foreign type reaching serialization in `type_id`'s `_` arm and fail with
`a package's public API cannot reference type '<T>' imported from '<pkg>'` at the
offending source location. Strictly better than the current opaque
`truncated binary representation`, but it does *not* deliver the title capability;
use only if the full encoding is abandoned, and flag that regression explicitly.

Expected shift in generated output: none for existing packages (new case only);
new fixtures gain new `.mfp` bytes.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Build the pA/pB/pC/app fixture set under `tests/` (Acceptance model
      above) following the existing package-dependency fixture conventions: `pA`
      exporting `A` and `C` plus a private `B`; `pB` naming `A` in an exported
      argument; `pC` naming `A` in an exported result; `app` importing pB+pC and
      wiring `pC::makesA()` → `pB::takesA(...)`. Assert `pB`/`pC` currently fail
      to build with `truncated binary representation`. Add a working control
      (dependency-function use with no foreign type in the API).
- [ ] Confirm the executable-consumer path (merge) builds and runs today for the
      already-working shapes, to pin it as a non-goal regression guard.
- [ ] Record the blast-radius verdicts above in-file (done).

Acceptance: the new package fixtures fail for the documented reason; the control
and executable-consumer fixtures pass; audit complete.
Commit: —

### Phase 2 — the fix

- [ ] Add the foreign-type-reference type-table kind (dep name + original type
      name + owning-package ABI hash) — writer in `src/binary_repr/sections.rs`,
      decoder in the BR reader. Wire `type_id`'s `_` arm to emit it for an
      imported-package type instead of the zero-field record.
- [ ] Compute exported `sig_hash` so a foreign type contributes pA's identity for
      A (original hash, no re-mangle), and route the foreign reference through
      `validate_abi_index`'s recompute so an ABI-incompatible dependency version
      is rejected at consume time.
- [ ] Resolve the foreign reference during the executable decode-and-merge to the
      dependency's merged definition, pulling the owning package in transitively
      when a consumer declares only the intermediaries.
- [ ] Resolve the naming decision (Open Decisions) and implement the chosen
      import/scoping rule for *naming* a re-exported type in the consumer.

Acceptance: the Phase 1 package fixtures build to valid `.mfp`s; app compiles,
links, and runs (value round-trips pC→pB); `B` and unused `C` are absent from
pB's surface; an ABI-incompatible pA fails app's build; the control fixtures
still build byte-identically; nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate any `.mfp`/BR goldens the new fixtures introduce; confirm no
      pre-existing package `.mfp` output changed (byte-identity guard).
- [ ] Run the full `cargo test` / artifact-gate suite.
- [ ] Re-run the Failing Reproduction on every target in the matrix; confirm it
      now builds/links/runs.

Acceptance: full suite green; expected-output deltas are exactly the new
fixtures; the reproduction passes where it previously failed.
Commit: —

## Validation Plan

- Regression test(s): the package-imports-package fixture pair + control, under
  `tests/`.
- Runtime proof: `app` wires `pC::makesA()` → `pB::takesA(...)` (and a nested
  `List OF A`) and prints the expected value, proving the two foreign references
  resolve to one `pA::A` identity.
- Doc sync: extend `src/docs/spec/architecture/05_binary-representation.md` (and
  `03_packages.md`) to state whether/how a package `.mfp` encodes a reference to
  a dependency's type; today neither documents it.
- Full suite: `cargo test` (workspace) + the artifact gate.

## Open Decisions

- **RESOLVED — scope:** full foreign-reference encoding (not the diagnostic-only
  descope). (§Goal)
- **RESOLVED — transitive deps:** *required*, not deferred. The pA/pB/pC/app
  model needs the owning package (pA) pulled in transitively so both foreign
  references resolve to one merged `pA::A`. (§Fix Design)
- **OPEN — naming/scoping of a re-exported type:** must app `IMPORT pA` to *name*
  `A` in its own source (identity-only re-export — values flow/unify without it),
  or does `IMPORT pB` bring `A` into scope under pA's original identity (true
  namespace re-export, idempotent across pB and pC)? Value flow works either way;
  this only governs `DIM x AS A` in the consumer. Blocks the resolver design in
  Phase 2. (§Goal → "Open naming decision")

## Summary

The engineering risk is concentrated in the BR type-table format change: adding a
foreign-type-reference kind — carrying the owning dependency's name, the type's
original name, and its owning-package ABI hash — computing exported `sig_hash` by
that original identity (no re-mangling), resolving the reference at executable
merge time (including a transitive owning package), and routing it through the
existing `validate_abi_index` recompute so an ABI-incompatible dependency version
is rejected rather than miscompiled — all without disturbing the byte-identity of
existing package `.mfp` outputs or the executable decode-and-merge path. The root
cause is precise, reproduced (macos-aarch64, 2026-07-26), and already
half-documented in-tree (the STATE-type comment describes the identical
empty-record → `truncated binary representation` failure for a sibling trigger).
The one remaining product decision is the naming/scoping rule for a re-exported
type (§Open Decisions).
