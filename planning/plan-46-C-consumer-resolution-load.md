# plan-46-C: consumer-side locator resolution + linker load

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-46-B (which depends on plan-46-A)

Consumes the `NATIVE_LIBRARY_TABLE` (section id 10) at **executable** build
time: for the target being emitted `(os, arch, libc)`, resolve the
most-specific locator per logical library, and replace `link_thunk.rs`'s
hardcoded `library_filename()` soname guess with the resolved `source` string.
For `system` locators this is the full end-to-end path (author-declared exact
soname → `dlopen`). For `vendor` locators, the consumer-placed file is verified
against the section-10 sha256 at build time; making a vendor library actually
*loadable* (output layout, RPATH, copy) is plan-46-D.

The single behavioral outcome: an executable importing the `sqlite3` binding
`dlopen`s the exact `source` the binding author declared for the build's
`(os,arch,libc)` — not a synthesized `libsqlite3.so.0` — and a target with no
matching locator fails the build with a precise diagnostic instead of silently
emitting a wrong soname.

References (read first):

- `src/target/shared/code/link_thunk.rs` — `library_filename` (lines 34-42, the
  guess to delete), `emit_link_support` (lines ~145-156, the sole caller that
  emits the `lib_symbol` cstring), `lower_link_initializer` (`_mfb_linker_init`,
  lines ~202-331, unchanged — it consumes whatever bytes sit in the cstring).
- `src/binary_repr/reader.rs` — where section 10 is decoded (plan-46-B) and how
  merged package data reaches codegen.
- `src/ir/package.rs:119` — how imported `link_functions` merge into the project
  (dedup by `(alias,name)`); the locator table merges alongside.
- `src/os/linux/flavor.rs` (`LinuxFlavor`, `::ALL` at line 8),
  `src/os/linux/link/mod.rs:85` (`write_executable(flavor)` — the per-flavor
  output path that makes libc a build-time axis).
- `src/docs/spec/language/17_native-libraries.md:167-179` (loading model + the
  "platform-specific dependency" note to update).

## 1. Goal

- Building an executable for `(os,arch,libc)` that imports a binding with a
  section-10 table emits, for each logical library, the `source` string of the
  **most-specific matching locator**, and `_mfb_linker_init` `dlopen`s that
  exact string.
- No matching locator for the build's target → hard build error
  `NATIVE_LIBRARY_NO_MATCH` (not a runtime `ErrNativeBindingUnavailable`).
- Two equally-specific matching locators for one target → hard error
  `NATIVE_LIBRARY_AMBIGUOUS`.
- A `vendor` locator: the file at `<consumer project root>/vendor/<source>` is
  read and its sha256 compared to the table's hash; mismatch →
  `NATIVE_LIBRARY_HASH_MISMATCH`, missing → `NATIVE_LIBRARY_FILE_MISSING`.
- `library_filename()` is deleted; no code path synthesizes a soname anymore.

### Non-goals (explicit constraints)

- **Vendor distribution and loadability are plan-46-D**: nothing here changes the
  output layout, emits an RPATH, or copies a vendored `.so` anywhere. This plan
  *resolves and verifies*; D makes the result findable at runtime. See §1.1.
- No change to `_mfb_linker_init`'s control flow, the resource model, or the
  `(alias,name,library,symbol,…)` trailer.
- No new manifest field on the *consumer* — the consumer needs no `libraries`
  section; it reads the imported binding's section 10.

### 1.1 Honest scope: what a vendor build does after C but before D

State this plainly rather than discovering it in testing. After C alone, a
`vendor` locator resolves correctly, its file is hash-verified at build time, and
its bare filename is emitted into the `dlopen` cstring — but **nothing has told
the dynamic loader where to look**, so `dlopen("libfoo.so")` searches only the
system path and the load fails at runtime. C does not make vendor libraries work;
it makes them *correct up to the point of loading*.

Nothing in the tree regresses, because no in-tree binding vendors anything today
(`bindings/sqlite3` is all `system` after plan-46-A Phase 3). But **C and D should
land together, or D should follow C promptly** — shipping C alone would let
someone author a vendor locator and get a binary that builds clean and fails at
runtime, which is exactly the failure mode this whole plan exists to remove. If C
must ship alone for longer than that, prefer a hard build error on any resolved
`vendor` locator over emitting a binary that cannot load.

