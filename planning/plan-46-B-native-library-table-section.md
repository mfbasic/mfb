# plan-46-B: `NATIVE_LIBRARY_TABLE` (.mfp section id 10) + author-side checks

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-46-A

## STATUS: IMPLEMENTED

Both phases landed and verified end-to-end: `bindings/sqlite3` builds with
section 10 present, container flag bit 0 set, and a real sha256 of a vendored
file embedded in the `.mfp`. All four diagnostics fire with actionable messages
(a macOS-only manifest emits exactly 6 uncovered warnings; an unused entry warns
and is genuinely absent from the encoding — asserted against the golden `.mfp`).

Correction to the plan: §4.1 says the wire format uses `stringId` "as `DocTable`
does". **`DocTable` does not** — it writes strings inline via `put_bytes`. The
specified `stringId` format was kept (the `os`/`arch`/soname tokens genuinely
repeat across locators, so interning dedups), but the citation was wrong and the
code notes the deviation.


Lights up the reserved-but-unused `.mfp` section **id 10** as
`NATIVE_LIBRARY_TABLE`: a per-logical-library table of platform locators
(`{os, arch?, libc?, type, source, hash?}`) built from plan-46-A's parsed
`libraries` section, emitted **only** for a binding package that declares a
`LINK` block. This is the binding-author side: assemble the table, compute the
sha256 for `vendor` locators from `<project root>/vendor/<source>`, **error**
when a `LINK "name"` in code has no matching `libraries` entry, **warn** per
supported target the table fails to cover, and encode it into the `.mfp`.

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
  (`container_flags`), `build_package_bytes`, and the private
  `sha256` helper at line 199 (§4.3).
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
- `vendor` locators carry a 32-byte sha256 computed from
  `<project root>/vendor/<source>`; `system` locators carry no hash. A `vendor`
  locator whose file is missing or unreadable is a hard error.
- A package with no `LINK` block emits **no** section 10 and does not set bit 0
  (byte-identical `.mfp` to today for non-binding packages).

### Non-goals (explicit constraints)

- No consumer-side resolution and no `link_thunk`/`library_filename` change
  (plan-46-C); no output-layout, RPATH, or vendor-copy change (plan-46-D).
- Do not touch the existing `(alias, name, library, symbol, …)` IR-payload
  trailer (`src/ir/binary.rs` `encode_link_function`) — it stays the interface
  record and is valid for any physical file. Section 10 is a **separate**,
  additive structure.
- Do not reuse `NATIVE_MANIFEST_INVALID` (`2-205-0002`) for these checks — it
  guards the trailer's shape, has no active raise site, and its code is already
  collided with `DOC_NAME_MISMATCH` (intentional; lookup is by rule *name*, so
  the collision is not itself a defect — but do not add to it). Allocate fresh
  native-library diagnostics.
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

### 2.1 Diagnostic code allocation (corrected)

**Verified against `src/rules/table.rs`** — an earlier draft suggested
`2-203-0099..0101` for the new rules. All three are taken. `2-203-0099` is
`NATIVE_FREE_INVALID` (line 944), which is easy to miss because it sits just
before the `2-200-0011` row rather than inside the contiguous native block; and
the `2-203` range runs on to **`2-203-0113`** (`TYPE_ISOLATED_NOT_VISIBLE`, line
764). **The next free native code is `2-203-0114`.** (Gaps exist below 0113 —
110 rows over a 0001..0113 range — but do not scavenge them; append.)

## 3. Design Overview

Three pieces, layered:

1. **Section codec** (`src/binary_repr/sections.rs` + `mod.rs` const +
   writer/reader wiring): `SECTION_NATIVE_LIBRARY_TABLE = 10`, a
   `NativeLibraryTable` struct, and string-pool-backed encode/decode mirroring
   `DocTable`. Round-trips independently of any manifest.
2. **Assembly + validation** (build path, `src/cli/build.rs` around the existing
   `package_metadata` call at line ~527, plus a builder in
   `src/manifest/libraries.rs`): from plan-46-A's `project_libraries` + the IR's
   distinct `IrLinkFunction.library` names, build the table, compute vendor
   sha256s, run the **missing-entry error** and **coverage warning** checks, and
   hand the table to the encoder.
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
    u32      locatorCount
    repeat locatorCount (stable order):
      stringId os                   // "macos" | "linux"
      stringId arch                 // "" = any-arch, else "aarch64"|"x86_64"|"riscv64"
      u8       libc                 // 0 = unspecified, 1 = glibc, 2 = musl
      u8       type                 // 0 = system, 1 = vendor
      stringId source               // bare filename: "libsqlite3.dylib" | "libsqlite3-riscv64-musl.so"
      // sha256 present iff type == vendor (1):
      [32 bytes] hash               // omitted entirely for system
