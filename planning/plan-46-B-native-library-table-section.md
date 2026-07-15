# plan-46-B: `NATIVE_LIBRARY_TABLE` (.mfp section id 10) + author-side checks

Last updated: 2026-07-14
Effort: medium (1h–2h)
Depends on: plan-46-A

Lights up the reserved-but-unused `.mfp` section **id 10** as
`NATIVE_LIBRARY_TABLE`: a per-logical-library table of platform locators
(`{os, arch?, libc?, source, kind, hash?}`) built from plan-46-A's parsed
`libraries` section, emitted **only** for a binding package that declares a
`LINK` block. This is the binding-author side: assemble the table, compute the
sha256 for `vendored` locators, **error** when a `LINK "name"` in code has no
matching `libraries` entry, **warn** per supported target the table fails to
cover, and encode it into the `.mfp`.

The single behavioral outcome: building a `LINK`-bearing package with a complete
`libraries` section produces a `.mfp` carrying a section-10 table that
round-trips byte-faithfully through the reader; a missing entry aborts the build
with a native-library diagnostic; a missing target coverage prints one warning
per uncovered target.

References (read first):

- `src/binary_repr/mod.rs` — `SECTION_*` id constants (id 10 is free),
  `MFPC_MAJOR_VERSION`, `BinaryReprMetadata`.
- `src/binary_repr/writer.rs` — `BinaryReprProject::encode` (optional-section
  push pattern, lines ~927-960), `lower_package_project`,
  `lower_project_with_external_functions` (metadata interning, lines ~92-115).
- `src/binary_repr/reader.rs` — `read_binary_repr_package` (optional-section
  decode-or-default, lines ~300-421).
- `src/binary_repr/sections.rs` — `DocTable`/`ResourceTable` encode/decode as
  the template for a new string-pool-backed table.
- `src/ir/link.rs` — `IrLinkFunction` (`.library` holds the logical name,
  line 22).
- `src/target/package_mfp/mod.rs` — `write_package` → container flags
  (`container_flags`), `build_package_bytes`.
- `src/docs/spec/package/01_container-format.md` (flag bit 0 + section table),
  `.../10_native-bindings.md`, `.../14_compact-summary.md` (the "id 10 reserved,
  not emitted" note to update).

## 1. Goal

- A `kind:"package"` build with a `LINK "sqlite3"` block and a `libraries`
  entry for `sqlite3` writes an `.mfp` whose section id 10 decodes back to the
  exact locator set; the container's optional flag **bit 0** ("contains native
  LINK metadata") is set.
- A `LINK "foo"` with **no** `libraries["foo"]` entry aborts the build with
  `NATIVE_LIBRARY_MISSING` (error).
- For every supported target `(os,arch,libc)` the assembled table does **not**
  cover, exactly one `NATIVE_LIBRARY_TARGET_UNCOVERED` **warning** is emitted.
- `vendored` locators carry a 32-byte sha256 computed from the file at `source`;
  `system` locators carry no hash. Building a `vendored` locator whose `source`
  file is unreadable is a hard error.
- A package with no `LINK` block emits **no** section 10 and does not set bit 0
  (byte-identical `.mfp` to today for non-binding packages).

### Non-goals (explicit constraints)

- No consumer-side resolution, no `link_thunk`/`library_filename` change, no
  vendored file distribution (all plan-46-C or explicitly deferred).
- Do not touch the existing `(alias, name, library, symbol, …)` IR-payload
  trailer (`src/ir/binary.rs` `encode_link_function`) — it stays the interface
  record and is valid for any physical file. Section 10 is a **separate**,
  additive structure.
- Do not reuse `NATIVE_MANIFEST_INVALID` (`2-205-0002`) for these checks — it
  guards the trailer's shape (and is already code-collided with
  `DOC_NAME_MISMATCH`); allocate fresh native-library diagnostics.
- No backward-compatibility shim: readers are updated in lockstep; an old
  reader seeing section 10 is out of scope (the project controls all readers).

## 2. Current State