## 2. Current State

`emit_link_support` (`src/target/shared/code/link_thunk.rs:~151`) collects each
distinct `IrLinkFunction.library` (declaration order, `library_index`) and, per
library, emits `cstring_object(lib_symbol(idx), library_filename(target,
logical))`. `library_filename` (lines 34-42) is the **only** producer of the
`dlopen` filename:

```rust
fn library_filename(target: &str, logical: &str) -> String {
    if target.contains("macos") { format!("lib{logical}.dylib") }
    else { format!("lib{logical}.so.0") }
}
```

This is the string the plan replaces — it "misses many valid libraries"
(unversioned `.so`, `.so.3`, non-`lib`-prefixed, framework paths, per-arch/libc
variants). Note it branches on `target.contains("macos")` and never consults the
manifest; the dead `native` manifest key (removed in plan-46-A Phase 3) never fed
it.

`lower_link_initializer` loads that cstring's address into the dlopen arg and
branches to `fail` on NULL; it needs **no change** — it already consumes
whatever bytes the constant holds.

Crucially, a single Linux build emits **both** libc flavors:
`write_executable(flavor)` (`src/os/linux/link/mod.rs:106-112`) writes
`<name>-glibc.out` and `<name>-musl.out`. `library_filename` ignores libc today
(same soname for both). Once locators can differ by libc, the emitted `source`
cstring may need to **differ per flavor output** (§4.3).

Merged imported `link_functions` arrive via `src/ir/package.rs:119`; the
section-10 table (plan-46-B) is merged alongside, keyed by logical name.

## 3. Design Overview

Three pieces:

1. **Resolver** (new, e.g. `src/target/shared/code/link_locator.rs`): pure
   function `resolve(table, logical, os, arch, libc) -> Result<&Locator,
   ResolveErr>` implementing the most-specific-match rule (§4.1). No I/O.
2. **`emit_link_support` rewrite**: call the resolver instead of
   `library_filename`; emit the resolved `source` cstring; surface
   no-match/ambiguous as build diagnostics; delete `library_filename`.
3. **Vendor verify** (build-time): for a resolved `vendor` locator, read
   `<consumer root>/vendor/<source>`, sha256, compare to the table hash; error on
   missing/mismatch.

Risk is **low and well-bounded**. The one axis worth confirming — Linux
per-libc-flavor emission — was checked against the pipeline and is **already
supported**: codegen runs once per flavor and emits a separate data image per
flavor (§4.3), so a libc-specific `source` lands in the right binary for free.
The remaining work there is pure plumbing (thread the flavor's libc into
`emit_link_support`). What real risk exists is in getting the match rule (§4.1)
and the vendor verify (§4.4) correct, not in codegen structure.

### 3.1 Emission is a **name**, never a path — but it does branch on type

Since `source` is always a **bare filename**, and plan-46-D's RPATH makes the
loader search the vendor directory, emission is always a plain name: no path
construction, no platform string-munging, no directory anywhere in the cstring.

This is what keeps the vendor directory's *location* entirely plan-46-D's
problem. That location is not uniform — `$ORIGIN/vendor` on Linux,
`@loader_path/vendor` for a macOS console build, and
`@executable_path/../Frameworks` for a macOS `.app` bundle, whose dylibs live in
the platform-standard `Contents/Frameworks/` (plan-46-D §4.4) — but none of that
variation reaches this plan. C emits a name; D decides where the loader looks.

**The name is not always `source`, though.** An earlier draft of this plan claimed
`system` and `vendor` emit the identical cstring — `source` verbatim, with no type
branch — and sold that as the payoff of plan-46-A's redesign. **That claim is
retracted.** It was buying a silent wrong-library load:

- a **`system`** locator emits `source` verbatim — the exact soname, which the
  platform's dynamic loader resolves and which knows nothing of our conventions;
- a **`vendor`** locator emits `<declaring-unit>-<source>` — the disambiguated
  name plan-46-D §4.5 writes into the output vendor directory.

The reason is plan-46-D §4.5: vendor `source` filenames are unique only *within
one manifest* (plan-46-A §4.3), the output flattens every vendor file into one
directory, and the emitted filename **is** the library's identity. Two packages
each vendoring a `libfoo.so` would otherwise collide, and both bindings would
`dlopen("libfoo.so")` and get whichever file won. So plan-46-D prefixes on copy,
and this plan must emit the same prefixed name.

