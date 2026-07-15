# plan-46-A: `libraries` project.json section

Last updated: 2026-07-14
Overall Effort: x-large (1d–3d) — the whole plan-46 feature (A + B + C)
Effort: medium (1h–2h)
Depends on: nothing

Adds a new optional top-level `"libraries"` section to `project.json` that maps
each `LINK` **logical library name** (e.g. `sqlite3`) to a list of per-platform
**locators** — where the concrete native shared object actually lives for a
given `os` / `arch` / `libc`. This sub-plan delivers only the **parse +
validate + in-memory accessor** for that section; it does not yet feed any
`.mfp` section (plan-46-B) or the linker (plan-46-C).

The single behavioral outcome: a binding package whose `project.json` carries a
well-formed `libraries` section validates cleanly and exposes a
`project_libraries(manifest)` accessor returning the parsed locators; a
malformed entry emits a precise `PROJECT_JSON_*` diagnostic and fails the build.

References (read first):

- `src/docs/spec/tooling/01_project-manifest.md` — the manifest schema, field
  table, and `2-200-00xx` diagnostic catalogue this section extends.
- `src/manifest/mod.rs` — `validate_project_manifest`, `validate_sources`,
  `validate_kind`/`validate_mode`, `validate_required_string`,
  `validate_optional_string`, `field_position`.
- `src/manifest/package.rs` — `package_dependencies` (the model for a repeated
  sub-object accessor).
- `src/rules/table.rs` — the `2-200-00xx` `PROJECT_JSON_*` rows.

## 1. Goal

- A `project.json` with a `libraries` object of the shape below parses into an
  ordered `Vec<LibraryLocator>` per logical name, reachable via a new
  `project_libraries(&manifest)` accessor, and `validate_project_manifest`
  returns `Ok` for it.
- Every malformed shape (non-object `libraries`, non-array value, non-object
  entry, missing/blank `os`/`source`/`kind`, unknown `os`/`arch`/`libc`/`kind`
  token) emits a specific diagnostic and fails validation.
- The section is **optional**: a manifest with no `libraries` key validates
  exactly as today.

Target JSON shape (the source form; `kind` is explicit, `hash` is NOT in the
manifest — it is computed at build time in plan-46-B):

```json
"libraries": {
  "sqlite3": [
    { "os": "macos", "arch": "aarch64", "kind": "system",   "source": "libsqlite3.dylib" },
    { "os": "linux", "arch": "riscv64", "libc": "musl", "kind": "vendored", "source": "libs/libsqlite3_riscv64.so" }
  ]
}
```

### Non-goals (explicit constraints)

- No `.mfp` format change here (that is plan-46-B, section id 10).
- No cross-check that a `LINK "name"` in code has a matching `libraries` entry
  (needs the IR link functions — plan-46-B), and no per-target coverage warning.
- No linker / `link_thunk` change (plan-46-C).
- Do not read or require a `hash` field in the manifest — vendored hashes are
  computed from the file at build time (plan-46-B). A manifest-supplied `hash`
  is ignored.
- Do not change any existing manifest field's meaning or its diagnostics.

## 2. Current State

`validate_project_manifest` (`src/manifest/mod.rs:24`) loads the file, requires
a top-level object, then runs per-field validators that accumulate a `valid`
bool and emit via `rules::show_diagnostic(CODE, msg, path, line, col_start,
col_end)`. The manifest fields validated today are `name`/`version`/`mfb`
(required strings), `sources` (array of objects), `entry`/`author`/`url`/`icon`
(optional strings), `kind`, `mode`. Everything else (`ident`, `packages`,
`targets`, `config`) is read lazily by later stages, not schema-checked.

The closest precedent for a new array-of-objects section is `validate_sources`
(`src/manifest/mod.rs:190`): absent → error; not-array → `PROJECT_JSON_FIELD_TYPE`;
empty → dedicated code; per-entry object check recursing into
`validate_required_string`. The soft-warn precedent for an unrecognized-but-typed
enum value is `validate_kind`/`validate_mode` (wrong **type** is a hard
`PROJECT_JSON_FIELD_TYPE`; unknown **value** only warns).

JSON access idiom throughout: `manifest.get(field).and_then(|v| v.get::<T>())`
with `T ∈ {String, Vec<JsonValue>, HashMap<String, JsonValue>, f64, bool}`.
Accessors for later-stage data live in `src/manifest/package.rs`
(`package_dependencies` at line 457 is the template: walk the array, build a
`Vec` of typed structs, silently skip malformed-but-non-fatal entries).

