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

This is dangerous because the artifact is *silently corrupt*: the exporter build
does not detect the problem — it writes a `.mfp` containing a malformed type
entry, and the failure surfaces only when something reads that `.mfp` back
(during a later build of the same package's consumer, or `mfb pkg info`), with a
message that points at neither the offending type nor the source line.

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

When that `.mfp` is read back, the decoder expects a real record body for a
kind-1 entry and runs off the end of the section, raising `truncated binary
representation` (`src/binary_repr/util.rs:136`/`201`+; the failure is the exact
chain the STATE-type comment at `src/binary_repr/sections.rs:type_id` already
documents for a different trigger — a `STATE` composite that used to fall to this
same `_` fallback and was fixed by giving it a real kind-11 encoding).

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

- A package whose exported API names a type imported from a declared dependency
  builds to a valid `.mfp`; an executable installing both packages compiles,
  links, and runs, with the imported type resolving to the dependency's
  definition (fields, unions, and nested `List`/`Map` of it all intact).
- No `.mfp` ever encodes an imported type as a zero-field record. If the full
  feature is descoped, the exporter fails at build time with a diagnostic that
  names the offending type, its owning package, and the source location — and
  still never writes a corrupt artifact.

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

Add a type-table entry kind for an **imported/foreign type reference**: it
records the owning package's dependency name (as declared in `packages[]`) plus
the exported type name, rather than an inline definition. On the writer side,
`type_id`'s `_` arm, when it misses `self.ids`, consults the set of
imported-package exported types (the same table
`binary_repr::read_package_exports` already surfaces for type-checking) and, on a
hit, emits the new foreign-reference kind instead of a zero-field record. On the
reader/merge side (the executable's decode-and-merge), a foreign reference
resolves to the already-merged, identity-prefixed definition of that dependency's
type — both dependencies are present transitively, per the container-format
dependency list.

Rejected alternative — **inline the full foreign definition** into the package's
own `.mfp`: rejected because when the executable later merges *both* the
dependency and this package, the type would be defined twice under one identity
(a definition clash / duplicated data), and it breaks the single-definition
invariant the merge relies on.

Interim option (may ship first, as its own commit) — **a precise export-time
diagnostic**: detect a foreign type reaching serialization in `type_id`'s `_` arm
and fail with `a package's public API cannot reference type '<T>' imported from
'<pkg>'` at the offending source location. Strictly better than the current
silent-corrupt artifact, and a safe stopgap if the full encoding slips.

Expected shift in generated output: none for existing packages (new case only);
new fixtures gain new `.mfp` bytes.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a package-imports-package fixture pair under `tests/` following the
      existing package-dependency fixture conventions (a `pkga` exporting a
      `Box` type; a `pkgb` whose exported signature names `Box`; and an
      executable consuming both). Assert `pkgb` currently fails to build with
      `truncated binary representation`. Add the working-control fixture (`ok`,
      dependency-function use with no foreign type in the API).
- [ ] Confirm the executable-consumer path (merge) builds and runs today, to pin
      it as a non-goal regression guard.
- [ ] Record the blast-radius verdicts above in-file (done).

Acceptance: the new package fixture fails for the documented reason; the control
and executable-consumer fixtures pass; audit complete.
Commit: —

### Phase 2 — the fix

- [ ] Add the foreign-type-reference type-table kind (writer in
      `src/binary_repr/sections.rs`; decoder in the BR reader). Wire
      `type_id`'s `_` arm to emit it for an imported-package type instead of the
      zero-field record.
- [ ] Resolve the foreign reference during the executable decode-and-merge to
      the dependency's merged definition.
- [ ] (Or, if descoping to the interim:) emit the precise export-time diagnostic
      and stop before writing a `.mfp`.

Acceptance: the Phase 1 package fixture builds to a valid `.mfp`; the executable
consuming both packages compiles and runs; the control fixtures still build
byte-identically; nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate any `.mfp`/BR goldens the new fixtures introduce; confirm no
      pre-existing package `.mfp` output changed (byte-identity guard).
- [ ] Run the full `cargo test` / artifact-gate suite.
- [ ] Re-run the Failing Reproduction on every target in the matrix; confirm it
      now builds/links/runs (or errors cleanly, per the chosen scope).

Acceptance: full suite green; expected-output deltas are exactly the new
fixtures; the reproduction passes where it previously failed.
Commit: —

## Validation Plan

- Regression test(s): the package-imports-package fixture pair + control, under
  `tests/`.
- Runtime proof: the executable that installs both packages passes a `Box`
  (and a nested `List OF Box`) from `pkga` through `pkgb`'s exported function and
  prints the expected value.
- Doc sync: extend `src/docs/spec/architecture/05_binary-representation.md` (and
  `03_packages.md`) to state whether/how a package `.mfp` encodes a reference to
  a dependency's type; today neither documents it.
- Full suite: `cargo test` (workspace) + the artifact gate.

## Open Decisions

- Full foreign-reference encoding vs. interim export-time diagnostic — recommend
  landing the diagnostic first (safe, small) and the encoding as the complete
  fix. (§Fix Design)
- Whether a package may reference a *transitive* dependency's type (dep-of-dep),
  or only a direct dependency's — recommend direct-only first. (§Fix Design)

## Summary

The engineering risk is concentrated in the BR type-table format change: adding
a foreign-type-reference kind and resolving it at executable merge time, without
disturbing the byte-identity of existing package `.mfp` outputs or the
executable decode-and-merge path. The root cause is precise and already
half-documented in-tree (the STATE-type comment describes the identical
empty-record → `truncated binary representation` failure for a sibling trigger).
The safe interim — a precise export-time diagnostic replacing the silent corrupt
artifact — can land independently and immediately.