```

`source` is a bare filename in the wire format exactly as in the manifest — the
`vendor/` prefix is **never** encoded. It is a fixed, known location that both
sides derive; storing it would be redundant data that could disagree with the
rule.

Decode validates: `libc`/`type` in range; `hash` present ⇔ `type==vendor`;
`source` still a bare filename (re-check on decode — the `.mfp` is an untrusted
input on the consumer side, and plan-46-C feeds `source` straight into a C string
and a filesystem path; do **not** rely on the producer having validated it);
otherwise a structural decode error (bounded reads, same style as
`decode_link_function`).

### 4.2 Supported-target coverage matrix

The set the coverage warning checks against — derived at build time from
`NATIVE_BACKENDS` (`src/target.rs:165`, four registered backends) ×
`LinuxFlavor::ALL` (`src/os/linux/flavor.rs:8`), with libc applied to linux only:

| os | arch | libc |
| --- | --- | --- |
| macos | aarch64 | — (N/A) |
| linux | aarch64 | glibc |
| linux | aarch64 | musl |
| linux | x86_64 | glibc |
| linux | x86_64 | musl |
| linux | riscv64 | glibc |
| linux | riscv64 | musl |

= **7 target slots**. Derive this from the registry rather than hardcoding it, so
plan-47's windows backend widens the matrix for free.

Coverage, per plan-46-A's settled semantics: `arch: None` covers **all arches**
of its `os`, and `libc: None` covers **both flavors** — the two axes are
symmetric wildcards. So one `{ "os": "linux", "type": "system", "source":
"libsqlite3.so.0" }` entry covers all six Linux slots, and adding
`{ "os": "linux", "arch": "riscv64", "libc": "musl", "source": "…" }` refines
exactly one of them without opening a hole.

Note a `vendor` locator can never be a wildcard on Linux (plan-46-A §3.2 requires
`arch` + `libc`), so it always covers **exactly one** slot. A binding that vendors
its way across all of Linux therefore needs six locators and six files — which is
the truth, not an inconvenience.

For each of the 7 slots not covered by any locator of a given logical library,
emit one `NATIVE_LIBRARY_TARGET_UNCOVERED` warning naming the logical name + the
uncovered `os/arch/libc`.

### 4.3 Assembly + checks (build path)

At the `kind:"package"` build, after IR is available and `package_metadata` is
read (`src/cli/build.rs:~527`):

1. `libs = project_libraries(&manifest)` (plan-46-A).
2. `linked = ` distinct `IrLinkFunction.library` names in the project IR.
3. **Missing-entry error:** for each name in `linked` with no `libs[name]` →
   `NATIVE_LIBRARY_MISSING` (error, abort). This is the "error if `LINK
   logical_name` not listed in libraries" requirement.
4. **Vendor hash:** for each `vendor` locator, read
   `<project root>/vendor/<source>` and sha256 it; missing or unreadable → hard
   error `NATIVE_LIBRARY_SOURCE_UNREADABLE` (the message must name the full
   expected path, since "put the file in `vendor/`" is the entire fix).
5. **Coverage warning:** per §4.2, emit `NATIVE_LIBRARY_TARGET_UNCOVERED` per
   uncovered slot per linked library.
6. **Unused-entry warning:** for each name in `libs` with no matching `LINK` in
   `linked` → `NATIVE_LIBRARY_UNUSED` (warn). This catches dead config — a
   renamed `LINK`, a removed binding, or a typo in the `libraries` key that would
   otherwise sit in the manifest looking authoritative while doing nothing.
7. Build `NativeLibraryTable` from the names in `linked` only — an unused
   `libraries` entry is warned about but **not** encoded, so the section stays
   minimal and never carries a locator nothing can reach. Thread it into
   `lower_package_project`/`BinaryReprProject` so `encode` pushes section 10
   when non-empty.

**sha256 source:** the repo has **no** reusable Rust digest helper. The `sha2`
crate is a direct dependency (`Cargo.toml:9`) and is used directly in four
modules (`src/target/package_mfp/mod.rs:3`, `src/binary_repr/mod.rs:3`,
`src/os/macos/link/mod.rs:2`, `src/audit/collect/mod.rs:85`) — follow that
pattern. Note `package_mfp::sha256` (line 199) is **module-private**, and
`package_content_hash` validates `MFP_MAGIC` first so it is not a
general-purpose digest. Either promote `package_mfp::sha256`'s visibility or use
`sha2::{Digest, Sha256}` directly; do **not** reach for
`src/target/shared/code/crypto.rs` or `src/builtins/crypto.rs` — those are
codegen for the MFBASIC-language `crypto::sha256` builtin, not callable Rust.
Stream the file (`package_content_hash_file` at line 165 uses 64 KiB chunks) —
a vendored `.so` can be tens of MB.

New diagnostics in `src/rules/table.rs`, appended at the next free native code
per §2.1:
- `2-203-0114` `NATIVE_LIBRARY_MISSING` (Error) — a `LINK "name"` has no
  `libraries` entry.
- `2-203-0115` `NATIVE_LIBRARY_TARGET_UNCOVERED` (Warn) — a supported target has
  no locator.
- `2-203-0116` `NATIVE_LIBRARY_SOURCE_UNREADABLE` (Error) — a `vendor` locator's
  file at `<project root>/vendor/<source>` is missing or cannot be read to hash
  it.
- `2-203-0117` `NATIVE_LIBRARY_UNUSED` (Warn) — a `libraries` entry has no
  matching `LINK` in code.

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

- [x] Add `const SECTION_NATIVE_LIBRARY_TABLE: u16 = 10;` to
      `src/binary_repr/mod.rs`.
- [x] Add `NativeLibraryTable` + `encode`/`decode` to
      `src/binary_repr/sections.rs` per §4.1, mirroring `DocTable`.
- [x] Wire the optional push in `BinaryReprProject::encode`
      (`src/binary_repr/writer.rs`, guarded by non-empty) and the optional
      decode-or-default in `read_binary_repr_package`
      (`src/binary_repr/reader.rs`); add the field to the project/package
      structs.
- [x] Tests: round-trip unit tests in `src/binary_repr/` — empty table absent
      from output; a populated table (system + vendor, wildcard arch, both
      libc values) encodes and decodes to an equal structure; duplicate section
      id rejected; `hash`⇔`vendor` invariant enforced on decode; **a `source`
      carrying a `/` or an interior NUL rejected on decode** (§4.1); out-of-range
      `libc`/`type` rejected.

Acceptance: a hand-built `NativeLibraryTable` round-trips byte-faithfully; a
project with an empty table produces an `.mfp` with no section 10; every
malformed-decode case above returns a structural error rather than panicking.
Commit: —

### Phase 2 — assembly, checks, hash, flag

- [x] Add the table builder to `src/manifest/libraries.rs`: `project_libraries` +
      IR link names → `NativeLibraryTable`, with streamed sha256 over
      `<project root>/vendor/<source>` per §4.3.
- [x] Wire it into the `kind:"package"` build in `src/cli/build.rs` near the
      `package_metadata` call; run the missing-entry error, coverage warnings,
      unused-entry warning, and unreadable-source error (§4.3).
- [x] Set container flag bit 0 in `container_flags`
      (`src/target/package_mfp/mod.rs`) when the table is non-empty.
- [x] Add the four `NATIVE_LIBRARY_*` rows (`2-203-0114`..`0117`) to
      `src/rules/table.rs`.
- [x] Commit the fixture blob: a tiny file at
      `<fixture>/vendor/libfixture.so` (see Open Decisions — bytes need only hash
      stably, not be a valid ELF), so the vendor-hash goldens are hermetic.
- [x] Tests: golden acceptance fixtures under the binding-package test area — a
      `LINK`+`libraries` package builds and its `.mfp` carries section 10
      (assert via a decode check or an `mfb`-side dump if one exists); a package
      with `LINK "x"` and no `libraries["x"]` fails with `NATIVE_LIBRARY_MISSING`
      (golden `build.log`); a package covering only macOS emits 6
      `NATIVE_LIBRARY_TARGET_UNCOVERED` warnings; a `libraries` entry with no
      matching `LINK` emits `NATIVE_LIBRARY_UNUSED` and is **absent from the
      encoded section**; a `vendor` locator with no file in `vendor/` fails with
      `NATIVE_LIBRARY_SOURCE_UNREADABLE`; a `vendor` locator with the committed
      fixture file produces the expected hash.
- [x] Verify the coverage math against a wildcard case: one
      `{ "os": "linux", "type": "system" }` entry must cover all six Linux slots
      and emit **zero** uncovered warnings (§4.2) — the regression that the old
      "libc defaults to glibc" semantics would have caused.
- [x] Doc: update `src/docs/spec/package/10_native-bindings.md` (section 10
      format), `01_container-format.md` (bit 0 now emitted, id 10 now used),
      `14_compact-summary.md` (drop id 10 from the "reserved, not emitted"
      list), and `src/docs/man/link/package.md` diagnostics table.

Acceptance: the fixtures above produce the exact section-10 bytes / error /
warning set; non-binding packages are byte-identical to pre-change `.mfp`.
Commit: —

## Validation Plan

- Tests: codec round-trip units (Phase 1); golden build fixtures for
  present-table / missing-entry-error / uncovered-warning / vendor-hash-pass /
  vendor-file-missing (Phase 2), including a small committed fixture `.so`.
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

None outstanding. Settled:

- **A `libraries` entry with no matching `LINK` warns** —
  `NATIVE_LIBRARY_UNUSED` (`2-203-0117`), and the entry is not encoded (§4.3).
  Dead config that looks authoritative is worth a line of output.
- **The vendor-hash fixture blob is committed**, not generated (§Phase 2). The
  hash check does not care that the file is a *valid* ELF — only that its bytes
  hash stably — so a handful of committed bytes named `libfixture.so` is enough,
  and it keeps the goldens hermetic with no toolchain dependency on the test
  host. Keep it small and mark it clearly as a non-ELF hashing fixture so nobody
  later "fixes" it into a real shared object.

## Summary

Additive `.mfp` section that finally uses reserved id 10 + flag bit 0. Real risk
is the coverage matrix (must equal the true target set, and depends on plan-46-A's
unsettled `libc: None` semantics) and deterministic encode order (byte-diff
gate). `source` stays a bare filename on the wire — `vendor/` is derived, never
stored — and is re-validated on decode because the consumer treats the `.mfp` as
untrusted. The interface trailer and every non-binding package stay untouched.