The elegance was real, but it cost correctness. The branch is one `if` in
`emit_link_support`; the collision is a wrong library loaded silently. Take the
branch — and **share the name-building helper with plan-46-D's copy step** rather
than constructing the string independently in two places, because the file written
and the string emitted must be byte-identical or the `dlopen` misses.

Rejected alternative: keep one shared cstring and pick glibc/musl at **runtime**
inside `_mfb_linker_init` via libc detection. Rejected — brittle runtime
detection for a fact fully known at build time, and unnecessary: the build
already forks per flavor with its own data image, so resolve at build time.

## 4. Detailed Design

### 4.1 Most-specific match rule

Target = concrete `(os, arch, libc)` (libc = the flavor being emitted; N/A for
macos). Per plan-46-A, `arch` and `libc` are **symmetric optional wildcards** —
`None` means *any*. A locator **matches** when:
- `locator.os == target.os`, and
- `locator.arch` is `None` or `== target.arch`, and
- `locator.libc` is `None` or `== target.libc` (macos: the libc axis is ignored;
  plan-46-A rejects `libc` on macOS at validation, so it cannot be `Some` here).

Among matches, **specificity** = the number of specified axes:
`(arch specified? 1:0) + (libc specified? 1:0)`. Highest wins. A tie →
`NATIVE_LIBRARY_AMBIGUOUS`. No match → `NATIVE_LIBRARY_NO_MATCH`.

Worked example, from plan-46-A's manifest — building `linux/riscv64/musl`:

| locator | matches? | specificity |
| --- | --- | --- |
| `{os: linux, type: system, source: libsqlite3.so.0}` | yes (both wildcards) | 0 |
| `{os: linux, arch: riscv64, libc: musl, source: libsqlite3-riscv64-musl.so}` | yes | 2 |

The vendor entry wins. Building `linux/x86_64/glibc`, only the first matches, so
the system soname is used. This is the "vendor on one slot, system everywhere
else" pattern falling out of the rule with no special case — and note a Linux
`vendor` locator always scores 2, because plan-46-A §3.2 requires both axes on
it, so it always beats a wildcarding `system` entry for its exact slot.

A tie is only reachable via genuinely duplicate entries (same `os`, same
specified axes), which is why `NATIVE_LIBRARY_AMBIGUOUS` is a real but rare
error.

### 4.2 `emit_link_support` rewrite

Replace the `library_filename(platform.target(), library)` call with
`resolve(&table, library, os, arch, libc)?`, then emit
`cstring_object(lib_symbol(idx), dlopen_name(&locator, declaring_unit))` where
(§3.1):

```rust
fn dlopen_name(locator: &Locator, declaring_unit: &str) -> String {
    match locator.lib_type {
        LibType::System => locator.source.clone(),                    // exact soname
        LibType::Vendor => format!("{declaring_unit}-{}", locator.source), // matches D §4.5
    }
}
```

`declaring_unit` is the package name for an imported binding's locator, the
project name for one from the project's own `libraries` section. This helper is
**shared with plan-46-D §4.5's copy step** — the file written and the string
emitted must be identical, so they must not be built independently.

On `ResolveErr`, raise the corresponding build diagnostic and abort (no cstring
emitted). Delete `library_filename`. `lower_link_initializer` is untouched.

For a `vendor` resolved locator, run §4.4 verify before emitting.

### 4.3 Per-flavor emission (already supported — plumbing only)

**Confirmed against the pipeline: codegen already runs once per libc flavor.**
`write_executable` is called per flavor by the caller, which loops
`LinuxFlavor::ALL` and calls `code::lower_module(&module, &native_plan, packages,
flavor)` per flavor, then encodes a **separate data image** per flavor
(`src/target/linux_x86_64/mod.rs:302-324`; linux-aarch64 at
`src/target/linux_aarch64/mod.rs:286-308` and riscv64 are analogous). The
lowering is already flavor-parameterized — the two worlds emit **different
import library names** today (`libc.so.6` vs `libc.musl-x86_64.so.1`) via the
per-flavor `platform_imports` map. The native `LINK` soname is just one more
such per-flavor string, and it lands in the correct flavor's data image for
free — there is **no shared-codegen divergence problem**.

