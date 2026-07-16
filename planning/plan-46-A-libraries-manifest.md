# plan-46-A: `libraries` project.json section

Last updated: 2026-07-16
Overall Effort: x-large (1d–3d) — the whole plan-46 feature (A + B + C + D)
Effort: medium (1h–2h)
Depends on: nothing

Adds a new optional top-level `"libraries"` section to `project.json` that maps
each `LINK` **logical library name** (e.g. `sqlite3`) to a list of per-platform
**locators** — which concrete native shared object to load for a given `os` /
`arch` / `libc`, and whether it is a **system** library (found by the dynamic
loader) or a **vendor** library (a file the author ships in
`<project root>/vendor/`). This sub-plan delivers only the **parse + validate +
in-memory accessor** for that section; it does not yet feed any `.mfp` section
(plan-46-B), the linker (plan-46-C), or the vendor output bundle (plan-46-D).

It also **replaces** the existing top-level `native` key, which is dead config
(§2).

The single behavioral outcome: a binding package whose `project.json` carries a
well-formed `libraries` section validates cleanly and exposes a
`project_libraries(manifest)` accessor returning the parsed locators; a
malformed entry emits a precise `PROJECT_JSON_*` diagnostic and fails the build.

References (read first):

- `src/docs/spec/tooling/01_project-manifest.md` — the manifest schema, field
  table (lines 29-42), and `2-200-####` diagnostic catalogue (lines 180-190)
  this section extends.
- `src/manifest/mod.rs` — `validate_project_manifest`, `validate_sources`,
  `validate_kind`/`validate_mode`, `validate_required_string`,
  `validate_optional_string`, `field_position`.
- `src/manifest/package.rs` — `package_dependencies` (the model for a repeated
  sub-object accessor).
- `src/rules/table.rs` — the `2-200-####` `PROJECT_JSON_*` rows (lines 5-59 and
  950-966).

## 1. Goal

- A `project.json` with a `libraries` object of the shape below parses into an
  ordered `Vec<LibraryLocator>` per logical name, reachable via a new
  `project_libraries(&manifest)` accessor, and `validate_project_manifest`
  returns `Ok` for it.
- Every malformed shape (non-object `libraries`, non-array value, non-object
  entry, missing/blank `os`/`source`, unknown `os`/`arch`/`libc`/`type` token,
  a `source` that is not a bare filename, a `libc` on macOS, a Linux `vendor`
  entry missing `arch` or `libc`, two `vendor` locators sharing a `source`)
  emits a specific diagnostic — with a message naming the actual cause — and
  fails validation.
- The section is **optional**: a manifest with no `libraries` key validates
  exactly as today.
- The dead `native` key is gone from the one manifest that carries it.

Target JSON shape (`hash` is NOT in the manifest — it is computed at build time
in plan-46-B):

```json
"libraries": {
  "sqlite3": [
    { "os": "macos", "type": "system", "source": "libsqlite3.dylib" },
    { "os": "linux", "type": "system", "source": "libsqlite3.so.0" },
    { "os": "linux", "arch": "riscv64", "libc": "musl", "source": "libsqlite3-riscv64-musl.so" }
  ]
}
```

Read as: use the system `libsqlite3.dylib` on macOS; use the system
`libsqlite3.so.0` everywhere on Linux — any arch, either libc — **except** on
riscv64/musl, where the third entry wins on specificity (§3.2) and loads a
vendored file that must exist at
`<project root>/vendor/libsqlite3-riscv64-musl.so`. The third entry omits `type`,
so it defaults to `vendor`, and as a Linux vendor entry it is required to name
both `arch` and `libc` (§3.2).

### Non-goals (explicit constraints)

- No `.mfp` format change here (that is plan-46-B, section id 10).
- No cross-check that a `LINK "name"` in code has a matching `libraries` entry
  (needs the IR link functions — plan-46-B), and no per-target coverage warning.
- No linker / `link_thunk` change (plan-46-C); no output-layout, RPATH, or
  vendor-copy change (plan-46-D).