`.mfp` sections are `(u16 id, bytes)` laid out by
`BinaryReprProject::encode` (`src/binary_repr/writer.rs:~927`). Always-present
sections are built into a fixed `Vec<Section>`; **optional** ones are `push`ed
only when non-empty — `SECTION_RESOURCE_TABLE` (id 11) `if
!self.resources.entries.is_empty()`, `SECTION_DOC_TABLE` (id 17) `if
!self.docs.is_empty()`. The reader (`read_binary_repr_package`,
`src/binary_repr/reader.rs:~300`) collects sections into a `HashMap<id,&[u8]>`
(rejecting duplicate ids) and decodes optionals with `match sections.get(&ID)
{ Some(s)=>read_x(s)?, None=>Default }`. **Section id 10 has no constant** —
it is free.

The current native `LINK` metadata is an **IR-payload trailer**, not a section:
`encode_project`/`decode_project` in `src/ir/binary.rs` append `link_functions`
+ `link_aliases` after the function list. Each `IrLinkFunction`
(`src/ir/link.rs:16`) stores the logical library name in `.library` (line 22);
there is no per-target locator anywhere today.

Manifest metadata reaches the encoder via `package_metadata(manifest)`
(`src/manifest/package.rs:420`) → `BinaryReprMetadata` → `write_package`
(`src/target.rs:303` → `src/target/package_mfp/mod.rs:46`) →
`build_package_binary_repr_bytes` → `lower_package_project`
(`src/binary_repr/writer.rs:16`). Container flags are computed in
`container_flags` (`src/target/package_mfp/mod.rs`); today only **bit 3**
(pre-release) is ever set — **bit 0** ("native LINK metadata") is defined by the
format but never emitted (`src/docs/spec/package/01_container-format.md:198`,
`206`).

Native diagnostics live around `2-203-0089`..`0098` (native ABI) and
`2-205-0002` (`NATIVE_MANIFEST_INVALID`, no active raise site).

## 3. Design Overview

Three pieces, layered:

1. **Section codec** (`src/binary_repr/sections.rs` + `mod.rs` const +
   writer/reader wiring): `SECTION_NATIVE_LIBRARY_TABLE = 10`, a
   `NativeLibraryTable` struct, and string-pool-backed encode/decode mirroring
   `DocTable`. Round-trips independently of any manifest.
2. **Assembly + validation** (build path, `src/cli/build.rs` around the existing
   `package_metadata` call at line ~527, plus a new
   `src/manifest/libraries.rs` builder): from plan-46-A's `project_libraries` +
   the IR's distinct `IrLinkFunction.library` names, build the table, compute
   vendored sha256s, run the **missing-entry error** and **coverage warning**
   checks, and hand the table to the encoder.
3. **Container flag** (`container_flags`): set optional bit 0 when the table is
   non-empty.

Correctness risk concentrates in (a) the coverage-matrix definition — the exact
set of `(os,arch,libc)` tuples the compiler supports, which must be derived from
the target registry + `LinuxFlavor::ALL`, not hardcoded loosely; and (b)
deterministic encode order (sort by logical name then a stable locator order) so
the `.mfp` is reproducible (the repo has a byte-identical self-diff gate).

Rejected alternative: extend the IR-payload trailer with a per-function locator
instead of a dedicated section. Rejected — locators are per *library*, not per
*function*; the trailer dedups by `(alias,name)` and would duplicate locator
data across every symbol; and the format already reserved section 10 + flag
bit 0 for exactly this.

## 4. Detailed Design

### 4.1 Section 10 wire format (`NativeLibraryTable`)

All strings are `stringId` into the existing string pool (as `DocTable` does):

```
NATIVE_LIBRARY_TABLE (section id 10):
  u32 libraryCount
  repeat libraryCount (sorted by logicalName):
    stringId logicalName            // "sqlite3"
    u32      localeCount
    repeat localeCount (stable order):
      stringId os                   // "macos" | "linux"
      stringId arch                 // "" = any-arch, else "aarch64"|"x86_64"|"riscv64"
      u8       libc                 // 0 = unspecified, 1 = glibc, 2 = musl
      u8       kind                 // 0 = system, 1 = vendored
      stringId source               // "libsqlite3.dylib" | "libs/libsqlite3.so"
      // sha256 present iff kind == vendored (1):
      [32 bytes] hash               // omitted entirely for system
```