The only gap: `emit_link_support` (`src/target/shared/code/link_thunk.rs:136`)
currently receives just `platform` and reads `platform.target()` (`os-arch`,
**no libc**). So the work is to **thread the flavor's libc into
`emit_link_support`** (from the per-flavor `lower_module` call, through
`lower_module_for_platform` at `src/target/shared/code/mod.rs:1162`), then hand
it to `resolve`. This mirrors how `platform_imports` is already passed in
flavor-specific. macos is single-flavor — pass libc `None`.

When glibc and musl resolve to the same `source` (the common case — one `system`
soname covering both flavors), the two flavor images simply hold identical
cstrings; when they differ (a `vendor` locator with per-libc files), each flavor
image holds its own — automatically, because each flavor is already an
independent codegen pass.

Note this interacts with plan-46-D: both flavor executables live in the **same**
output directory and share **one** `vendor/` subdirectory. That works only
because plan-46-A §4.3 forces vendor `source` filenames to be unique project-wide
— `libsqlite3-x86_64-glibc.so` and `libsqlite3-x86_64-musl.so` coexist in one
directory without collision.

### 4.4 Vendor verify (build-time)

For a resolved `vendor` locator at the **consumer** executable build:
- read `<consumer project root>/vendor/<source>` — the file the consumer author
  placed by hand (the `.mfp` does not carry the blob; distribution is deferred);
- missing → `NATIVE_LIBRARY_FILE_MISSING` (error, naming the full expected path);
- sha256(file) ≠ the section-10 hash → `NATIVE_LIBRARY_HASH_MISMATCH` (error,
  "wrong version");
- match → proceed.

Use `sha2::{Digest, Sha256}` streamed in chunks, per plan-46-B §4.3 (there is no
reusable Rust digest helper; `package_mfp::sha256` is module-private and
`src/*/crypto.rs` is codegen for the MFBASIC builtin, not callable Rust).

New diagnostics in `src/rules/table.rs`. **Next free native code is `2-203-0118`**
— plan-46-B takes `0114`..`0117`, and note `2-203-0099` and everything through
`2-203-0113` is already allocated (an earlier draft's suggestion of
`2-203-0102`..`0105` was wrong; those are live rows):
- `2-203-0118` `NATIVE_LIBRARY_NO_MATCH` (Error)
- `2-203-0119` `NATIVE_LIBRARY_AMBIGUOUS` (Error)
- `2-203-0120` `NATIVE_LIBRARY_FILE_MISSING` (Error)
- `2-203-0121` `NATIVE_LIBRARY_HASH_MISMATCH` (Error)

## Compatibility / Format Impact

- **Runtime/codegen behavior change:** the `dlopen` filename now comes from the
  binding's section-10 locator, not a synthesized soname. A binding that
  previously "happened to work" because `libX.so.0` matched will keep working
  only if its `libraries` section declares that soname (plan-46-B made the
  binding author responsible; that is the intended, non-backward-compatible
  shift — the plan-46 non-goal notes no back-compat is required).
- **No `.mfp` format change here** (section 10 defined in plan-46-B). No ABI or
  wire change. Consumer manifests unchanged.

## Phases

### Phase 1 — resolver + system-case emission

Full end-to-end for `system` locators; the common, lower-risk case.

- [ ] Add `src/target/shared/code/link_locator.rs` with the pure `resolve`
      function and `ResolveErr` per §4.1; unit tests covering wildcard-arch,
      wildcard-libc, **both wildcards (the one-line all-of-Linux system entry)**,
      **a concrete vendor entry outranking a wildcard system entry for its slot**
      (the §4.1 worked example), no-match, ambiguous, macos-libc-ignored.
- [ ] Rewrite `emit_link_support` (`src/target/shared/code/link_thunk.rs`) to
      call `resolve` and emit `dlopen_name(&locator, declaring_unit)` (§4.2 — the
      bare soname for `system`, the `<declaring-unit>-<source>` prefixed name for
      `vendor`); delete `library_filename`; surface
      `NATIVE_LIBRARY_NO_MATCH`/`AMBIGUOUS` as build diagnostics.
- [ ] Put `dlopen_name` where plan-46-D §4.5's copy step can call the **same**
      helper (§3.1); a test must assert the emitted cstring equals the copied
      file's name, since a divergence is a `dlopen` miss at runtime and invisible
      at build time.