- **No filesystem access.** This plan validates the manifest's *shape* only. It
  does NOT check that `<project root>/vendor/<source>` exists — that error
  belongs to plan-46-B, which is the phase that reads the file to hash it
  (`NATIVE_LIBRARY_SOURCE_UNREADABLE`). Keeping A pure keeps it unit-testable
  without fixtures.
- Do not read or require a `hash` field in the manifest — vendor hashes are
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

### 2.1 The `native` key is dead config

`bindings/sqlite3/project.json` carries:

```json
"native": [ { "library": "sqlite3", "platforms": ["linux", "macos", "windows"] } ]
```

**Verified: nothing reads it.** There is no `manifest.get("native")` anywhere in
`src/**` (`grep -rni native src/manifest/` returns zero hits across all three
files); it is not in the spec's top-level field table
(`src/docs/spec/tooling/01_project-manifest.md:29-42`); and it has no validator.
It survives only because `validate_project_manifest` has **no unknown-key
rejection** — it validates a fixed allowlist and silently ignores everything
else.

`platforms` in particular has no consumer at all: the logical library name comes
from the `LINK "sqlite3"` statement in source (`IrLinkFunction.library`), and the
filename is derived algorithmically at codegen time by `library_filename`
(`src/target/shared/code/link_thunk.rs:34`), which branches on
`target.contains("macos")` and never consults the manifest. Listing or omitting
`windows` changes nothing today.

`libraries` replaces it outright, so the migration is pure deletion — no code to
remove, no compatibility shim. **The key appears in 12 tracked `project.json`
files**, not one: `bindings/sqlite3/project.json` plus 11 fixtures under
`tests/syntax/native/**` and `tests/syntax/resources/**`, all with the same inert
shape. (`NATIVE_MANIFEST_INVALID` / `2-205-0002` has no active raise site and is
unrelated to this key — see plan-46-B.)

### 2.2 Diagnostic code allocation (corrected)

**Verified against `src/rules/table.rs`** — an earlier draft of this plan claimed
`2-200-0011` was "unused/reserved" and proposed taking it. That is **wrong**:

| code | name | severity |
| --- | --- | --- |
| `2-200-0011` | `PROJECT_ENTRY_INVALID` | error (raised by `src/manifest/entry.rs`) |
| `2-200-0012` | `PROJECT_JSON_UNKNOWN_MODE` | warn |
| `2-200-0013` | `PROJECT_JSON_ICON_MISSING` | error |

`2-200-0100`/`0101` are `BUILD_FAILED`/`FMT_CHECK_FAILED` (a separate block).
**The next free manifest codes are `2-200-0014` and `2-200-0015`.**

## 3. Design Overview

Two pieces:

1. **Data model + accessor** (`src/manifest/libraries.rs`, new): the parsed
   `LibraryLocator`/`LibType`/`Libc` types and `project_libraries(&manifest) ->
   BTreeMap<String, Vec<LibraryLocator>>` (deterministic key order for later
   encoding). Parses leniently — it assumes validation already ran.
2. **Validator** (`validate_libraries` in `src/manifest/mod.rs`): the strict
   schema walk wired into `validate_project_manifest`, emitting the new
   diagnostics.

Correctness risk is low and concentrated in the enum-token validation (the
`os`/`arch`/`libc` allowed-value sets must match the canonical target axes so
plan-46-B/C can resolve against them). Get those value sets from the target
registry, not from memory.

### 3.1 Why `type` is explicit, and why it defaults to `vendor`

An earlier draft derived system-vs-vendor from the *form* of `source` (bare name
→ system, path → vendored). That detection is **removed**: `type` is now an
explicit field, and because `source` is always a bare filename (§4.2), its form
carries no kind information at all. The consistency warning that draft proposed
(`PROJECT_JSON_LIBRARY_KIND_MISMATCH`) is therefore dead and is **not**
allocated.

The default is `vendor` rather than `system` because it **fails closed**. A
missing or typo'd `type` under a `vendor` default resolves to
`<root>/vendor/<source>` and hard-errors at build time when that file is absent
(plan-46-B). Under a `system` default, the same mistake would silently hand
`source` to the dynamic loader, which would search the system library path and
load whatever it found under that name — a wrong-library load that only shows up
at runtime, and a supply-chain footgun. A build error beats a silent
mis-resolution.