Manifest diagnostics occupy `2-200-0001` .. `2-200-0013` in `src/rules/table.rs`
(`2-200-0011` is unused/reserved). The next free codes are `2-200-0011` and
`2-200-0014`+.

## 3. Design Overview

Two pieces:

1. **Data model + accessor** (`src/manifest/libraries.rs`, new): the parsed
   `LibraryLocator`/`LibKind`/`Libc` types and `project_libraries(&manifest) ->
   BTreeMap<String, Vec<LibraryLocator>>` (deterministic key order for later
   encoding). Parses leniently — it assumes validation already ran.
2. **Validator** (`validate_libraries` in `src/manifest/mod.rs`): the strict
   schema walk wired into `validate_project_manifest`, emitting the new
   diagnostics.

Correctness risk is low and concentrated in the enum-token validation (the
`os`/`arch`/`libc`/`kind` allowed-value sets must match the canonical target
axes so plan-46-B/C can resolve against them). Get those value sets from the
target registry, not from memory.

Rejected alternative: deriving `kind` from `source`'s form (bare name → system,
path → vendored) instead of an explicit field. Rejected — the user chose an
explicit `kind` flag so intent is unambiguous in the manifest and the `.mfp`;
we still *validate* the flag against the source form (a `vendored` with a bare
filename, or a `system` with a path, is a warning) to catch mistakes.

## 4. Detailed Design

### 4.1 Data model (`src/manifest/libraries.rs`)

```rust
pub enum Libc { Glibc, Musl }
pub enum LibKind { System, Vendored }

pub struct LibraryLocator {
    pub os: String,            // "macos" | "linux"  (canonical, matches BuildTarget.os)
    pub arch: Option<String>,  // None = any arch; else "aarch64"|"x86_64"|"riscv64"
    pub libc: Option<Libc>,    // linux only; None → glibc default (see plan-46-C match rule)
    pub kind: LibKind,
    pub source: String,        // bare soname (system) or project-relative path (vendored)
}
// project_libraries returns BTreeMap<String /*logical name*/, Vec<LibraryLocator>>
```

Canonical value sets (source them from `src/target.rs` / `src/os/linux/flavor.rs`
rather than hardcoding a second copy where practical):
- `os` ∈ {`macos`, `linux`} — the `BuildTarget.os` values of the four
  registered backends (`src/target.rs:161` `NATIVE_BACKENDS`).
- `arch` ∈ {`aarch64`, `x86_64`, `riscv64`} — the registered backends' arches.
- `libc` ∈ {`glibc`, `musl`} — `LinuxFlavor::{Glibc,Musl}` (`src/os/linux/flavor.rs`).

### 4.2 `validate_libraries` (in `src/manifest/mod.rs`)

Absent `libraries` → return `true` (optional). Present:
- not a JSON object → `PROJECT_JSON_FIELD_TYPE`.
- each value must be a non-empty **array**; else `PROJECT_JSON_FIELD_TYPE` /
  new empty-list code.
- each array element must be an object; else `PROJECT_JSON_FIELD_TYPE`.
- `os`: required non-blank string via the `validate_required_string` idiom;
  value must be in the `os` set — unknown value → new
  `PROJECT_JSON_LIBRARY_INVALID` (hard error, since an unknown token yields a
  dead entry — unlike `kind`/`mode` where an unknown value is still a runnable
  default).
- `arch`: optional string; if present, value in the `arch` set or
  `PROJECT_JSON_LIBRARY_INVALID`.
- `libc`: optional string; if present, value in {`glibc`,`musl`} or
  `PROJECT_JSON_LIBRARY_INVALID`; **on `os:"macos"` a present `libc` is a
  warning** (macOS has no libc axis) and is ignored.
- `source`: required non-blank string.
- `kind`: required string in {`system`,`vendored`}; else
  `PROJECT_JSON_LIBRARY_INVALID`.
- consistency **warning** (non-fatal): `kind:"vendored"` with a bare filename
  (no `/`), or `kind:"system"` with a path (contains `/`).

Diagnostic positions use `field_position(contents, "libraries")` as the anchor
(entry-level precision is a nice-to-have, not required).