- [ ] Thread the flavor's libc from the per-flavor `lower_module` call into
      `emit_link_support` (via `lower_module_for_platform`,
      `src/target/shared/code/mod.rs:1162`) so `resolve` sees the correct libc
      (§4.3); pass `None` for macos.
- [ ] Add `2-203-0118`/`0119` to `src/rules/table.rs`.
- [ ] Tests: golden acceptance — an executable importing a `system`-locator
      binding links and its `_mfb_linker_init` cstring holds the declared
      `source` (assert via the linked binary / a codegen dump); a target with no
      locator fails with `NATIVE_LIBRARY_NO_MATCH`. Run the **artifact/byte-diff
      gate** (`scripts/artifact-gate.sh`, `.ai/compiler.md`) — for bindings
      whose declared soname equals the old guess, codegen should be identical.

Acceptance: an executable `dlopen`s the author-declared `source` for its
`(os,arch,libc)`; unmatched target fails at build with `NATIVE_LIBRARY_NO_MATCH`;
`library_filename` no longer exists. Verified by golden + a runtime link of a
real `system` binding (sqlite3) on at least one target — per `.ai/compiler.md`'s
runtime completion gate, a codegen change is not done until a real binary runs.
Commit: —

### Phase 2 — vendor verify

Build-time hash verification of a locally-placed vendor library. Read §1.1: this
phase does **not** make a vendor library loadable — plan-46-D does.

- [ ] Implement §4.4 verify in the link-support path (read
      `<consumer root>/vendor/<source>`, streamed sha256, compare to table hash).
- [ ] Add `2-203-0120`/`0121` to `src/rules/table.rs`.
- [ ] Tests: golden — a `vendor` binding + a correctly-placed fixture file in
      `vendor/` builds and verifies; a tampered file fails with
      `NATIVE_LIBRARY_HASH_MISMATCH`; an absent file fails with
      `NATIVE_LIBRARY_FILE_MISSING`.
- [ ] Doc: update `src/docs/spec/language/17_native-libraries.md` (line ~179
      platform-dependency note + the loading section: locators replace the
      synthesized soname; vendor verify semantics; loadability deferred to
      plan-46-D), and `src/docs/man/link/package.md` (Loading + diagnostics).

Acceptance: a correctly-placed vendor file builds and verifies; a wrong-hash or
missing file fails with the exact diagnostic. Verified by the three goldens.
Commit: —

## Validation Plan

- Tests: resolver unit tests (all match/tie/no-match branches); golden builds
  for system-resolve, no-match error, vendor verify pass/mismatch/missing.
- Runtime proof: build + run an executable that calls the `sqlite3` binding on a
  real target and confirm it opens the author-declared library (system case).
  The vendor *load* proof belongs to plan-46-D; C's vendor proof is the
  build-time verify only (§1.1).
- Doc sync: `src/docs/spec/language/17_native-libraries.md`,
  `src/docs/man/link/package.md`; `.ai/specifications.md` and `.ai/compiler.md`
  obligations (codegen change → runtime completion gate + byte-diff gate).
- Acceptance: `scripts/test-accept.sh` green; `scripts/artifact-gate.sh` clean
  (execution-free codegen gate) since `link_thunk` is on the codegen path.

## Open Decisions

None outstanding. The consumer-side vendor root is settled as
`<consumer project root>/vendor/<source>`, symmetric with the binding author's
layout (plan-46-A §4.2) and with plan-46-D's copy source. The `libc: None`
semantics §4.1 depends on are settled in plan-46-A: `None` = any libc, symmetric
with `arch`, scoring equally in specificity.

## Summary

Turns the author-declared locator table into real load behavior and kills the
soname guess. The feared Linux per-libc-flavor emission turned out to be a
non-issue — codegen already runs per flavor with its own data image, so it's
just plumbing the libc into `emit_link_support`. Because `source` is a bare
filename and D's RPATH covers `vendor/`, emission is always a plain **name** —
never a path, no platform string-munging. It does branch on `type`, though: a
`system` locator emits the exact soname, a `vendor` locator emits the
`<declaring-unit>-<source>` name plan-46-D §4.5 copies it under, because vendor
filenames are only unique *within* a manifest and the output flattens them into
one directory (§3.1). What's left is a pure resolver (§4.1) + a build-time
sha256 check (§4.4). Vendor *loadability* is plan-46-D and C should not ship far
ahead of it (§1.1).