### 3.2 `system` may wildcard; `vendor` must name its exact target

The two types differ in a way that should be enforced, not just documented:

- **`system` means "ask the loader for this name."** The platform supplies the
  build that fits it, so `arch` and `libc` are legitimately omittable —
  `{ "os": "linux", "type": "system", "source": "libsqlite3.so.0" }` is a correct
  and complete statement covering all six Linux slots.
- **`vendor` means "load this exact file I shipped."** That file is one concrete
  artifact compiled for exactly one `(os, arch, libc)` triple. A `.so` built
  against glibc will not load on musl (it wants `libc.so.6`); an x86_64 `.so`
  will not load on aarch64. **There is no such thing as a fat ELF.** So a Linux
  `vendor` locator that omits `arch` or `libc` is not expressing a wildcard — it
  is making a claim that cannot be true.

Therefore: **a `vendor` locator on `os: "linux"` must specify both `arch` and
`libc`**; omitting either is `PROJECT_JSON_LIBRARY_INVALID`. This is not a new
constraint so much as making explicit what §4.3's uniqueness rule already
implies — every vendor locator names a distinct physical file, and a distinct
file *is* a distinct `(arch, libc)` build. An author covering all of Linux needs
six files and six locators either way; this just stops them from shipping one
file and claiming it covers six slots.

**macOS is exempt from the `arch` half**, deliberately: Mach-O *does* have fat
binaries, so a universal `.dylib` covering arm64 and x86_64 with `arch` omitted
is a legitimate, correct locator. (It is also moot today — `NATIVE_BACKENDS`
registers only `macos_aarch64` — but the rule should be right for the reason,
not by accident.) macOS has no libc axis at all (§4.4).

A consequence worth noting for plan-46-C's resolver: since a `vendor` locator
always carries concrete axes, it always outranks a wildcarding `system` locator
on specificity. That is the desired behavior — "use my vendored build on musl,
the system library everywhere else" is expressed by simply adding the one vendor
entry alongside a wildcard system entry, and it works with no extra rules.

## 4. Detailed Design

### 4.1 Data model (`src/manifest/libraries.rs`)

```rust
pub enum Libc { Glibc, Musl }

#[derive(Default)]
pub enum LibType {
    System,
    #[default]
    Vendor,       // §3.1: absent `type` fails closed
}

pub struct LibraryLocator {
    pub os: String,           // "macos" | "linux"  (canonical, matches BuildTarget.os)
    pub arch: Option<String>, // None = ANY arch; else "aarch64"|"x86_64"|"riscv64"
    pub libc: Option<Libc>,   // None = ANY libc (linux only; N/A on macos)
    pub lib_type: LibType,    // JSON key is `type` (a Rust keyword; do not name the field `type`)
    pub source: String,       // bare filename, never a path (§4.2)
}
// project_libraries returns BTreeMap<String /*logical name*/, Vec<LibraryLocator>>
```

Canonical value sets (source them from `src/target.rs` / `src/os/linux/flavor.rs`
rather than hardcoding a second copy where practical):
- `os` ∈ {`macos`, `linux`} — the `BuildTarget.os` values of the registered
  backends (`src/target.rs` `NATIVE_BACKENDS`). Windows joins this set with
  plan-47; the value set must be read from the registry so it widens for free.
- `arch` ∈ {`aarch64`, `x86_64`, `riscv64`} — the registered backends' arches.
- `libc` ∈ {`glibc`, `musl`} — `LinuxFlavor::{Glibc,Musl}` (`src/os/linux/flavor.rs`).

### 4.2 `source` is a bare filename

`source` names a file, never a location. Validation rejects, with
`PROJECT_JSON_LIBRARY_INVALID`, any `source` that:

- is missing, or blank after trim;
- contains `/` or `\` (a path separator of either flavor);
- is exactly `.` or `..`;
- contains a **NUL byte** — `source` is emitted verbatim as a C string into the
  binary by plan-46-C, so an interior NUL would silently truncate the `dlopen`
  argument;
- carries a Windows drive prefix (`C:`) — rejected now so plan-47 does not
  inherit a hole.

For a `system` locator, `source` is the exact soname the dynamic loader is asked
for (`libsqlite3.so.0`, `libsqlite3.dylib`). For a `vendor` locator, the file
must live at **`<project root>/vendor/<source>`** — flat, no subdirectories. The
resolved path is never spelled in the manifest; it is always
`vendor/` + `source`.

### 4.3 Vendor `source` filenames are unique project-wide

Because `vendor/` is flat, one filename means one file. Two `vendor` locators
sharing a `source` — anywhere in the section, across *all* logical names —
are therefore either a redundant duplicate or, far more likely, the real bug:
an author who copied an entry for a new platform and forgot to rename the blob,
leaving (say) the riscv64 entry pointing at the x86_64 file. That is exactly the
mistake this check exists to catch, so it is an **error**
(`PROJECT_JSON_LIBRARY_SOURCE_CONFLICT`), not a warning.

**Scope this check to `vendor` locators only.** `system` sonames legitimately
repeat all the time — `linux/x86_64` and `linux/aarch64` both asking the loader
for `libsqlite3.so.0` is the *normal* case, not an error. A blanket
uniqueness check over every locator would false-positive on the most common
manifest anyone will write.

### 4.4 `validate_libraries` (in `src/manifest/mod.rs`)

Absent `libraries` → return `true` (optional). Present:
- not a JSON object → `PROJECT_JSON_FIELD_TYPE`.
- each value must be a non-empty **array**; else `PROJECT_JSON_FIELD_TYPE` /
  `PROJECT_JSON_EMPTY_FIELD`.
- each array element must be an object; else `PROJECT_JSON_FIELD_TYPE`.
- `os`: required non-blank string via the `validate_required_string` idiom;
  value must be in the `os` set — unknown value → `PROJECT_JSON_LIBRARY_INVALID`
  (hard error, since an unknown token yields a dead entry — unlike `kind`/`mode`
  where an unknown value still leaves a runnable default).
- `arch`: optional string, **`None` = any arch**; if present, value in the `arch`
  set or `PROJECT_JSON_LIBRARY_INVALID`.
- `libc`: optional string, **`None` = any libc** (symmetric with `arch`); if
  present, value in {`glibc`,`musl`} or `PROJECT_JSON_LIBRARY_INVALID`. On
  `os:"macos"` a present `libc` is `PROJECT_JSON_LIBRARY_INVALID` — macOS has no
  libc axis, so the field is meaningless there and rejecting it is consistent
  with every other unknown-token case.
- `type`: optional string, default `vendor`; must be `system` or `vendor`; else
  `PROJECT_JSON_LIBRARY_INVALID` (hard error — an unknown token would otherwise
  silently take the `vendor` default and produce a confusing missing-file error
  two plans downstream).
- **`type:"vendor"` + `os:"linux"` requires both `arch` and `libc`** (§3.2);
  either omitted → `PROJECT_JSON_LIBRARY_INVALID`. macOS `vendor` may omit `arch`
  (fat binaries).
- `source`: required, and a bare filename per §4.2.
- vendor-source uniqueness per §4.3 → `PROJECT_JSON_LIBRARY_SOURCE_CONFLICT`.

**Diagnostic messages must name the specific cause.** `PROJECT_JSON_LIBRARY_INVALID`
covers a dozen distinct mistakes, so its message — not just its code — is what
makes it actionable. Say *"`libc` is not valid on `os: \"macos\"` — macOS has no
libc axis; remove it"*, not "libraries entry is invalid". Same for each other
cause: unknown token (name the field and the accepted set), non-bare `source`
(name the offending character), missing `arch`/`libc` on a Linux `vendor` entry
(say why — the file is one concrete build).

Diagnostic positions use `field_position(contents, "libraries")` as the anchor
(entry-level precision is a nice-to-have, not required).