Decode validates: `libc`/`kind` in range; `hash` present ⇔ `kind==vendored`;
otherwise a structural decode error (bounded reads, same style as
`decode_link_function`).

### 4.2 Supported-target coverage matrix

The set the coverage warning checks against — derived at build time from
`NATIVE_BACKENDS` (`src/target.rs:161`) × `LinuxFlavor::ALL`
(`src/os/linux/flavor.rs`), with libc applied to linux only:

| os | arch | libc |
| --- | --- | --- |
| macos | aarch64 | — (N/A) |
| linux | aarch64 | glibc |
| linux | aarch64 | musl |
| linux | x86_64 | glibc |
| linux | x86_64 | musl |
| linux | riscv64 | glibc |
| linux | riscv64 | musl |

= **7 target slots**. A locator with `arch:None` covers all arches of its `os`;
a linux locator with `libc:None` covers **glibc only** (the default, per
plan-46-A). For each of the 7 slots not covered by any locator of a given
logical library, emit one `NATIVE_LIBRARY_TARGET_UNCOVERED` warning naming the
logical name + the uncovered `os/arch/libc`.

### 4.3 Assembly + checks (build path)

At the `kind:"package"` build, after IR is available and `package_metadata` is
read (`src/cli/build.rs:~527`):

1. `libs = project_libraries(&manifest)` (plan-46-A).
2. `linked = ` distinct `IrLinkFunction.library` names in the project IR.
3. **Missing-entry error:** for each name in `linked` with no `libs[name]` →
   `NATIVE_LIBRARY_MISSING` (error, abort). This is the "error if `LINK
   logical_name` not listed in libraries" requirement.
4. **Vendored hash:** for each `vendored` locator, read the file at `source`
   (resolved relative to the project root); sha256 it; unreadable → hard error
   `NATIVE_LIBRARY_SOURCE_UNREADABLE`.
5. **Coverage warning:** per §4.2, emit `NATIVE_LIBRARY_TARGET_UNCOVERED` per
   uncovered slot per linked library.
6. Build `NativeLibraryTable` (only for names in `linked` — a `libraries` entry
   with no matching `LINK` is ignored, or optionally a warning) and thread it
   into `lower_package_project`/`BinaryReprProject` so `encode` pushes section
   10 when non-empty.

New diagnostics in `src/rules/table.rs` (allocate next-free in the native
range; suggested `2-203-0099`..`2-203-0101`):
- `NATIVE_LIBRARY_MISSING` (Error) — a `LINK "name"` has no `libraries` entry.
- `NATIVE_LIBRARY_TARGET_UNCOVERED` (Warn) — a supported target has no locator.
- `NATIVE_LIBRARY_SOURCE_UNREADABLE` (Error) — a vendored `source` file cannot
  be read to hash it.

### 4.4 Container flag

In `container_flags` (`src/target/package_mfp/mod.rs`), OR in **bit 0** when the
table is non-empty. Keep it an **optional** flag (bits 4-15 rule does not apply;
bit 0 is a low/optional bit) — a reader that ignores it must not reject the
package; section 10 is the source of truth.

## Compatibility / Format Impact

- **Changes:** `.mfp` gains optional section id 10 and now sets container flag
  bit 0 for binding packages. `MFPC_MAJOR_VERSION` stays `2` (append-only,
  optional section — consistent with how id 11/17 were added). No IR-trailer,
  ABI-index, or metadata-header change.
- **Unchanged:** every non-binding package's `.mfp` is byte-identical to today
  (no section 10, bit 0 clear). The `(alias,name,library,symbol,…)` IR trailer
  is untouched.

## Phases

### Phase 1 — section 10 codec + round-trip

Codec in isolation, hand-built table; no manifest wiring. Safe to land alone.

- [ ] Add `const SECTION_NATIVE_LIBRARY_TABLE: u16 = 10;` to
      `src/binary_repr/mod.rs`.
- [ ] Add `NativeLibraryTable` + `encode`/`decode` to
      `src/binary_repr/sections.rs` per §4.1, mirroring `DocTable`.
- [ ] Wire the optional push in `BinaryReprProject::encode`
      (`src/binary_repr/writer.rs`, guarded by non-empty) and the optional
      decode-or-default in `read_binary_repr_package`
      (`src/binary_repr/reader.rs`); add the field to the project/package
      structs.
- [ ] Tests: round-trip unit tests in `src/binary_repr/` — empty table absent
      from output; a populated table (system + vendored, wildcard arch, both
      libc values) encodes and decodes to an equal structure; duplicate section
      id rejected; `hash`⇔`vendored` invariant enforced on decode.

Acceptance: a hand-built `NativeLibraryTable` round-trips byte-faithfully; a
project with an empty table produces an `.mfp` with no section 10.
Commit: —

### Phase 2 — assembly, checks, hash, flag

- [ ] Add the table builder to `src/manifest/libraries.rs` (or a new
      `src/target/package_mfp` helper): `project_libraries` + IR link names →
      `NativeLibraryTable`, with sha256 over vendored sources
      (reuse the repo's existing sha256, e.g. the crypto core / signing path).
- [ ] Wire it into the `kind:"package"` build in `src/cli/build.rs` near the
      `package_metadata` call; run the missing-entry error, coverage warnings,
      and unreadable-source error (§4.3).
- [ ] Set container flag bit 0 in `container_flags`
      (`src/target/package_mfp/mod.rs`) when the table is non-empty.
- [ ] Add the three `NATIVE_LIBRARY_*` rows to `src/rules/table.rs`.
- [ ] Tests: golden acceptance fixtures under the binding-package test area — a
      `LINK`+`libraries` package builds and its `.mfp` carries section 10
      (assert via a decode check or an `mfb`-side dump if one exists); a package
      with `LINK "x"` and no `libraries["x"]` fails with `NATIVE_LIBRARY_MISSING`
      (golden `build.log`); a package covering only macOS emits 6
      `NATIVE_LIBRARY_TARGET_UNCOVERED` warnings.
- [ ] Doc: update `src/docs/spec/package/10_native-bindings.md` (section 10
      format), `01_container-format.md` (bit 0 now emitted, id 10 now used),
      `14_compact-summary.md` (drop id 10 from the "reserved, not emitted"
      list), and `src/docs/man/link/package.md` diagnostics table.

Acceptance: the three fixtures above produce the exact section-10 bytes / error /
warning set; non-binding packages are byte-identical to pre-change `.mfp`.
Commit: —

## Validation Plan

- Tests: codec round-trip units (Phase 1); golden build fixtures for
  present-table / missing-entry-error / uncovered-warning (Phase 2), including
  a vendored-hash case with a fixture `.so`.
- Runtime proof: `./mfb build` a `LINK`-bearing package fixture → `.mfp` decodes
  with the expected locators and bit 0 set; remove the `libraries["sqlite3"]`
  entry → build fails with `NATIVE_LIBRARY_MISSING`.
- Doc sync: the four spec files + the `link` man page above; `.ai/specifications.md`
  obligation.
- Acceptance: `scripts/test-accept.sh` green; **byte-identical self-diff gate**
  (`scripts/artifact-gate.sh` per `.ai/compiler.md`) confirms non-binding
  packages and codegen are unchanged, since this phase changes package
  encoding — verify determinism of the section bytes.

## Open Decisions

- A `libraries` entry with **no** matching `LINK` in code — silently ignore
  (recommended, keeps the section minimal) vs warn (catches dead config). (§4.3)
- Exact diagnostic code numbers within the native range (`2-203-0099`+ vs a new
  `2-205-000x`). Recommend the `2-203` native block for locality. (§4.3)

## Summary

Additive `.mfp` section that finally uses reserved id 10 + flag bit 0. Real risk
is the coverage matrix (must equal the true target set) and deterministic encode
order (byte-diff gate). The interface trailer and every non-binding package stay
untouched.