New rows in `src/rules/table.rs` (allocate next-free; suggested):
- `2-200-0011` `PROJECT_JSON_LIBRARY_INVALID` (Error) — a `libraries` entry is
  malformed or carries an unknown `os`/`arch`/`libc`/`kind` token.
- `2-200-0014` `PROJECT_JSON_LIBRARY_KIND_MISMATCH` (Warn) — `source` form does
  not match the declared `kind`.

(If `2-200-0011` cannot be reused cleanly, take `2-200-0014`/`0015`; the exact
numbers are an implementation detail as long as they live in `2-200-00xx` and
the spec table is updated.)

## Compatibility / Format Impact

Manifest-only. A new **optional** top-level key. No `.mfp`, ABI, or wire change.
Manifests without `libraries` are unaffected. `project.json` files that happen
to already contain a `libraries` key of a different shape would now be validated
— acceptable, since the key was previously undefined and unread.

## Phases

### Phase 1 — data model + accessor

Parse-only, no diagnostics; independently valuable (later phases/plans consume
the accessor).

- [ ] Add `src/manifest/libraries.rs` with `Libc`, `LibKind`, `LibraryLocator`,
      and `project_libraries(&HashMap<String,JsonValue>) -> BTreeMap<String,
      Vec<LibraryLocator>>`, following `package_dependencies`
      (`src/manifest/package.rs:457`): lenient, skips malformed-but-non-fatal.
- [ ] Register the module in `src/manifest/mod.rs`.
- [ ] Tests: unit tests in `src/manifest/libraries.rs` `#[cfg(test)]` — a
      multi-entry `libraries` object parses to the expected `Vec`s in key order;
      absent key → empty map; `libc` omitted → `None`; `arch` omitted → `None`.

Acceptance: `project_libraries` returns the exact parsed locators for a
representative manifest (verified by unit test), and an empty map when the key
is absent.
Commit: —

### Phase 2 — validation wired into `validate_project_manifest`

- [ ] Add `validate_libraries(manifest, contents, path) -> bool` to
      `src/manifest/mod.rs` per §4.2; call it from `validate_project_manifest`
      into the accumulating `valid` flag.
- [ ] Add the new `PROJECT_JSON_LIBRARY_INVALID` (and
      `PROJECT_JSON_LIBRARY_KIND_MISMATCH` warn) rows to `src/rules/table.rs`.
- [ ] Tests: negative fixtures/cases — non-object `libraries`, non-array value,
      non-object entry, missing `os`, unknown `os`/`arch`/`libc`/`kind` token,
      blank `source`, macOS-with-`libc` (warn), vendored-bare-name (warn). Use
      the existing manifest-validation test harness (mirror how
      `PROJECT_JSON_*` cases are currently tested).
- [ ] Doc: extend `src/docs/spec/tooling/01_project-manifest.md` with the
      `libraries` schema, the value sets, and the new diagnostic rows.

Acceptance: each malformed manifest above emits its specific diagnostic and
`validate_project_manifest` returns `Err`; a well-formed manifest returns `Ok`.
Verified by the negative-case tests and one positive case.
Commit: —

## Validation Plan

- Tests: unit (parse) + validation negative/positive cases as above.
- Runtime proof: `./mfb build` on a `kind:"package"` fixture whose `project.json`
  has a valid `libraries` section completes validation with no manifest error;
  the same fixture with an unknown `os` token fails with
  `PROJECT_JSON_LIBRARY_INVALID`.
- Doc sync: `src/docs/spec/tooling/01_project-manifest.md` (schema + diagnostics)
  and `.ai/specifications.md` obligation (keep spec current).
- Acceptance: repo acceptance suite (`scripts/test-accept.sh`) stays green;
  new diagnostics appear only in the intended fixtures.

## Open Decisions

- Exact new diagnostic code numbers — reuse reserved `2-200-0011` vs append
  `2-200-0014`+. Recommend: use `2-200-0011` for the hard error (fills the
  reserved gap), `2-200-0014` for the warn. (§4.2)
- `arch` omitted semantics — **any-arch wildcard** (recommended) vs required.
  The match rule that consumes this lives in plan-46-C §match; A only records
  `None`. (§4.1)

## Summary

Low-risk, manifest-only. The only real care is making the `os`/`arch`/`libc`
value sets exactly the canonical target axes so plan-46-B's coverage check and
plan-46-C's resolver match against the same vocabulary. Nothing outside the
manifest layer changes.