New rows in `src/rules/table.rs` (next free per §2.2):
- `2-200-0014` `PROJECT_JSON_LIBRARY_INVALID` (Error) — a `libraries` entry is
  malformed, carries an unknown `os`/`arch`/`libc`/`type` token, or a `source`
  that is not a bare filename.
- `2-200-0015` `PROJECT_JSON_LIBRARY_SOURCE_CONFLICT` (Error) — two `vendor`
  locators declare the same `source` filename.

## Compatibility / Format Impact

Manifest-only. A new **optional** top-level key, and the removal of the unread
`native` key. No `.mfp`, ABI, or wire change. Manifests without `libraries` are
unaffected. `project.json` files that happen to already contain a `libraries`
key of a different shape would now be validated — acceptable, since the key was
previously undefined and unread.

## Phases

### Phase 1 — data model + accessor

Parse-only, no diagnostics; independently valuable (later phases/plans consume
the accessor).

- [ ] Add `src/manifest/libraries.rs` with `Libc`, `LibType` (defaulting to
      `Vendor`), `LibraryLocator`, and
      `project_libraries(&HashMap<String,JsonValue>) -> BTreeMap<String,
      Vec<LibraryLocator>>`, following `package_dependencies`
      (`src/manifest/package.rs:457`): lenient, skips malformed-but-non-fatal.
- [ ] Register the module in `src/manifest/mod.rs`.
- [ ] Tests: unit tests in `src/manifest/libraries.rs` `#[cfg(test)]` — a
      multi-entry `libraries` object parses to the expected `Vec`s in key order;
      absent key → empty map; `libc` omitted → `None`; `arch` omitted → `None`;
      **`type` omitted → `LibType::Vendor`**.

Acceptance: `project_libraries` returns the exact parsed locators for a
representative manifest (verified by unit test), and an empty map when the key
is absent.
Commit: —

### Phase 2 — validation wired into `validate_project_manifest`

- [ ] Add `validate_libraries(manifest, contents, path) -> bool` to
      `src/manifest/mod.rs` per §4.4; call it from `validate_project_manifest`
      into the accumulating `valid` flag.
- [ ] Add the `2-200-0014` `PROJECT_JSON_LIBRARY_INVALID` and `2-200-0015`
      `PROJECT_JSON_LIBRARY_SOURCE_CONFLICT` rows to `src/rules/table.rs`.
- [ ] Tests: negative fixtures/cases — non-object `libraries`, non-array value,
      non-object entry, missing `os`, unknown `os`/`arch`/`libc`/`type` token,
      blank `source`, **`source` with a `/`**, **`source` of `..`**, **`source`
      with an interior NUL**, **two vendor locators sharing a `source`**,
      **macOS entry carrying `libc`**, **Linux `vendor` entry missing `arch`**,
      **Linux `vendor` entry missing `libc`**. Positive cases that **must pass**:
      two *system* locators sharing a soname (§4.3), a Linux `system` entry with
      neither `arch` nor `libc` (§3.2), a macOS `vendor` entry with no `arch`
      (fat binary, §3.2). Assert on the diagnostic **message**, not just the
      code — one code covers a dozen causes (§4.4). Use the existing
      manifest-validation test harness (mirror how `PROJECT_JSON_*` cases are
      currently tested).
- [ ] Doc: extend `src/docs/spec/tooling/01_project-manifest.md` — add
      `libraries` to the top-level field table (lines 29-42), add a *Library
      Locator Entries* section documenting `os`/`arch`/`libc`/`type`/`source`,
      the `vendor` default and its fail-closed rationale, the bare-filename rule,
      the flat `vendor/` layout, and the two new diagnostic rows (lines 180-190).

Acceptance: each malformed manifest above emits its specific diagnostic and
`validate_project_manifest` returns `Err`; a well-formed manifest — including
one with repeated *system* sonames — returns `Ok`. Verified by the negative-case
tests and the positive cases.
Commit: —

### Phase 3 — retire `native`, migrate the sqlite3 binding

Small but load-bearing: it makes the tree's one real binding the fixture for
plan-46-B. Pure manifest edits — no `src/**` change, since nothing reads `native`
(§2.1).

- [ ] Delete the `native` key from all **12** tracked `project.json` files:
      `bindings/sqlite3/project.json` plus the 11 fixtures under
      `tests/syntax/native/**` and `tests/syntax/resources/**`. Enumerate them
      first with `grep -rln '"native"' --include=project.json .` — do not work
      from this list from memory.
- [ ] Add the equivalent `libraries` section to `bindings/sqlite3/project.json`,
      declaring the real sonames per platform (`libsqlite3.dylib` on macos; the
      actual `libsqlite3.so.N` on linux — **check the real soname on a target
      box, do not guess**; `library_filename`'s current `libsqlite3.so.0` guess
      is not evidence). All entries are `type: "system"`; the binding vendors
      nothing.
- [ ] Decide per fixture whether the 11 `tests/syntax/**` fixtures need a
      `libraries` section at all. They are syntax fixtures that never link, so
      most likely just drop `native` and add nothing — but any fixture that
      plan-46-B would later fail with `NATIVE_LIBRARY_MISSING` (i.e. one whose
      source has a `LINK` block **and** gets built) needs a real section. Check
      each rather than assuming.
- [ ] Re-sync any goldens the fixture edits churn (`scripts/sync-goldens.sh`).

Acceptance: `grep -rn '"native"' --include=project.json .` returns nothing;
`./mfb build` on `bindings/sqlite3` validates clean with the new section; the
declared sonames are the ones actually present on each target box (verified by
`ls` / `ldconfig -p` on the box, not from memory); acceptance suite green.
Commit: —

## Validation Plan

- Tests: unit (parse) + validation negative/positive cases as above.
- Runtime proof: `./mfb build` on `bindings/sqlite3` completes validation with
  no manifest error; the same manifest with an unknown `os` token fails with
  `PROJECT_JSON_LIBRARY_INVALID`; with two vendor locators sharing a `source`,
  fails with `PROJECT_JSON_LIBRARY_SOURCE_CONFLICT`.
- Doc sync: `src/docs/spec/tooling/01_project-manifest.md` (field table, locator
  schema, diagnostics) and the `.ai/specifications.md` obligation.
- Acceptance: repo acceptance suite (`scripts/test-accept.sh`) stays green;
  new diagnostics appear only in the intended fixtures.

## Open Decisions

None outstanding. Settled:

- **`libc: None` = any libc**, symmetric with `arch: None` (§4.1/§4.4). Earlier
  drafts fixed it to a glibc default, which made `libc` an asymmetric axis and
  meant a system soname identical on both flavors needed two entries to cover
  Linux — with the one-entry form silently leaving musl uncovered. Now
  `{ "os": "linux", "type": "system", "source": "libsqlite3.so.0" }` covers all
  six Linux slots in one line. **plan-46-B §4.2's coverage math and plan-46-C
  §4.1's specificity rule follow from this** (libc scores like arch).
- **A Linux `vendor` locator must specify both `arch` and `libc`** (§3.2) — a
  vendored file is one concrete build and there is no fat ELF, so a wildcard
  there is a claim that cannot be true. macOS `vendor` may omit `arch` (fat
  Mach-O binaries are real).
- **macOS + `libc` → `PROJECT_JSON_LIBRARY_INVALID`** (error, not a warn), with
  the message naming the cause (§4.4). No new code allocated.

## Summary

Low-risk, manifest-only. `type` is explicit (`system`/`vendor`, defaulting to
`vendor` so mistakes fail closed), `source` is always a bare filename, and a
`vendor` source resolves to `<root>/vendor/<source>` — flat, unique project-wide
among vendor entries. `arch` and `libc` are symmetric optional wildcards, but
only `system` may use them: a `vendor` locator names one concrete file, so on
Linux it must name its exact `(arch, libc)` (§3.2).

The only real care is making the `os`/`arch`/`libc` value sets exactly the
canonical target axes so plan-46-B's coverage check and plan-46-C's resolver
match against the same vocabulary — and writing diagnostic *messages* that name
the actual cause, since one code carries a dozen of them. Nothing outside the
manifest layer changes.
